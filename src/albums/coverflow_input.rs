//! Normalize wheel detents and bound a continuous touchpad gesture.
#[derive(Default)]
pub(super) struct ScrollNavigation {
    accumulated: f64,
    last_ms: Option<u32>,
    surface: bool,
    stepped: bool,
}
impl ScrollNavigation {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn step(&mut self, delta: f64, pixels: bool, now_ms: u32) -> i32 {
        if !delta.is_finite() || delta == 0.0 {
            return 0;
        }
        if self.surface != pixels
            || self
                .last_ms
                .is_some_and(|last| now_ms.wrapping_sub(last) > 250)
        {
            self.reset();
        }
        self.surface = pixels;
        self.last_ms = Some(now_ms);
        // GTK supplies no portable kinetic phase on all supported backends.
        // Bound a continuous surface gesture to one step, until stop or idle.
        if pixels && self.stepped {
            return 0;
        }
        if self.accumulated.signum() != delta.signum() {
            self.accumulated = 0.0;
        }
        self.accumulated += delta;
        let threshold = if pixels { 64.0 } else { 1.0 };
        if self.accumulated.abs() + 1e-6 < threshold {
            return 0;
        }
        let direction = self.accumulated.signum() as i32;
        self.accumulated = 0.0; // no backlog from a coalesced/large event
        self.stepped = pixels;
        direction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_wheel_detents_accumulate_without_skipping() {
        let mut scroll = ScrollNavigation::default();
        assert_eq!(scroll.step(0.25, false, 1), 0);
        assert_eq!(scroll.step(0.25, false, 2), 0);
        assert_eq!(scroll.step(0.5, false, 3), 1);
        assert_eq!(scroll.step(-1.0, false, 4), -1);
        assert_eq!(scroll.step(4.0, false, 5), 1);
        assert_eq!(scroll.step(0.0, false, 6), 0);
    }
    #[test]
    fn touchpad_threshold_reversal_and_gesture_limit() {
        let mut scroll = ScrollNavigation::default();
        assert_eq!(scroll.step(10.0, true, 1), 0);
        assert_eq!(scroll.step(20.0, true, 20), 0);
        assert_eq!(scroll.step(35.0, true, 40), 1);
        assert_eq!(
            scroll.step(400.0, true, 60),
            0,
            "one step per gesture bounds momentum"
        );
        scroll.reset();
        assert_eq!(scroll.step(30.0, true, 100), 0);
        assert_eq!(scroll.step(-30.0, true, 120), 0);
        assert_eq!(scroll.step(-35.0, true, 140), -1);
        assert_eq!(
            scroll.step(65.0, true, 600),
            1,
            "idle gap starts a new gesture"
        );
        assert_eq!(scroll.step(f64::NAN, true, 620), 0);
    }
}
