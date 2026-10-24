//! Waveshare 0.85-inch ScreenKey / ST7735S, shared SPI1 and 74HC154 CS.
//! Panel register settings follow Waveshare's LCD_Driver.cpp; see THIRD_PARTY.md.
use embassy_stm32::{
    Peri,
    gpio::{Level, Output, OutputType},
    mode::Async,
    peripherals,
    spi::{Error, Spi, mode::Master},
    time::Hertz,
    timer::{
        low_level::CountingMode,
        simple_pwm::{PwmPin, SimplePwm},
    },
};
use embassy_time::{Duration, Instant, Timer};

mod wave;

const SCREEN_COUNT: usize = 4;
const WIDTH: usize = 128;
const HEIGHT: usize = 128;
const X_OFFSET: u8 = 2;
const Y_OFFSET: u8 = 3;

pub struct Screens {
    pub spi: Spi<'static, Async, Master>,
    pub enable: Output<'static>,
    pub address: [Output<'static>; 4],
    pub dc: Output<'static>,
    pub reset: Output<'static>,
    // Keep flash deselected for the lifetime of the shared bus owner.
    pub flash_cs: Output<'static>,
}

impl Screens {
    fn transfer(&mut self, screen: usize, data: bool, bytes: &[u8]) -> Result<(), Error> {
        assert!(screen < 12);
        self.enable.set_high();
        self.flash_cs.set_high();
        for (bit, pin) in self.address.iter_mut().enumerate() {
            pin.set_level(if screen & (1 << bit) == 0 {
                Level::Low
            } else {
                Level::High
            });
        }
        self.dc
            .set_level(if data { Level::High } else { Level::Low });
        // Address and D/C are stable before the active-low decoder enable.
        cortex_m::asm::delay(16);
        self.enable.set_low();
        let result = self.spi.blocking_write(bytes);
        self.enable.set_high();
        result
    }

    fn command(&mut self, screen: usize, command: u8, data: &[u8]) -> Result<(), Error> {
        self.transfer(screen, false, &[command])?;
        if !data.is_empty() {
            self.transfer(screen, true, data)?;
        }
        Ok(())
    }

    async fn init(&mut self) -> Result<(), Error> {
        self.enable.set_high();
        self.reset.set_low();
        Timer::after_millis(20).await;
        self.reset.set_high();
        Timer::after_millis(120).await;
        for screen in 0..SCREEN_COUNT {
            self.command(screen, 0x11, &[])?; // Sleep out.
        }
        Timer::after_millis(120).await;
        let mut baseline_us = 0;
        for screen in 0..SCREEN_COUNT {
            for &(cmd, data) in INIT {
                self.command(screen, cmd, data)?;
            }
            // Clear/draw RAM before display-on and before enabling the backlight.
            let start = Instant::now();
            self.draw(screen).await?;
            baseline_us += start.elapsed().as_micros();
            self.command(screen, 0x29, &[])?;
            defmt::info!("ScreenKey {} initialized and drawn", screen + 1);
        }
        defmt::info!(
            "Display baseline: four full screens, blocking SPI 4 MHz + row rendering/yields = {} us",
            baseline_us
        );
        Ok(())
    }

    async fn draw(&mut self, screen: usize) -> Result<(), Error> {
        let mut row = [0u8; WIDTH * 2];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let color = pixel(x, y, screen);
                row[x * 2..x * 2 + 2].copy_from_slice(&color.to_be_bytes());
            }
            self.command(screen, 0x2a, &[0, X_OFFSET, 0, X_OFFSET + 127])?;
            let line = Y_OFFSET + y as u8;
            self.command(screen, 0x2b, &[0, line, 0, line])?;
            self.command(screen, 0x2c, &[])?;
            self.transfer(screen, true, &row)?;
            // Bound blocking bus work to one row, allowing key scanning to run.
            Timer::after_micros(1).await;
        }
        Ok(())
    }
    async fn rectangle(
        &mut self,
        screen: usize,
        x: u8,
        y: u8,
        w: u8,
        h: u8,
        pixels: &[u8],
    ) -> Result<(), Error> {
        assert_eq!(pixels.len(), w as usize * h as usize * 2);
        assert!(
            w > 0 && h > 0 && x as usize + w as usize <= WIDTH && y as usize + h as usize <= HEIGHT
        );
        self.command(screen, 0x2a, &[0, X_OFFSET + x, 0, X_OFFSET + x + w - 1])?;
        self.command(screen, 0x2b, &[0, Y_OFFSET + y, 0, Y_OFFSET + y + h - 1])?;
        self.command(screen, 0x2c, &[])?;
        // Command left this screen's address selected, with decoder disabled.
        self.dc.set_high();
        cortex_m::asm::delay(16);
        self.enable.set_low();
        let result = self.spi.write(pixels).await;
        self.enable.set_high();
        result
    }

    async fn animate(
        &mut self,
        backlight: &mut SimplePwm<'static, peripherals::TIM2>,
    ) -> Result<(), Error> {
        let mut config = embassy_stm32::spi::Config::default();
        config.frequency = Hertz(8_000_000);
        self.spi.set_config(&config).expect("valid SPI clock");
        let mut previous = [[0; wave::LAYERS]; 512];
        let mut next = previous;
        let mut buffer = [0u8; wave::BUFFER_BYTES];
        wave::heights(Instant::now().as_millis() as u32, &mut previous);
        for x in (0..512).step_by(wave::TILE_WIDTH) {
            let (y, height, len) = wave::render_tile(
                x,
                None,
                &previous[x..x + wave::TILE_WIDTH],
                &previous[x..x + wave::TILE_WIDTH],
                &mut buffer,
            );
            self.rectangle(
                x / WIDTH,
                (x % WIDTH) as u8,
                y,
                wave::TILE_WIDTH as u8,
                height,
                &buffer[..len],
            )
            .await?;
        }
        let mut frames = 0u64;
        let mut total_bytes = 0u64;
        let mut busy_us = 0u64;
        let mut epoch = Instant::now();
        defmt::info!(
            "Ocean started: SPI1 8 MHz DMA, 23 mm pitch / 15.2064 mm LCD windows, three water layers"
        );
        let mut applied = 0u32;
        let mut screen_on = true;
        loop {
            let setting =
                crate::encoder::control::DISPLAY.load(core::sync::atomic::Ordering::Relaxed);
            let on = setting & crate::encoder::control::ENABLED != 0;
            if setting != applied {
                if !on {
                    backlight.ch3().set_duty_cycle_fully_off();
                }
                if on != screen_on {
                    for screen in 0..SCREEN_COUNT {
                        self.command(screen, if on { 0x29 } else { 0x28 }, &[])?;
                    }
                    screen_on = on;
                    epoch = Instant::now();
                    frames = 0;
                    total_bytes = 0;
                    busy_us = 0;
                }
                if on {
                    backlight
                        .ch3()
                        .set_duty_cycle_percent((setting & 255) as u8);
                }
                applied = setting;
                defmt::info!("Displays applied: on={}, brightness={}%", on, setting & 255);
            }
            if !on {
                Timer::after_millis(10).await;
                continue;
            }
            let start = Instant::now();
            // Wall-clock phase avoids slowing the wave when frame rate changes.
            wave::heights(start.as_millis() as u32, &mut next);
            for tile in 0..512 / wave::TILE_WIDTH {
                let x = tile * wave::TILE_WIDTH;
                for layer in 0..wave::LAYERS {
                    let (y, height, len) = wave::render_tile(
                        x,
                        Some(layer),
                        &previous[x..x + wave::TILE_WIDTH],
                        &next[x..x + wave::TILE_WIDTH],
                        &mut buffer,
                    );
                    self.rectangle(
                        x / WIDTH,
                        (x % WIDTH) as u8,
                        y,
                        wave::TILE_WIDTH as u8,
                        height,
                        &buffer[..len],
                    )
                    .await?;
                    total_bytes += len as u64;
                }
            }
            previous = next;
            frames += 1;
            busy_us += start.elapsed().as_micros();
            // Cap at 30 Hz; avoid catch-up bursts if rendering exceeds the budget.
            Timer::at(start + Duration::from_micros(33_333)).await;
            let elapsed = epoch.elapsed().as_micros();
            if elapsed >= 5_000_000 {
                defmt::info!(
                    "Ocean stats: fps_x10={}, mean_update_us={}, pixel_bytes_per_frame={}",
                    frames * 10_000_000 / elapsed,
                    busy_us / frames,
                    total_bytes / frames
                );
                epoch = Instant::now();
                frames = 0;
                total_bytes = 0;
                busy_us = 0;
            }
        }
    }
}

// Panel-specific power, frame-rate and gamma settings from Waveshare.
const INIT: &[(u8, &[u8])] = &[
    (0x21, &[]), // IPS panel requires inversion.
    (0xb1, &[0x05, 0x3a, 0x3a]),
    (0xb2, &[0x05, 0x3a, 0x3a]),
    (0xb3, &[0x05, 0x3a, 0x3a, 0x05, 0x3a, 0x3a]),
    (0xb4, &[0x03]),
    (0xc0, &[0x62, 0x02, 0x04]),
    (0xc1, &[0xc0]),
    (0xc2, &[0x0d, 0x00]),
    (0xc3, &[0x8d, 0x6a]),
    (0xc4, &[0x8d, 0xee]),
    (0xc5, &[0x0e]),
    (
        0xe0,
        &[
            0x10, 0x0e, 0x02, 0x03, 0x0e, 0x07, 0x02, 0x07, 0x0a, 0x12, 0x27, 0x37, 0x00, 0x0d,
            0x0e, 0x10,
        ],
    ),
    (
        0xe1,
        &[
            0x10, 0x0e, 0x03, 0x03, 0x0f, 0x06, 0x02, 0x08, 0x0a, 0x13, 0x26, 0x36, 0x00, 0x0d,
            0x0e, 0x10,
        ],
    ),
    (0x3a, &[0x05]), // RGB565.
    (0x36, &[0xc8]), // Portrait coordinates, mirror X/Y, BGR.
];

fn glyph(c: u8) -> [u8; 7] {
    match c {
        b'H' => [17, 17, 17, 31, 17, 17, 17],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'L' => [16, 16, 16, 16, 16, 16, 31],
        b'O' | b'0' => [14, 17, 17, 17, 17, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        _ => [0; 7],
    }
}

fn text_pixel(x: usize, y: usize, text: &[u8], ox: usize, oy: usize, scale: usize) -> bool {
    if x < ox || y < oy {
        return false;
    }
    let x = (x - ox) / scale;
    let y = (y - oy) / scale;
    let index = x / 6;
    y < 7 && index < text.len() && x % 6 < 5 && glyph(text[index])[y] & (1 << (4 - x % 6)) != 0
}

fn pixel(x: usize, y: usize, screen: usize) -> u16 {
    let number = [
        b'0' + ((screen + 1) / 10) as u8,
        b'0' + ((screen + 1) % 10) as u8,
    ];
    if x == 0 || x == 127 || y == 0 || y == 127 {
        return 0xffff;
    }
    if text_pixel(x, y, b"HELLO", 20, 20, 3) || text_pixel(x, y, &number, 31, 55, 6) {
        return 0xffff;
    }
    // Red/green/blue strip identifies orientation and color order.
    if (112..124).contains(&y) {
        return [0xf800, 0x07e0, 0x001f][(x * 3 / WIDTH).min(2)];
    }
    0x0841
}

#[embassy_executor::task]
pub async fn run(
    mut screens: Screens,
    timer: Peri<'static, peripherals::TIM2>,
    pin: Peri<'static, peripherals::PA9>,
) {
    let mut backlight = SimplePwm::new(
        timer,
        None,
        None,
        Some(PwmPin::new(pin, OutputType::PushPull)),
        None,
        Hertz(20_000),
        CountingMode::EdgeAlignedUp,
    );
    backlight.ch3().set_duty_cycle_fully_off();
    backlight.ch3().enable();
    match screens.init().await {
        Ok(()) => {
            defmt::info!("ScreenKeys ready; encoder controls backlight, 20 kHz PWM");
            if let Err(e) = screens.animate(&mut backlight).await {
                backlight.ch3().set_duty_cycle_fully_off();
                defmt::error!("Wave stopped: SPI error {}", e);
            }
        }
        Err(e) => defmt::error!("ScreenKey SPI failure: {}; backlights remain off", e),
    }
    // Retain GPIO and peripheral ownership while static images are displayed.
    core::future::pending::<()>().await;
}
