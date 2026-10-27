//! Pure model of the Logi Options+ mouse-wheel pipeline (macOS agent).
//!
//! Each wheel report becomes one pixel delta, shaped only by an EMA-smoothed
//! speed that selects a gain; there is no timer, easing or momentum. The
//! constants and curve reproduce the observed behaviour of the vendor's
//! reference implementation, validated against recorded event traces (see the
//! replay tests). Nothing here reads a clock or does I/O; callers pass
//! `Instant`s. The reference's direction-change hysteresis is modelled
//! ([`StepHysteresis`]); its ratchet detection is not, and the middle-click disturbance filter runs upstream of the model.
//!
//! Timing caveat: the EMA resets on any gap of 100 ms or more, so a wheel that
//! ticks at 10 notches per second or slower always runs at gain 1 and one
//! notch is a fixed ~13 px; the gain only builds up above that rate. On the
//! hook path the interval comes from the instant the tap callback ran rather
//! than from the OS event's own timestamp, so events delivered in one batch
//! collapse onto the 8 ms floor and read as a faster flick than they were.

use std::time::Instant;

/// One notch moves the pointer `1 / 7.714285714` of an inch.
const NOTCHES_PER_INCH: f64 = 7.714_285_714_285_714;
/// Lower clamp of the per-report period in seconds.
const PERIOD_MIN_SECS: f64 = 0.008;
/// Upper clamp of the per-report period, and the gap at which the EMA resets.
const PERIOD_MAX_SECS: f64 = 0.1;
/// EMA retention and new-sample weight.
const EMA_HISTORY_WEIGHT: f64 = 0.95;
const EMA_SAMPLE_WEIGHT: f64 = 0.05;
/// Screen DPI the reference assumes for a mouse wheel.
const SCREEN_DPI: f64 = 100.0;
/// The speed setting in [-1,1] maps linearly onto this range.
const SPEED_MIN: f64 = 0.06;
const SPEED_MAX: f64 = 1.2;
/// Curve thresholds in EMA inches/second.
const CURVE_X: [f64; 5] = [0.0, 0.3, 0.6, 60.0, 120.0];
/// Base gains at [`CURVE_X`]. Each is scaled to `1 + (G - 1) * s`. The
/// reference table has a sixth entry (-1) that makes the gain at or above
/// 120 ips `1 - 2s`, a step to zero or below. That region is deliberately not
/// reproduced: see [`curve_gain`].
const CURVE_Y: [f64; 5] = [1.0, 1.0, 12.0, 12.0, 6.0];
/// Pixels per line unit.
const PIXELS_PER_LINE: f64 = 10.0;
/// Hi-res steps per notch the reference's hysteresis counts in (the wheel's
/// `0x2121` multiplier, 15 on the MX Master 3S).
const STEPS_PER_NOTCH: f64 = 15.0;
/// Steps of dead band held after an idle gap, and the cap on how much travel
/// is remembered, both in ratchet mode. These are the reference's ratchet
/// defaults (base 1, cap 1). The effect is a step or two, about
/// a pixel: scored by per-burst pixel totals on two recordings of Options+,
/// variants from none to the reference's free-spin base of 4 differ by under
/// 0.1%, so the data neither confirms nor rules out the values. The point of
/// carrying them is the unit: reversals lose at most one hi-res step, not the
/// notches the whole-notch filter this replaced swallowed.
const HYSTERESIS_BASE_STEPS: f64 = 1.0;
const HYSTERESIS_CAP_STEPS: f64 = 1.0;
/// An idle gap this long relaxes the accumulators, one step per whole period.
const HYSTERESIS_RELAX_SECS: f64 = 1.0;
/// Steps below this are rounding dust, not wheel movement.
const STEP_EPSILON: f64 = 1e-6;

/// Direction-change dead band of the reference, in hi-res steps.
///
/// Travel in one direction is remembered (up to a cap) and cancels the first
/// steps of travel the other way; after an idle gap both memories relax to the
/// base, so the first step of any new movement is swallowed.
#[derive(Clone, Debug, Default)]
struct StepHysteresis {
    negative: f64,
    positive: f64,
    last: Option<Instant>,
}

impl StepHysteresis {
    fn relax_toward_base(value: f64, steps: f64) -> f64 {
        if value >= HYSTERESIS_BASE_STEPS {
            (value - steps).max(HYSTERESIS_BASE_STEPS)
        } else {
            (value + steps).min(HYSTERESIS_BASE_STEPS)
        }
    }

    /// Filter signed `steps` arriving at `now`; the result keeps the sign (or
    /// is zero) and is never larger in magnitude.
    fn filter(&mut self, steps: f64, now: Instant) -> f64 {
        if let Some(last) = self.last {
            let gap = now.saturating_duration_since(last).as_secs_f64();
            if gap >= HYSTERESIS_RELAX_SECS {
                let relax = (gap / HYSTERESIS_RELAX_SECS).floor();
                self.negative = Self::relax_toward_base(self.negative, relax);
                self.positive = Self::relax_toward_base(self.positive, relax);
            }
        }
        self.last = Some(now);
        if steps > 0.0 {
            self.positive = (self.positive + steps).min(HYSTERESIS_CAP_STEPS);
            let used = self.negative.min(steps);
            self.negative -= used;
            steps - used
        } else if steps < 0.0 {
            self.negative = (self.negative - steps).min(HYSTERESIS_CAP_STEPS);
            let used = self.positive.min(-steps);
            self.positive -= used;
            steps + used
        } else {
            0.0
        }
    }
}

/// Result of one [`LoWheelModel::process`] call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoWheelOutput {
    /// Whole pixels to post this event (sub-pixel remainder is carried).
    pub pixels: i32,
    /// Line delta (`accumulated pixels / 10`, at least 1 in magnitude when
    /// non-zero); zero while fewer than 10 pixels have accumulated. Posted as
    /// `trunc` in the integer line field and `round` in the fixed-point field,
    /// which is what the captured Options+ events carry.
    pub line_delta: f64,
    /// Gain applied to this report (diagnostics).
    pub gain: f64,
    /// EMA of |finger inches/second| after this report (diagnostics).
    pub ema_ips: f64,
}

/// Stateful Options+ wheel model for one axis.
#[derive(Clone, Debug)]
pub struct LoWheelModel {
    speed_scalar: f64,
    ema_ips: f64,
    last_event: Option<Instant>,
    pixel_remainder: f64,
    line_pixels: i64,
    hysteresis: StepHysteresis,
}

impl LoWheelModel {
    /// `speed_setting` is the UI value in [-1,1] (clamped).
    #[must_use]
    pub fn new(speed_setting: f64) -> Self {
        Self {
            speed_scalar: speed_scalar(speed_setting),
            ema_ips: 0.0,
            last_event: None,
            pixel_remainder: 0.0,
            line_pixels: 0,
            hysteresis: StepHysteresis::default(),
        }
    }

    /// Update the user speed setting in [-1,1] without touching history.
    pub fn set_speed_setting(&mut self, speed_setting: f64) {
        self.speed_scalar = speed_scalar(speed_setting);
    }

    /// Process one wheel report of signed `notches` (fractional: a device
    /// reporting 15 hi-res steps per notch passes `steps / 15`, and the
    /// multiplier cancels out of the physics), arriving at `now`. Zero or
    /// non-finite notches return zero output without touching state.
    pub fn process(&mut self, notches: f64, now: Instant) -> LoWheelOutput {
        if notches == 0.0 || !notches.is_finite() {
            return LoWheelOutput {
                pixels: 0,
                line_delta: 0.0,
                gain: 1.0,
                ema_ips: self.ema_ips,
            };
        }
        let steps = self.hysteresis.filter(notches * STEPS_PER_NOTCH, now);
        if steps.abs() < STEP_EPSILON {
            // Swallowed by the dead band: like the reference, such a report
            // neither posts nor feeds the speed estimate.
            return LoWheelOutput {
                pixels: 0,
                line_delta: 0.0,
                gain: 1.0,
                ema_ips: self.ema_ips,
            };
        }
        let notches = steps / STEPS_PER_NOTCH;
        let period = match self.last_event {
            Some(previous) => {
                let gap = now.saturating_duration_since(previous).as_secs_f64();
                if gap >= PERIOD_MAX_SECS {
                    self.ema_ips = 0.0;
                }
                gap.clamp(PERIOD_MIN_SECS, PERIOD_MAX_SECS)
            }
            // First event: no reset of the EMA (it starts at 0), T = 0.1.
            None => PERIOD_MAX_SECS,
        };
        self.last_event = Some(now);

        let inches = notches / NOTCHES_PER_INCH;
        let finger_ips = inches / period;
        self.ema_ips = EMA_HISTORY_WEIGHT * self.ema_ips + EMA_SAMPLE_WEIGHT * finger_ips.abs();
        let gain = curve_gain(self.ema_ips, self.speed_scalar);

        // pixels = T * (fingerIPS * gain) * dpi == inches * gain * dpi.
        let exact = period * (finger_ips * gain) * SCREEN_DPI + self.pixel_remainder;
        // The integer part is posted, the fractional part carried.
        let whole = exact.trunc();
        self.pixel_remainder = exact - whole;
        let pixels = trunc_to_i32(whole);

        self.line_pixels += i64::from(pixels);
        let line_delta = self.take_line_delta();
        LoWheelOutput {
            pixels,
            line_delta,
            gain,
            ema_ips: self.ema_ips,
        }
    }

    /// Simplified flush of the injector line accumulator: the reference
    /// flushes on an inch threshold that is not modelled here; this flushes
    /// once |pixels| >= 10. Totals match the captured lines; the flush
    /// cadence does not.
    fn take_line_delta(&mut self) -> f64 {
        // Pixel counts are small; exact in f64.
        #[expect(
            clippy::cast_precision_loss,
            reason = "accumulator stays far below 2^52"
        )]
        let lines = self.line_pixels as f64 / PIXELS_PER_LINE;
        if lines.abs() < 1.0 {
            return 0.0;
        }
        self.line_pixels = 0;
        lines
    }
}

/// `s = lo + (v - min)/(max - min) * (hi - lo)` with v clamped to [-1,1].
fn speed_scalar(setting: f64) -> f64 {
    let clamped = if setting.is_finite() {
        setting.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    SPEED_MIN + f64::midpoint(clamped, 1.0) * (SPEED_MAX - SPEED_MIN)
}

/// Gain for smoothed speed `ema_ips` and speed scalar `s`.
///
/// Piecewise linear over [`CURVE_X`]: 0.3-0.6 ramps up, 0.6-60 is flat,
/// 60-120 ramps down to `1 + 5s`.
///
/// At or above 120 ips the reference steps to `1 - 2s`, which is zero or
/// negative for `s >= 0.5` (the default is 0.63): the wheel would stop or
/// reverse. This model deliberately deviates and holds the gain reached at the
/// end of the ramp, `1 + 5s`, which is always positive. The region is
/// unreachable for a real wheel (120 ips is over 900 notches per second), but
/// the hook path feeds OS line deltas that can be many notches per event, so
/// the guard keeps a fast flick from ever reversing direction.
fn curve_gain(ema_ips: f64, s: f64) -> f64 {
    let gain_at = |index: usize| 1.0 + (CURVE_Y[index] - 1.0) * s;
    let speed = ema_ips.abs();
    for segment in 1..CURVE_X.len() {
        if speed < CURVE_X[segment] {
            let t = (speed - CURVE_X[segment - 1]) / (CURVE_X[segment] - CURVE_X[segment - 1]);
            return gain_at(segment - 1) + t * (gain_at(segment) - gain_at(segment - 1));
        }
    }
    gain_at(CURVE_Y.len() - 1)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "saturating float-to-int, value pre-truncated"
)]
fn trunc_to_i32(whole: f64) -> i32 {
    whole as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Notches a 15-steps-per-notch device reports for `steps` hi-res steps.
    fn notches(steps: i64) -> f64 {
        #[expect(clippy::cast_precision_loss, reason = "test steps are tiny")]
        let steps = steps as f64;
        steps / 15.0
    }

    #[test]
    fn speed_mapping_matches_native_range() {
        assert!((speed_scalar(-1.0) - 0.06).abs() < 1e-12);
        assert!((speed_scalar(1.0) - 1.2).abs() < 1e-12);
        assert!((speed_scalar(0.0) - 0.63).abs() < 1e-12);
        assert!((speed_scalar(5.0) - 1.2).abs() < 1e-12);
    }

    #[test]
    fn slow_single_tick_is_about_one_notch_at_gain_one() {
        let mut model = LoWheelModel::new(0.0);
        let out = model.process(1.0, Instant::now());
        // 100 / 7.7142857 = 12.963 px; first event T=0.1 -> ema 0.065 < 0.3.
        assert!((out.gain - 1.0).abs() < 1e-12);
        assert_eq!(out.pixels, 12);
        assert!((model.pixel_remainder - 0.963).abs() < 1e-3);
    }

    #[test]
    fn default_speed_reaches_plateau_gain() {
        let mut model = LoWheelModel::new(0.0);
        let t0 = Instant::now();
        let mut last = None;
        for i in 0..200u32 {
            last = Some(model.process(1.0, t0 + Duration::from_millis(10 * u64::from(i))));
        }
        let out = last.unwrap();
        assert!((out.gain - 7.93).abs() < 1e-9, "gain {}", out.gain);
        assert!((out.ema_ips - 12.96).abs() < 0.05);
    }

    #[test]
    fn ema_resets_after_gap() {
        let mut model = LoWheelModel::new(0.0);
        let t0 = Instant::now();
        for i in 0..100u32 {
            model.process(1.0, t0 + Duration::from_millis(10 * u64::from(i)));
        }
        assert!(model.ema_ips > 0.6);
        let out = model.process(1.0, t0 + Duration::from_millis(990 + 100));
        // Reset then one sample: 0.05 * (0.1296/0.1).
        assert!((out.ema_ips - 0.05 * 1.2963).abs() < 1e-3);
        assert!((out.gain - 1.0).abs() < 1e-12);
    }

    /// The tick-rate cliff documented on the module: at 10 notches per second
    /// or slower every gap is >= 100 ms, so the gain never leaves 1.
    #[test]
    fn ten_ticks_per_second_or_slower_stays_at_gain_one() {
        let mut model = LoWheelModel::new(1.0);
        let t0 = Instant::now();
        for i in 0..50u64 {
            let out = model.process(1.0, t0 + Duration::from_millis(100 * i));
            assert!((out.gain - 1.0).abs() < 1e-12, "tick {i}: {}", out.gain);
        }
    }

    #[test]
    fn remainder_carry_conserves_total() {
        let mut model = LoWheelModel::new(-1.0);
        let t0 = Instant::now();
        let mut total = 0i64;
        let mut exact = 0.0;
        for i in 0..50u32 {
            // Slow: 200 ms apart, 3/8 notch each, gain stays 1.
            let out = model.process(3.0 / 8.0, t0 + Duration::from_millis(200 * u64::from(i)));
            total += i64::from(out.pixels);
            exact += 3.0 / 8.0 / NOTCHES_PER_INCH * 100.0;
        }
        let carried = f64::from(i32::try_from(total).unwrap()) + model.pixel_remainder;
        assert!((carried - exact).abs() < 1e-9, "{carried} vs {exact}");
    }

    #[test]
    fn sign_follows_notches_and_truncates_toward_zero() {
        let mut model = LoWheelModel::new(0.0);
        let out = model.process(-1.0, Instant::now());
        assert_eq!(out.pixels, -12);
        assert!(model.pixel_remainder < 0.0);
        assert!(out.ema_ips > 0.0);
    }

    #[test]
    fn zero_and_non_finite_notches_leave_the_state_alone() {
        let mut model = LoWheelModel::new(0.0);
        let t0 = Instant::now();
        for bad in [0.0, f64::NAN, f64::INFINITY] {
            assert_eq!(model.process(bad, t0).pixels, 0);
        }
        assert!(model.last_event.is_none());
    }

    #[test]
    fn line_delta_accumulates_pixels_over_ten() {
        let mut model = LoWheelModel::new(0.0);
        let t0 = Instant::now();
        let a = model.process(1.0, t0);
        assert_eq!((a.pixels, a.line_delta), (12, 1.2));
        // The reversal loses the one step the hysteresis remembers (14 of 15
        // steps move), and the 0.96 px carried from the first report adds
        // back: -12.09 + 0.96 truncates to -11.
        let b = model.process(-1.0, t0 + Duration::from_millis(500));
        assert_eq!((b.pixels, b.line_delta), (-11, -1.1));
    }

    #[test]
    fn hysteresis_swallows_one_step_of_a_reversal_and_of_a_start_after_idle() {
        let mut h = StepHysteresis::default();
        let t0 = Instant::now();
        // A fresh wheel passes everything.
        assert_eq!(h.filter(3.0, t0), 3.0);
        // Reversing straight away loses the step the first move remembered.
        assert_eq!(h.filter(-3.0, t0 + Duration::from_millis(10)), -2.0);
        // That memory is spent: the next reverse report passes whole.
        assert_eq!(h.filter(-3.0, t0 + Duration::from_millis(20)), -3.0);
        // After an idle second both memories relax to one step, so the first
        // step of the next movement in either direction is swallowed.
        assert_eq!(h.filter(3.0, t0 + Duration::from_millis(2_000)), 2.0);
        assert_eq!(h.filter(0.0, t0 + Duration::from_millis(2_010)), 0.0);
    }

    #[test]
    fn a_report_swallowed_by_hysteresis_does_not_feed_the_speed_estimate() {
        let mut model = LoWheelModel::new(0.0);
        let t0 = Instant::now();
        model.process(0.2, t0);
        let ema = model.ema_ips;
        // One step back right after three forward: exactly the remembered step.
        let swallowed = model.process(-1.0 / 15.0, t0 + Duration::from_millis(8));
        assert_eq!(swallowed.pixels, 0);
        assert_eq!(model.last_event, Some(t0));
        assert_eq!(model.ema_ips, ema);
    }

    #[test]
    fn gain_is_monotonic_in_verified_regions() {
        let s = speed_scalar(0.0);
        let mut prev = curve_gain(0.0, s);
        let mut v = 0.0;
        while v < 59.9 {
            v += 0.05;
            let g = curve_gain(v, s);
            assert!(g >= prev - 1e-12, "rising region at {v}");
            prev = g;
        }
        v = 60.0;
        prev = curve_gain(v, s);
        while v < 119.99 {
            v += 0.05;
            let g = curve_gain(v, s);
            assert!(g <= prev + 1e-12, "falling region at {v}");
            prev = g;
        }
    }

    #[test]
    fn curve_segments_match_table() {
        let s = 0.5;
        assert!((curve_gain(0.29, s) - 1.0).abs() < 1e-12);
        assert!((curve_gain(0.45, s) - (1.0 + 5.5 / 2.0)).abs() < 1e-9);
        assert!((curve_gain(30.0, s) - 6.5).abs() < 1e-12);
        // Midpoint of the 60-120 ramp: halfway between 1+11s=6.5 and 1+5s=3.5.
        assert!((curve_gain(90.0, s) - 5.0).abs() < 1e-9);
    }

    /// Deliberate deviation from the reference, whose table steps to `1 - 2s`
    /// at 120 ips (zero or negative at the default speed): the gain holds the
    /// value the falling ramp ends on, `1 + 5s`, and is always positive.
    #[test]
    fn at_or_above_120_ips_gain_holds_the_end_of_the_falling_ramp() {
        for s in [SPEED_MIN, 0.5, 0.63, SPEED_MAX] {
            let held = 1.0 + 5.0 * s;
            assert!((curve_gain(119.999, s) - held).abs() < 0.01);
            assert!((curve_gain(120.0, s) - held).abs() < 1e-12);
            assert!((curve_gain(500.0, s) - held).abs() < 1e-12);
            assert!(curve_gain(1e6, s) > 0.0);
        }
    }

    /// A sustained hook-style burst (OS line deltas of several notches per
    /// event at the 8 ms floor) drives the EMA far past 120 ips. It must keep
    /// its direction and never emit an opposite-sign pixel.
    #[test]
    fn sustained_fast_burst_keeps_its_direction() {
        for (sign, speed) in [(1.0, 0.0), (-1.0, 0.0), (1.0, 1.0), (-1.0, 1.0)] {
            let mut model = LoWheelModel::new(speed);
            let t0 = Instant::now();
            let mut peak: f64 = 0.0;
            for i in 0..400u64 {
                let out = model.process(sign * 10.0, t0 + Duration::from_millis(8 * i));
                peak = peak.max(out.ema_ips);
                assert!(out.gain > 0.0, "gain {} at {i}", out.gain);
                assert!(
                    out.pixels == 0 || (out.pixels > 0) == (sign > 0.0),
                    "report {i} emitted {} for sign {sign}",
                    out.pixels
                );
            }
            assert!(peak >= 120.0, "burst never reached the held region: {peak}");
        }
    }

    /// Replays captured Options+ reports (see each CSV header) and compares
    /// with the pixels it actually posted. Rows with a blank pixel column had
    /// no paired event and are only fed to the model. Every slice starts at a
    /// gap of 100 ms or more, so the model starts from the same empty history.
    /// Rows of `lo_plus_reversal.csv` where Options+ posts 2 px for a 2-step
    /// report 8 ms after a 5-step one and the model posts 14: unexplained,
    /// possibly its ratchet logic, which is not modelled.
    const KNOWN_DIVERGENCES: [usize; 1] = [206];

    struct Replay {
        rows: usize,
        model_pixels: i64,
        capture_pixels: i64,
        model_lines: f64,
        capture_lines: i64,
        worst: i64,
        reversals: usize,
        mid_stream_gaps: usize,
    }

    fn replay(csv: &str, speed: f64, known_divergences: &[usize]) -> Replay {
        let mut model = LoWheelModel::new(speed);
        let mut at = Instant::now();
        let mut out = Replay {
            rows: 0,
            model_pixels: 0,
            capture_pixels: 0,
            model_lines: 0.0,
            capture_lines: 0,
            worst: 0,
            reversals: 0,
            mid_stream_gaps: 0,
        };
        let mut previous_steps = 0i64;
        for line in csv.lines().filter(|line| !line.starts_with('#')) {
            let mut fields = line.split(',');
            let mut next = || fields.next().unwrap();
            let gap_ns: u64 = next().parse().unwrap();
            let steps: i64 = next().parse().unwrap();
            let expected_pixels = next().parse::<i64>().ok();
            let expected_lines = next().parse::<i64>().ok();

            at += Duration::from_nanos(gap_ns);
            if out.rows > 0 && gap_ns >= 100_000_000 {
                out.mid_stream_gaps += 1;
            }
            if previous_steps * steps < 0 {
                out.reversals += 1;
            }
            previous_steps = steps;

            let result = model.process(notches(steps), at);
            if known_divergences.contains(&out.rows) {
                // Still fed to the model so its state follows the capture, but
                // kept out of the totals and asserted to really differ.
                let expected = expected_pixels.unwrap();
                assert!(
                    (i64::from(result.pixels) - expected).abs() > 1,
                    "row {} no longer diverges: drop it from the known divergences",
                    out.rows
                );
            } else if let Some(expected) = expected_pixels {
                out.model_lines += result.line_delta;
                out.capture_lines += expected_lines.unwrap_or(0);
                let pixels = i64::from(result.pixels);
                out.worst = out.worst.max((pixels - expected).abs());
                out.model_pixels += pixels;
                out.capture_pixels += expected;
                assert!(
                    (pixels - expected).abs() <= 1,
                    "row {}: model {pixels} vs Options+ {expected}",
                    out.rows
                );
            }
            out.rows += 1;
        }
        out
    }

    /// Totals within 0.5% of pixels, 10% of lines (the line flush cadence is
    /// simplified), with matching signs.
    fn assert_totals(replay: &Replay) {
        assert!(
            (replay.model_pixels - replay.capture_pixels).abs() * 200 < replay.capture_pixels.abs(),
            "pixels {} vs {}",
            replay.model_pixels,
            replay.capture_pixels
        );
        #[expect(clippy::cast_precision_loss, reason = "small test totals")]
        let capture_lines = replay.capture_lines as f64;
        assert!(
            (replay.model_lines - capture_lines).abs() < capture_lines.abs() * 0.1,
            "lines {} vs {capture_lines}",
            replay.model_lines
        );
    }

    #[test]
    fn replays_captured_options_plus_events() {
        let replay = replay(include_str!("fixtures/lo_plus_replay.csv"), 0.0, &[]);
        assert_eq!(replay.rows, 200);
        assert_eq!((replay.reversals, replay.mid_stream_gaps), (0, 0));
        assert_totals(&replay);
    }

    /// One burst that scrolls down, reverses direction mid-stream and then
    /// pauses (>= 100 ms, EMA reset) three times.
    #[test]
    fn replays_a_reversal_and_mid_stream_gaps() {
        let replay = replay(
            include_str!("fixtures/lo_plus_reversal.csv"),
            0.0,
            &KNOWN_DIVERGENCES,
        );
        assert_eq!(replay.rows, 229);
        assert_eq!(replay.reversals, 1);
        assert_eq!(replay.mid_stream_gaps, 3);
        assert_totals(&replay);
    }

    /// A reversal across a gap, followed by further gaps.
    #[test]
    fn replays_a_reversal_across_a_gap() {
        let replay = replay(
            include_str!("fixtures/lo_plus_reversal_after_gap.csv"),
            0.0,
            &[],
        );
        assert_eq!(replay.rows, 60);
        assert!(replay.mid_stream_gaps >= 3);
        assert_totals(&replay);
    }
}
