use core::sync::atomic::{AtomicU32, Ordering};

pub const ENABLED: u32 = 1 << 8;
pub static DISPLAY: AtomicU32 = AtomicU32::new(ENABLED | 10);

pub struct Control {
    pub setting: u32,
    accepted: u16,
    candidate: u16,
    count_since: u64,
    pressed: bool,
    press_candidate: bool,
    press_since: u64,
}

impl Control {
    pub fn count(&self) -> u16 {
        self.accepted
    }
    pub fn pressed(&self) -> bool {
        self.pressed
    }
    pub fn new(count: u16, pressed: bool) -> Self {
        Self {
            setting: ENABLED | 10,
            accepted: count,
            candidate: count,
            count_since: 0,
            pressed,
            press_candidate: pressed,
            press_since: 0,
        }
    }

    pub fn sample(&mut self, now: u64, count: u16, pressed: bool) -> bool {
        let before = self.setting;
        if count != self.candidate {
            self.candidate = count;
            self.count_since = now;
        }
        if count != self.accepted && now - self.count_since >= 3 {
            let delta = count.wrapping_sub(self.accepted) as i16;
            self.accepted = count;
            let brightness = ((self.setting & 255) as i32 + delta as i32 * 2).clamp(2, 50);
            self.setting = (self.setting & ENABLED) | brightness as u32;
        }
        if pressed != self.press_candidate {
            self.press_candidate = pressed;
            self.press_since = now;
        }
        if pressed != self.pressed && now - self.press_since >= 20 {
            self.pressed = pressed;
            if pressed {
                self.setting ^= ENABLED;
            }
        }
        if before != self.setting {
            DISPLAY.store(self.setting, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_bounce_wrap_limits_and_power_memory() {
        let mut c = Control::new(0, false);
        c.sample(1, 1, false);
        c.sample(2, 0, false); // Contact bounces back: no brightness change.
        assert!(!c.sample(6, 0, false));
        c.sample(7, 65535, false);
        c.sample(10, 65535, false);
        assert_eq!(c.setting, ENABLED | 8);
        c.sample(11, 0, false);
        c.sample(14, 0, false);
        assert_eq!(c.setting, ENABLED | 10);
        c.sample(15, 0, true);
        c.sample(17, 0, false);
        c.sample(19, 0, true);
        c.sample(39, 0, true);
        assert_eq!(c.setting, 10);
        assert!(!c.sample(500, 0, true)); // Holding does not repeat.
        c.sample(501, 100, true);
        c.sample(504, 100, true);
        assert_eq!(c.setting, 50); // Dial adjusts stored brightness while off.
        c.sample(505, 100, false);
        c.sample(525, 100, false);
        c.sample(526, 100, true);
        c.sample(546, 100, true);
        assert_eq!(c.setting, ENABLED | 50);
        c.sample(547, 0, true);
        c.sample(550, 0, true);
        assert_eq!(c.setting, ENABLED | 2);
    }
}
