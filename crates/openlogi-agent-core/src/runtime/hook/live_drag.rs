//! Continuous 1:1 live drag tracking for desktop Spaces navigation.
//!
//! When a gesture button is held and dragged horizontally on macOS, this tracker
//! streams real-time normalized progress to the WindowServer DockSwipe API,
//! allowing desktop Spaces to slide smoothly in 1:1 sync with the physical mouse,
//! matching the Apple Trackpad gesture experience.

use std::time::Instant;
use openlogi_core::binding::GESTURE_HOLD_FOR_SWIPE;

pub use openlogi_inject::LiveDragPhase;

/// Continuous 1:1 live drag state machine.
#[derive(Debug, Default)]
pub struct LiveSpaceDrag {
    active: bool,
    held_since: Option<Instant>,
    accumulated_x: f64,
    accumulated_y: f64,
}

/// Drag travel distance (in pixels) required to break deadband and activate live drag.
const DRAG_DEADBAND: f64 = 15.0;

/// Mouse travel distance (in pixels) that corresponds to 1.0 (100%) Space travel.
/// 400px provides natural, comfortable hand movement on typical mouse DPI settings.
const FULL_TRAVEL_DISTANCE: f64 = 400.0;

/// Minimum progress required on release to complete the switch rather than spring back.
const SNAP_THRESHOLD: f64 = 0.35;

impl LiveSpaceDrag {
    /// Reset tracker state upon a new hold.
    pub fn reset(&mut self) {
        self.active = false;
        self.held_since = Some(Instant::now());
        self.accumulated_x = 0.0;
        self.accumulated_y = 0.0;
    }

    /// Whether live drag is actively streaming frames.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Feed delta movement into the tracker.
    ///
    /// Returns `Some((progress, phase))` if a live drag frame should be posted.
    pub fn update(&mut self, dx: f64, dy: f64) -> Option<(f64, LiveDragPhase)> {
        self.accumulated_x += dx;
        self.accumulated_y += dy;

        if !self.active {
            let held_long_enough = self
                .held_since
                .is_some_and(|t| t.elapsed() >= GESTURE_HOLD_FOR_SWIPE);
            let abs_x = self.accumulated_x.abs();
            let abs_y = self.accumulated_y.abs();
            if held_long_enough && abs_x >= DRAG_DEADBAND && abs_x > abs_y {
                self.active = true;
                let progress = self.current_progress();
                return Some((progress, LiveDragPhase::Began));
            }
            return None;
        }

        let progress = self.current_progress();
        Some((progress, LiveDragPhase::Changed))
    }

    /// Finish the drag on button release.
    ///
    /// Returns `Some((progress, LiveDragPhase::Ended))` if live drag was active.
    /// If progress exceeds [`SNAP_THRESHOLD`], the signed progress is passed
    /// so the Dock switches to the target desktop. Otherwise, 0.0 is passed
    /// so the Dock springs back to the origin space.
    pub fn finish(&mut self) -> Option<(f64, LiveDragPhase)> {
        if !self.active {
            self.reset();
            return None;
        }

        let raw_progress = self.current_progress();
        self.active = false;
        self.held_since = None;
        self.accumulated_x = 0.0;
        self.accumulated_y = 0.0;

        let end_progress = if raw_progress.abs() >= SNAP_THRESHOLD {
            raw_progress
        } else {
            0.0
        };

        Some((end_progress, LiveDragPhase::Ended))
    }

    fn current_progress(&self) -> f64 {
        (self.accumulated_x / FULL_TRAVEL_DISTANCE).clamp(-1.0, 1.0)
    }

    /// Test-only seam: backdate the hold duration past [`GESTURE_HOLD_FOR_SWIPE`].
    #[cfg(test)]
    #[doc(hidden)]
    pub fn backdate_hold_for_test(&mut self) {
        if self.held_since.is_some() {
            self.held_since = Instant::now().checked_sub(GESTURE_HOLD_FOR_SWIPE * 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_click_with_motion_does_not_activate_before_hold_threshold() {
        let mut drag = LiveSpaceDrag::default();
        drag.reset();
        // User clicked and released within 50ms, moving 25px
        assert_eq!(drag.update(25.0, 0.0), None);
        assert!(!drag.is_active());
        assert_eq!(drag.finish(), None);
    }

    #[test]
    fn small_movements_within_deadband_do_not_activate() {
        let mut drag = LiveSpaceDrag::default();
        drag.reset();
        drag.backdate_hold_for_test();
        assert_eq!(drag.update(5.0, 2.0), None);
        assert_eq!(drag.update(4.0, -1.0), None);
        assert!(!drag.is_active());
        assert_eq!(drag.finish(), None);
    }

    #[test]
    fn vertical_dominant_motion_does_not_activate_space_drag() {
        let mut drag = LiveSpaceDrag::default();
        drag.reset();
        drag.backdate_hold_for_test();
        assert_eq!(drag.update(5.0, 25.0), None);
        assert!(!drag.is_active());
        assert_eq!(drag.finish(), None);
    }

    #[test]
    fn horizontal_motion_beyond_deadband_begins_and_changes() {
        let mut drag = LiveSpaceDrag::default();
        drag.reset();
        drag.backdate_hold_for_test();
        let first = drag.update(20.0, 0.0);
        assert_eq!(first, Some((0.05, LiveDragPhase::Began)));
        assert!(drag.is_active());

        let second = drag.update(80.0, 0.0);
        assert_eq!(second, Some((0.25, LiveDragPhase::Changed)));

        let third = drag.update(100.0, 0.0);
        assert_eq!(third, Some((0.5, LiveDragPhase::Changed)));

        // Release past SNAP_THRESHOLD (0.5 >= 0.35)
        let ended = drag.finish();
        assert_eq!(ended, Some((0.5, LiveDragPhase::Ended)));
        assert!(!drag.is_active());
    }

    #[test]
    fn drag_retracted_below_snap_threshold_springs_back() {
        let mut drag = LiveSpaceDrag::default();
        drag.reset();
        drag.backdate_hold_for_test();
        let _ = drag.update(40.0, 0.0);
        assert!(drag.is_active());

        // Drag back toward origin
        let _ = drag.update(-30.0, 0.0);
        let progress = drag.current_progress();
        assert!(progress < SNAP_THRESHOLD);

        // Released at low travel -> springs back (0.0)
        let ended = drag.finish();
        assert_eq!(ended, Some((0.0, LiveDragPhase::Ended)));
    }
}
