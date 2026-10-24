//! EC11E15244G1: 30 detents / 15 pulses; hardware x2 = one count/detent.
pub mod control;
use embassy_stm32::{
    Peri,
    gpio::{Input, Pull},
    peripherals,
    timer::qei::{Config, Qei, QeiMode},
};
use embassy_time::{Duration, Instant, Ticker};

#[embassy_executor::task]
pub async fn run(
    timer: Peri<'static, peripherals::TIM3>,
    a: Peri<'static, peripherals::PA6>,
    b: Peri<'static, peripherals::PA7>,
    button: Peri<'static, peripherals::PC13>,
) {
    let qei = Qei::new(
        timer,
        a,
        b,
        Config {
            ch1_pull: Pull::Up,
            ch2_pull: Pull::Up,
            mode: QeiMode::Mode1,
            ..Default::default()
        },
    );
    let button = Input::new(button, Pull::Up);
    let mut control = control::Control::new(qei.count(), button.is_low());
    crate::usb::encoder(control.count(), control.pressed(), 0);
    let mut ticker = Ticker::every(Duration::from_millis(1));
    defmt::info!("Encoder ready: TIM3 x2, dial brightness 2..50%, push toggles screens");
    loop {
        let old_count = control.count();
        let old_pressed = control.pressed();
        if control.sample(Instant::now().as_millis(), qei.count(), button.is_low()) {
            defmt::info!(
                "Encoder: count={}, screens_on={}, brightness={}%, button_pressed={}",
                qei.count(),
                control.setting & control::ENABLED != 0,
                control.setting & 255,
                button.is_low()
            );
        }
        if control.count() != old_count || control.pressed() != old_pressed {
            crate::usb::encoder(
                control.count(),
                control.pressed(),
                control.count().wrapping_sub(old_count) as i16,
            );
        }
        ticker.next().await;
    }
}
