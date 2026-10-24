//! SK6812MINI-E: PA8 / TIM1_CH1 -> level shifter -> 16 LEDs.
//! DMA feeds timer compare values, so interrupts cannot stretch data pulses.
use embassy_stm32::{
    Peri, bind_interrupts, dma,
    gpio::OutputType,
    peripherals,
    time::Hertz,
    timer::{
        Channel,
        low_level::CountingMode,
        simple_pwm::{PwmPin, SimplePwm},
    },
};

bind_interrupts!(struct Irqs {
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
});

pub const LED_COUNT: usize = 16;
// 160 us of low both before and after each frame (>80 us required).
const RESET_SLOTS: usize = 128;
const FRAME_SLOTS: usize = RESET_SLOTS * 2 + LED_COUNT * 24;
// Conservative bring-up ceiling: at most ~36 mA for all 16 full-white pixels,
// excluding LED idle current, using the board's 36 mA/pixel estimate.
const MAX_CHANNEL: u8 = 16;

#[derive(Clone, Copy, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub struct Leds {
    pwm: SimplePwm<'static, peripherals::TIM1>,
    dma: Peri<'static, peripherals::DMA1_CH1>,
    waveform: [u16; FRAME_SLOTS],
}

impl Leds {
    pub fn new(
        timer: Peri<'static, peripherals::TIM1>,
        pin: Peri<'static, peripherals::PA8>,
        dma: Peri<'static, peripherals::DMA1_CH1>,
    ) -> Self {
        let mut pwm = SimplePwm::new(
            timer,
            Some(PwmPin::new(pin, OutputType::PushPull)),
            None,
            None,
            None,
            Hertz(800_000),
            CountingMode::EdgeAlignedUp,
        );
        // The current 16 MHz timer clock gives 20 ticks per 1.25 us bit.
        // Fail explicitly if clock changes invalidate the encoded pulse widths.
        assert_eq!(pwm.max_duty_cycle(), 20);
        pwm.ch1().set_duty_cycle_fully_off();
        Self {
            pwm,
            dma,
            waveform: [0; FRAME_SLOTS],
        }
    }

    pub async fn write(&mut self, pixels: &[Rgb; LED_COUNT]) {
        let mut offset = RESET_SLOTS;
        for pixel in pixels {
            // On-wire order is GRB, most significant bit first.
            for channel in [pixel.g, pixel.r, pixel.b] {
                let channel = ((channel as u16 * MAX_CHANNEL as u16) / 255) as u8;
                for bit in (0..8).rev() {
                    // 0: 312.5 ns high, 937.5 ns low.
                    // 1: 625 ns high, 625 ns low.
                    self.waveform[offset] = if channel & (1 << bit) == 0 { 5 } else { 10 };
                    offset += 1;
                }
            }
        }
        self.pwm
            .waveform_up(self.dma.reborrow(), Irqs, Channel::Ch1, &self.waveform)
            .await;
        // Trailing zero slots also flush the timer's compare preload pipeline.
        // Leave the timer running at zero duty between frames.
    }
}
