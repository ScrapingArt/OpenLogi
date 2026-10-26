//! Logitech Options+ reverse-engineered finger-velocity scroll acceleration.
//!
//! Models physical finger inches-per-second (IPS) on the MagSpeed wheel,
//! tracks smoothed velocity via exponential moving average (EMA), and evaluates
//! a 5-point piecewise linear acceleration curve.

use std::time::{Duration, Instant};

/// Breakpoints in finger velocity (inches per second).
const CURVE_X: [f64; 5] = [0.0, 0.3, 0.6, 60.0, 120.0];
/// Nominal acceleration gain at each breakpoint.
const CURVE_Y: [f64; 5] = [1.0, 1.0, 12.0, 12.0, 6.0];

/// Physical wheel conversion constant from reverse engineering (ticks per inch).
pub const TICKS_PER_INCH: f64 = 7.678_571_428_571_429;
/// Idle timeout after which smoothed velocity resets to zero.
const VELOCITY_IDLE_TIMEOUT: Duration = Duration::from_millis(250);

/// EMA weights matching Logitech Options+ binary: 95% history, 5% new sample.
const EMA_HISTORY_WEIGHT: f64 = 0.95;
const EMA_SAMPLE_WEIGHT: f64 = 0.05;

#[derive(Clone, Debug)]
pub struct WheelAcceleration {
    last_tick: Option<Instant>,
    smoothed_ips: f64,
}

impl Default for WheelAcceleration {
    fn default() -> Self {
        Self {
            last_tick: None,
            smoothed_ips: 0.0,
        }
    }
}

impl WheelAcceleration {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset smoothed velocity to baseline.
    pub fn reset(&mut self) {
        self.last_tick = None;
        self.smoothed_ips = 0.0;
    }

    /// Current smoothed velocity in finger inches per second.
    #[must_use]
    pub fn smoothed_velocity(&self) -> f64 {
        self.smoothed_ips
    }

    /// Process one incoming wheel delta (in ticks), update smoothed velocity,
    /// and return the accelerated wheel ticks.
    pub fn accelerate(&mut self, ticks: f64, now: Instant, user_sensitivity: f64) -> f64 {
        if ticks == 0.0 {
            return 0.0;
        }

        let dt = if let Some(prev) = self.last_tick {
            let elapsed = now.saturating_duration_since(prev);
            if elapsed >= VELOCITY_IDLE_TIMEOUT {
                self.smoothed_ips = 0.0;
                Duration::from_millis(100)
            } else {
                elapsed.max(Duration::from_millis(2))
            }
        } else {
            self.smoothed_ips = 0.0;
            Duration::from_millis(100)
        };
        self.last_tick = Some(now);

        let dt_secs = dt.as_secs_f64();
        let instant_ips = (ticks.abs() / (dt_secs * TICKS_PER_INCH)).min(200.0);
        self.smoothed_ips =
            (EMA_HISTORY_WEIGHT * self.smoothed_ips) + (EMA_SAMPLE_WEIGHT * instant_ips);

        let gain = evaluate_gain(self.smoothed_ips, user_sensitivity);
        ticks * gain
    }
}

/// Piecewise linear interpolation across the 5 curve breakpoints, scaled by user sensitivity.
#[must_use]
pub fn evaluate_gain(ips: f64, sensitivity: f64) -> f64 {
    let raw_gain = if ips <= CURVE_X[0] {
        CURVE_Y[0]
    } else if ips < CURVE_X[1] {
        interpolate(ips, CURVE_X[0], CURVE_X[1], CURVE_Y[0], CURVE_Y[1])
    } else if ips < CURVE_X[2] {
        interpolate(ips, CURVE_X[1], CURVE_X[2], CURVE_Y[1], CURVE_Y[2])
    } else if ips < CURVE_X[3] {
        interpolate(ips, CURVE_X[2], CURVE_X[3], CURVE_Y[2], CURVE_Y[3])
    } else if ips < CURVE_X[4] {
        interpolate(ips, CURVE_X[3], CURVE_X[4], CURVE_Y[3], CURVE_Y[4])
    } else {
        CURVE_Y[4]
    };

    // Scale with user scrolling speed: gain = 1.0 + (raw_gain - 1.0) * sensitivity
    1.0 + (raw_gain - 1.0) * sensitivity.max(0.1)
}

fn interpolate(x: f64, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    let t = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
    y0 + t * (y1 - y0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_velocity_stays_at_gain_one() {
        assert_eq!(evaluate_gain(0.0, 1.0), 1.0);
        assert_eq!(evaluate_gain(0.2, 1.0), 1.0);
        assert_eq!(evaluate_gain(0.3, 1.0), 1.0);
    }

    #[test]
    fn fast_flick_reaches_maximum_gain() {
        let gain = evaluate_gain(10.0, 1.0);
        assert!((gain - 12.0).abs() < 1e-4);
    }

    #[test]
    fn extreme_freewheel_tapers_gain() {
        let gain = evaluate_gain(120.0, 1.0);
        assert!((gain - 6.0).abs() < 1e-4);
    }

    #[test]
    fn sensitivity_scales_gain_properly() {
        // At sensitivity 0.5: gain = 1.0 + (12.0 - 1.0) * 0.5 = 6.5
        let gain = evaluate_gain(10.0, 0.5);
        assert!((gain - 6.5).abs() < 1e-4);
    }

    #[test]
    fn acceleration_smoothes_velocity_and_resets_on_idle() {
        let mut accel = WheelAcceleration::new();
        let now = Instant::now();

        // Single click after idle: gain is 1.0
        let val1 = accel.accelerate(1.0, now, 1.0);
        assert_eq!(val1, 1.0);

        // Rapid subsequent clicks accelerate
        let val2 = accel.accelerate(1.0, now + Duration::from_millis(15), 1.0);
        assert!(val2 >= val1);

        // Idle reset
        accel.accelerate(1.0, now + Duration::from_millis(500), 1.0);
        assert!(accel.smoothed_velocity() < 2.0);
    }
}
