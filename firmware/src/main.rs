#![no_std]
#![no_main]

use defmt::{info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts, dma,
    gpio::{Input, Level, Output, Pull, Speed},
    peripherals,
    spi::{self, Spi},
    time::Hertz,
};
use embassy_time::{Duration, Instant, Ticker, Timer};
use {defmt_rtt as _, panic_probe as _};

mod boot;
mod encoder;
mod rgb;
mod screens;
mod usb;

bind_interrupts!(struct SpiIrqs {
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
    DMA1_CHANNEL3 => dma::InterruptHandler<peripherals::DMA1_CH3>;
});

#[embassy_executor::task]
async fn heartbeat() {
    let mut ticker = Ticker::every(Duration::from_secs(1));
    loop {
        info!("alive: uptime={}s", Instant::now().as_secs());
        ticker.next().await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Default clocks use the internal 16 MHz HSI; the board has no crystal.
    let mut config = embassy_stm32::Config::default();
    config.rcc.hsi48 = Some(embassy_stm32::rcc::Hsi48Config {
        sync_from_usb: true,
    });
    config.rcc.mux.clk48sel = embassy_stm32::rcc::mux::Clk48sel::HSI48;
    let p = embassy_stm32::init(config);
    spawner.spawn(unwrap!(usb::run(p.USB, p.PA12, p.PA11)));

    // Deselect everything before enabling SPI. Keep these GPIO owners alive.
    let screen_enable = Output::new(p.PA10, Level::High, Speed::Low);
    let mut flash_cs = Output::new(p.PB6, Level::High, Speed::Low);
    let screen_address = [
        Output::new(p.PA4, Level::Low, Speed::Low),
        Output::new(p.PB2, Level::Low, Speed::Low),
        Output::new(p.PB10, Level::Low, Speed::Low),
        Output::new(p.PB11, Level::Low, Speed::Low),
    ];
    let screen_dc = Output::new(p.PB0, Level::Low, Speed::Low);
    let screen_reset = Output::new(p.PB1, Level::Low, Speed::Low);
    let mut leds = rgb::Leds::new(p.TIM1, p.PA8, p.DMA1_CH1);
    let mut pixels = [rgb::Rgb::default(); rgb::LED_COUNT];
    leds.write(&pixels).await;

    info!("Hello world! keeb-v2 STM32G431CB / Embassy");
    info!("USB CDC event stream enabled; starting displays and input tasks");
    spawner.spawn(unwrap!(heartbeat()));
    spawner.spawn(unwrap!(encoder::run(p.TIM3, p.PA6, p.PA7, p.PC13)));

    let mut config = spi::Config::default();
    config.frequency = Hertz(4_000_000);
    let mut spi = Spi::new(
        p.SPI1, p.PA5, p.PB5, p.PB4, p.DMA1_CH2, p.DMA1_CH3, SpiIrqs, config,
    );
    // Read-only JEDEC identity check. The screen decoder stays disabled.
    Timer::after_millis(10).await;
    let mut id = [0x9f_u8, 0, 0, 0];
    flash_cs.set_low();
    let result = spi.blocking_transfer_in_place(&mut id);
    flash_cs.set_high();
    match result {
        Ok(()) => {
            info!("SPI flash JEDEC ID: {=[u8]:x}", &id[1..]);
            if id[1..] != [0xef, 0x40, 0x18] {
                warn!("Expected W25Q128 ID ef 40 18; check population/wiring");
            }
        }
        Err(e) => warn!("SPI flash read failed: {}", e),
    }

    spawner.spawn(unwrap!(screens::run(
        screens::Screens {
            spi,
            enable: screen_enable,
            address: screen_address,
            dc: screen_dc,
            reset: screen_reset,
            flash_cs,
        },
        p.TIM2,
        p.PA9
    )));

    // COL2ROW diodes require active-high columns and pull-down rows.
    let mut cols = [
        Output::new(p.PA0, Level::Low, Speed::Low),
        Output::new(p.PA1, Level::Low, Speed::Low),
        Output::new(p.PA2, Level::Low, Speed::Low),
        Output::new(p.PA3, Level::Low, Speed::Low),
    ];
    let rows = [
        Input::new(p.PB12, Pull::Down),
        Input::new(p.PB13, Pull::Down),
        Input::new(p.PB14, Pull::Down),
        Input::new(p.PB15, Pull::Down),
    ];
    let mut stable = [false; 16];
    let mut candidate = [false; 16];
    let mut changed_at = [Instant::now(); 16];
    let mut ticker = Ticker::every(Duration::from_millis(1));
    info!("Matrix ready: top row SW1..SW4; press keys to see events");
    loop {
        let mut leds_changed = false;
        for (col, output) in cols.iter_mut().enumerate() {
            output.set_high();
            Timer::after_micros(10).await;
            for (row, input) in rows.iter().enumerate() {
                let key = row * 4 + col;
                let pressed = input.is_high();
                let now = Instant::now();
                if candidate[key] != pressed {
                    candidate[key] = pressed;
                    changed_at[key] = now;
                }
                if stable[key] != pressed && now - changed_at[key] >= Duration::from_millis(5) {
                    stable[key] = pressed;
                    usb::key(key as u8, pressed);
                    pixels[key] = if pressed {
                        rgb::Rgb {
                            r: 255,
                            g: 255,
                            b: 255,
                        }
                    } else {
                        rgb::Rgb::default()
                    };
                    leds_changed = true;
                    info!("SW{} row={} col={} pressed={}", key + 1, row, col, pressed);
                }
            }
            output.set_low();
        }
        if leds_changed {
            leds.write(&pixels).await;
        }
        ticker.next().await;
    }
}
