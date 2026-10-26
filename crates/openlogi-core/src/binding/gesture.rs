//! Gesture-button direction slots (swipe + click).

use std::fmt;

use serde::{Deserialize, Serialize};

/// One of the five sub-bindings on the gesture button: hold + swipe up/down/
/// left/right or a plain click without movement. Logi ships these as
/// independent assignments (`SLOT_NAME_GESTURE_*_BUTTON` in the
/// `device_gesture_buttons_image` metadata block) — OpenLogi mirrors the
/// same shape.
///
/// Variant identifiers are TOML-stable: renames are migration events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GestureDirection {
    /// Hold + swipe up (negative raw-XY `dy`).
    Up,
    /// Hold + swipe down (positive raw-XY `dy`).
    Down,
    /// Hold + swipe left (negative raw-XY `dx`).
    Left,
    /// Hold + swipe right (positive raw-XY `dx`).
    Right,
    /// A press-and-release that never committed a swipe — the gesture
    /// button's plain-click slot.
    Click,
    /// A double click without swipe.
    DoubleClick,
    /// A press held in place without swiping.
    Hold,
}

impl GestureDirection {
    /// All direction and click slots, swipes first, then click, double-click, and hold.
    /// Iterated to seed or complete a full gesture map — see
    /// [`Binding::fill_gesture_defaults`](crate::binding::Binding::fill_gesture_defaults)
    /// and [`default_binding_for`](crate::binding::default_binding_for).
    pub const ALL: [GestureDirection; 7] = [
        GestureDirection::Up,
        GestureDirection::Down,
        GestureDirection::Left,
        GestureDirection::Right,
        GestureDirection::Click,
        GestureDirection::DoubleClick,
        GestureDirection::Hold,
    ];

    /// Human-readable label for popovers and tooltips.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            GestureDirection::Up => "Up",
            GestureDirection::Down => "Down",
            GestureDirection::Left => "Left",
            GestureDirection::Right => "Right",
            GestureDirection::Click => "Click",
            GestureDirection::DoubleClick => "Double Click",
            GestureDirection::Hold => "Click & Hold",
        }
    }

    /// Stable catalog key for the localized direction label.
    #[must_use]
    pub fn translation_key(self) -> &'static str {
        match self {
            GestureDirection::Up => "common.up",
            GestureDirection::Down => "common.down",
            GestureDirection::Left => "common.left",
            GestureDirection::Right => "common.right",
            GestureDirection::Click => "common.click",
            GestureDirection::DoubleClick => "common.double_click",
            GestureDirection::Hold => "common.hold",
        }
    }
}

impl fmt::Display for GestureDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GestureDirection::Up => f.write_str("Up"),
            GestureDirection::Down => f.write_str("Down"),
            GestureDirection::Left => f.write_str("Left"),
            GestureDirection::Right => f.write_str("Right"),
            GestureDirection::Click => f.write_str("Click"),
            GestureDirection::DoubleClick => f.write_str("DoubleClick"),
            GestureDirection::Hold => f.write_str("Hold"),
        }
    }
}
