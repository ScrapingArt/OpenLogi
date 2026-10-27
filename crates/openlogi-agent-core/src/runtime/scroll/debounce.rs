//! Wheel debouncing, directional hysteresis, and velocity-adaptive fluidity.
//!
//! Reverse-engineered from Logitech Options+ macOS daemon (`logioptionsplus_agent`),
//! implementing:
//! - `WheelHysteresis`: Direction-flip rebound damping and mechanical oscillation absorption.
//! - `WheelDisturbanceFilter`: Suppression of accidental micro-scrolls during middle-button clicks.
//! - `WheelFluidity`: Velocity estimation via Exponential Moving Average (EMA) for fluid scrolling.

use std::time::{Duration, Instant};

use openlogi_core::scroll::ScrollDelta;

use super::{ANIMATION_DURATION, WheelDelta};

/// Relaxation time window for directional hysteresis accumulators (1.0 second in Logi Options+).
const HYSTERESIS_RELAX_WINDOW: Duration = Duration::from_millis(1000);

/// Middle-click disturbance suppression window (150ms in Logi Options+).
const DISTURBANCE_ACTIVE_TIMEOUT: Duration = Duration::from_millis(150);

/// Exponential Moving Average smoothing factor for wheel velocity (0.05 / 0.95 in Logi Options+).
const SPEED_EMA_ALPHA: f64 = 0.05;

/// Pause threshold after which wheel momentum is considered completely idle.
const FLUIDITY_IDLE_TIMEOUT: Duration = Duration::from_millis(200);

/// Single-axis hysteresis state to absorb reverse rebound and mechanical wobble.
#[derive(Clone, Copy, Debug, Default)]
struct AxisHysteresis {
    positive_accumulator: f64,
    negative_accumulator: f64,
}

impl AxisHysteresis {
    fn relax(&mut self, relax_steps: f64) {
        self.positive_accumulator = (self.positive_accumulator - relax_steps).max(0.0);
        self.negative_accumulator = (self.negative_accumulator - relax_steps).max(0.0);
    }

    fn filter(&mut self, value: f64, maximum_accumulator: f64) -> f64 {
        if value > 0.0 {
            let cancellation = self.negative_accumulator.min(value);
            self.negative_accumulator -= cancellation;
            let effective = value - cancellation;
            self.positive_accumulator =
                (self.positive_accumulator + effective).min(maximum_accumulator);
            effective
        } else if value < 0.0 {
            let absolute_value = -value;
            let cancellation = self.positive_accumulator.min(absolute_value);
            self.positive_accumulator -= cancellation;
            let effective = absolute_value - cancellation;
            self.negative_accumulator =
                (self.negative_accumulator + effective).min(maximum_accumulator);
            -effective
        } else {
            0.0
        }
    }

    fn reset(&mut self) {
        self.positive_accumulator = 0.0;
        self.negative_accumulator = 0.0;
    }
}

/// Directional hysteresis and rebound debounce filter.
///
/// Prevents the physical mouse wheel spring rebound (accidental reverse tick
/// when scrolling stops or flicks) from jerking the screen.
#[derive(Clone, Debug, Default)]
pub struct WheelHysteresis {
    x_axis: AxisHysteresis,
    y_axis: AxisHysteresis,
    last_timestamp: Option<Instant>,
}

impl WheelHysteresis {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.x_axis.reset();
        self.y_axis.reset();
        self.last_timestamp = None;
    }

    #[must_use]
    pub(crate) fn filter(&mut self, delta: WheelDelta, at: Instant) -> WheelDelta {
        const MAXIMUM_ACCUMULATOR: f64 = 2.0;

        if let Some(last) = self.last_timestamp {
            let elapsed = at.saturating_duration_since(last);
            if elapsed >= HYSTERESIS_RELAX_WINDOW {
                let relax_steps =
                    (elapsed.as_secs_f64() / HYSTERESIS_RELAX_WINDOW.as_secs_f64()).floor();
                self.x_axis.relax(relax_steps);
                self.y_axis.relax(relax_steps);
            }
        }
        self.last_timestamp = Some(at);

        let filtered_x = self.x_axis.filter(delta.x, MAXIMUM_ACCUMULATOR);
        let filtered_y = self.y_axis.filter(delta.y, MAXIMUM_ACCUMULATOR);

        WheelDelta {
            x: filtered_x,
            y: filtered_y,
        }
    }
}

/// Middle button disturbance filter.
///
/// Clicking the mouse wheel can jolt it by a notch or two. Like Logi Options+,
/// drops wheel input for `DISTURBANCE_ACTIVE_TIMEOUT` after the press; the
/// release does not extend it. (The reference also keeps swallowing a wheel
/// that was already spinning at the press until it changes direction or
/// speeds up; captured Options+ output around clicks showed no effect from
/// either rule, so only the fixed window is kept.)
#[derive(Clone, Debug, Default)]
pub struct WheelDisturbanceFilter {
    press_instant: Option<Instant>,
}

impl WheelDisturbanceFilter {
    pub fn on_middle_button(&mut self, pressed: bool, at: Instant) {
        if pressed {
            self.press_instant = Some(at);
        }
    }

    #[must_use]
    pub fn is_disturbed(&self, at: Instant) -> bool {
        self.press_instant
            .is_some_and(|press| at.saturating_duration_since(press) <= DISTURBANCE_ACTIVE_TIMEOUT)
    }
}

/// Combined debouncer protecting against middle-click disturbance and rebound ticks.
#[derive(Clone, Debug, Default)]
pub struct WheelDebouncer {
    disturbance: WheelDisturbanceFilter,
    hysteresis: WheelHysteresis,
}

impl WheelDebouncer {
    pub fn on_middle_button(&mut self, pressed: bool, at: Instant) {
        self.disturbance.on_middle_button(pressed, at);
    }

    pub fn reset(&mut self) {
        self.disturbance = WheelDisturbanceFilter::default();
        self.hysteresis.reset();
    }

    #[must_use]
    pub fn filter_scroll(&mut self, delta: ScrollDelta, at: Instant) -> Option<ScrollDelta> {
        if self.disturbance.is_disturbed(at) {
            return None;
        }
        let ScrollDelta::WheelTicks { x, y } = delta else {
            return Some(delta);
        };
        let impulse = WheelDelta { x, y };
        let filtered = self.hysteresis.filter(impulse, at);
        if filtered.is_zero() {
            None
        } else {
            Some(ScrollDelta::wheel_ticks(filtered.x, filtered.y))
        }
    }
}

/// Smooth-scroll duration adapter tracking velocity via Exponential Moving Average.
#[derive(Clone, Debug)]
pub struct WheelFluidity {
    last_impulse: Option<Instant>,
    smoothed_speed: f64,
}

impl Default for WheelFluidity {
    fn default() -> Self {
        Self {
            last_impulse: None,
            smoothed_speed: 0.0,
        }
    }
}

impl WheelFluidity {
    pub(crate) fn adapt(&mut self, delta: WheelDelta, at: Instant) -> Duration {
        let Some(last) = self.last_impulse else {
            self.last_impulse = Some(at);
            return ANIMATION_DURATION;
        };
        let elapsed = at.saturating_duration_since(last);
        if elapsed > FLUIDITY_IDLE_TIMEOUT {
            self.smoothed_speed = 0.0;
            self.last_impulse = Some(at);
            return ANIMATION_DURATION;
        }
        let elapsed_seconds = elapsed.as_secs_f64().clamp(0.005, 0.2);
        self.last_impulse = Some(at);

        let magnitude = (delta.x * delta.x + delta.y * delta.y).sqrt();
        let instant_speed = magnitude / elapsed_seconds;
        self.smoothed_speed =
            (1.0 - SPEED_EMA_ALPHA) * self.smoothed_speed + SPEED_EMA_ALPHA * instant_speed;

        if self.smoothed_speed < 3.0 {
            ANIMATION_DURATION
        } else if self.smoothed_speed < 10.0 {
            Duration::from_millis(95)
        } else {
            Duration::from_millis(75)
        }
    }

    pub fn reset(&mut self) {
        self.last_impulse = None;
        self.smoothed_speed = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hysteresis_absorbs_rebound_tick() {
        let mut filter = WheelHysteresis::new();
        let base = Instant::now();

        let f1 = filter.filter(WheelDelta { x: 0.0, y: -3.0 }, base);
        assert_eq!(f1, WheelDelta { x: 0.0, y: -3.0 });

        let f2 = filter.filter(
            WheelDelta { x: 0.0, y: 1.0 },
            base + Duration::from_millis(20),
        );
        assert_eq!(f2, WheelDelta::ZERO);

        let f3 = filter.filter(
            WheelDelta { x: 0.0, y: 1.0 },
            base + Duration::from_millis(40),
        );
        assert_eq!(f3, WheelDelta::ZERO);

        let f4 = filter.filter(
            WheelDelta { x: 0.0, y: 1.0 },
            base + Duration::from_millis(60),
        );
        assert_eq!(f4, WheelDelta { x: 0.0, y: 1.0 });
    }

    #[test]
    fn rebound_absorption_does_not_dampen_subsequent_intentional_scroll() {
        let mut filter = WheelHysteresis::new();
        let base = Instant::now();

        let step1 = filter.filter(WheelDelta { x: 0.0, y: 1.0 }, base);
        assert_eq!(step1, WheelDelta { x: 0.0, y: 1.0 });

        let rebound = filter.filter(
            WheelDelta { x: 0.0, y: -0.5 },
            base + Duration::from_millis(20),
        );
        assert_eq!(rebound, WheelDelta::ZERO);

        let step2 = filter.filter(
            WheelDelta { x: 0.0, y: 1.0 },
            base + Duration::from_millis(40),
        );
        assert_eq!(
            step2,
            WheelDelta { x: 0.0, y: 1.0 },
            "subsequent intentional scroll must not be dampened by prior absorbed rebound"
        );
    }

    #[test]
    fn idle_period_does_not_swallow_first_scroll_tick() {
        let mut filter = WheelHysteresis::new();
        let base = Instant::now();

        // Idle for 5 seconds from start
        let first_tick =
            filter.filter(WheelDelta { x: 0.0, y: 1.0 }, base + Duration::from_secs(5));
        assert_eq!(
            first_tick,
            WheelDelta { x: 0.0, y: 1.0 },
            "first tick after idle pause must not be swallowed"
        );
    }

    #[test]
    fn disturbance_filter_blocks_scroll_during_middle_click() {
        let mut filter = WheelDisturbanceFilter::default();
        let base = Instant::now();

        assert!(!filter.is_disturbed(base));
        filter.on_middle_button(true, base);

        assert!(filter.is_disturbed(base + Duration::from_millis(50)));
        assert!(filter.is_disturbed(base + Duration::from_millis(150)));
        assert!(!filter.is_disturbed(base + Duration::from_millis(200)));

        // Releasing neither extends nor ends the window.
        filter.on_middle_button(false, base + Duration::from_millis(100));
        assert!(filter.is_disturbed(base + Duration::from_millis(140)));
        assert!(!filter.is_disturbed(base + Duration::from_millis(160)));
    }

    #[test]
    fn fluidity_resets_on_idle_timeout() {
        let mut fluidity = WheelFluidity::default();
        let base = Instant::now();

        // First tick
        assert_eq!(
            fluidity.adapt(WheelDelta { x: 0.0, y: 1.0 }, base),
            ANIMATION_DURATION
        );

        // Rapid second tick (fast burst)
        let fast_duration = fluidity.adapt(
            WheelDelta { x: 0.0, y: 2.0 },
            base + Duration::from_millis(15),
        );
        assert!(fast_duration < ANIMATION_DURATION);

        // Wait 1 second (idle pause)
        let after_pause =
            fluidity.adapt(WheelDelta { x: 0.0, y: 1.0 }, base + Duration::from_secs(1));
        assert_eq!(
            after_pause, ANIMATION_DURATION,
            "subsequent single click after pause must reset to standard duration"
        );
    }
}
