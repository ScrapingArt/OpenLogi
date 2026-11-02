//! Actions Ring configuration vocabulary.
//!
//! The ring is host-side UI: a trigger opens an eight-position layout and the
//! agent executes the selected action. The types live beside [`Action`] because
//! they are persisted directly in `config.toml` and shared by the agent and GUI.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use nutype::nutype;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::Action;
use crate::app::{AppAliases, overlay_for, profile_key_for};

mod icon;

pub use icon::ActionRingIcon;

/// One of the eight fixed positions in an Actions Ring, clockwise from the top.
///
/// Variant names are part of the TOML schema and must remain stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActionRingSlot {
    /// Twelve o'clock.
    Top,
    /// Between top and right.
    TopRight,
    /// Three o'clock.
    Right,
    /// Between right and bottom.
    BottomRight,
    /// Six o'clock.
    Bottom,
    /// Between bottom and left.
    BottomLeft,
    /// Nine o'clock.
    Left,
    /// Between left and top.
    TopLeft,
}

impl ActionRingSlot {
    /// All ring positions in clockwise display order.
    pub const ALL: [Self; 8] = [
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
        Self::TopLeft,
    ];

    /// Unit vector from the ring's centre to this slot, with positive Y
    /// pointing **down** — the screen convention both GPUI frontends draw in,
    /// not the mathematical one.
    #[must_use]
    pub fn unit_offset(self) -> (f32, f32) {
        let diagonal = std::f32::consts::FRAC_1_SQRT_2;
        match self {
            Self::Top => (0.0, -1.0),
            Self::TopRight => (diagonal, -diagonal),
            Self::Right => (1.0, 0.0),
            Self::BottomRight => (diagonal, diagonal),
            Self::Bottom => (0.0, 1.0),
            Self::BottomLeft => (-diagonal, diagonal),
            Self::Left => (-1.0, 0.0),
            Self::TopLeft => (-diagonal, -diagonal),
        }
    }

    /// Top-left corner at which to place this slot's `slot_size` box, on a
    /// square `canvas` whose ring has the given `radius`. All in the caller's
    /// own units; the live overlay and the settings preview differ only in
    /// what they pass.
    #[must_use]
    pub fn placement(self, canvas: f32, radius: f32, slot_size: f32) -> (f32, f32) {
        let (x, y) = self.unit_offset();
        (
            canvas / 2.0 + x * radius - slot_size / 2.0,
            canvas / 2.0 + y * radius - slot_size / 2.0,
        )
    }

    /// Stable display index matching [`Self::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Top => 0,
            Self::TopRight => 1,
            Self::Right => 2,
            Self::BottomRight => 3,
            Self::Bottom => 4,
            Self::BottomLeft => 5,
            Self::Left => 6,
            Self::TopLeft => 7,
        }
    }
}

/// Why an [`Action`] cannot be placed in an Actions Ring slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RingActionError {
    /// Empty slots are represented by an absent map entry, not `Action::None`.
    #[error("Do Nothing is represented by an empty Actions Ring slot")]
    EmptyAction,
    /// A ring cannot recursively open itself.
    #[error("Show Actions Ring cannot be assigned inside an Actions Ring")]
    RecursiveTrigger,
}

/// An action that is valid inside an Actions Ring.
///
/// Construction and deserialization reject actions that would make the ring's
/// state ambiguous (`None`) or recursively invoke another ring
/// (`ShowActionsRing`).
#[nutype(
    validate(with = validate_ring_action, error = RingActionError),
    derive(Clone, Debug, PartialEq, Eq, Hash, AsRef, TryFrom, Into, Serialize, Deserialize),
)]
pub struct RingAction(Action);

impl RingAction {
    /// Validate and wrap an ordinary action for placement in a ring.
    pub fn new(action: Action) -> Result<Self, RingActionError> {
        Self::try_new(action)
    }

    /// The action the agent should execute when this slot is activated.
    #[must_use]
    pub fn action(&self) -> &Action {
        self.as_ref()
    }

    /// Consume the wrapper and return its action.
    #[must_use]
    pub fn into_action(self) -> Action {
        self.into_inner()
    }
}

fn validate_ring_action(action: &Action) -> Result<(), RingActionError> {
    match action {
        Action::None => Err(RingActionError::EmptyAction),
        Action::ShowActionsRing(_) => Err(RingActionError::RecursiveTrigger),
        _ => Ok(()),
    }
}

/// One populated Actions Ring slot.
///
/// Keeping the action and optional presentation icon in one value makes an
/// orphan icon impossible: clearing a slot removes the complete entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRingEntry {
    action: RingAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<ActionRingIcon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

impl ActionRingEntry {
    /// Create a slot with its action-derived icon and label.
    #[must_use]
    pub const fn new(action: RingAction) -> Self {
        Self {
            action,
            icon: None,
            label: None,
        }
    }

    /// Executable action for this slot.
    #[must_use]
    pub fn action(&self) -> &Action {
        self.action.action()
    }

    /// User-selected icon, or `None` to derive it from [`Self::action`].
    #[must_use]
    pub const fn custom_icon(&self) -> Option<ActionRingIcon> {
        self.icon
    }

    /// User-provided display label, or `None` to derive one from
    /// [`Self::action`]. Free text: the overlay's localization pass returns
    /// unknown keys verbatim, so user labels render as written.
    #[must_use]
    pub fn custom_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Consume the entry into its executable action and presentation
    /// overrides (icon, label).
    #[must_use]
    pub fn into_parts(self) -> (Action, Option<ActionRingIcon>, Option<String>) {
        (self.action.into_action(), self.icon, self.label)
    }

    fn replace_action(&mut self, action: RingAction) {
        self.action = action;
    }

    fn set_icon(&mut self, icon: Option<ActionRingIcon>) {
        self.icon = icon;
    }

    fn set_label(&mut self, label: Option<String>) {
        self.label = label;
    }
}

/// The actions displayed at the eight fixed ring positions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRingLayout {
    /// Populated ring positions. An absent key is an intentionally empty slot.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slots: BTreeMap<ActionRingSlot, ActionRingEntry>,
}

impl ActionRingLayout {
    /// Replace or clear a slot while preserving its custom icon when replaced.
    pub fn set_action(&mut self, slot: ActionRingSlot, action: Option<RingAction>) {
        match (self.slots.entry(slot), action) {
            (Entry::Occupied(mut entry), Some(action)) => entry.get_mut().replace_action(action),
            (Entry::Vacant(entry), Some(action)) => {
                entry.insert(ActionRingEntry::new(action));
            }
            (Entry::Occupied(entry), None) => {
                entry.remove();
            }
            (Entry::Vacant(_), None) => {}
        }
    }

    /// Set a custom icon for a populated slot. Empty slots remain empty.
    pub fn set_icon(&mut self, slot: ActionRingSlot, icon: Option<ActionRingIcon>) {
        if let Some(entry) = self.slots.get_mut(&slot) {
            entry.set_icon(icon);
        }
    }

    /// Set a custom label for a populated slot. Empty slots remain empty.
    pub fn set_label(&mut self, slot: ActionRingSlot, label: Option<String>) {
        if let Some(entry) = self.slots.get_mut(&slot) {
            entry.set_label(label);
        }
    }
}

impl Default for ActionRingLayout {
    #[expect(
        clippy::expect_used,
        reason = "the built-in ring actions are statically known to satisfy RingAction's invariant"
    )]
    fn default() -> Self {
        use ActionRingSlot as Slot;

        let actions = [
            (Slot::Top, Action::Cut),
            (Slot::TopRight, Action::Copy),
            (Slot::Right, Action::Paste),
            (Slot::BottomRight, Action::BrowserForward),
            (Slot::Bottom, Action::PlayPause),
            (Slot::BottomLeft, Action::BrowserBack),
            (Slot::Left, Action::Undo),
            (Slot::TopLeft, Action::Redo),
        ];
        let slots = actions
            .into_iter()
            .map(|(slot, action)| {
                (
                    slot,
                    ActionRingEntry::new(
                        RingAction::new(action).expect("default ring actions must be valid"),
                    ),
                )
            })
            .collect();
        Self { slots }
    }
}

/// The reserved name for the built-in primary Actions Ring setup.
pub const DEFAULT_SETUP_NAME: &str = "Default";

/// A named Actions Ring setup, containing a default layout and optional per-application layout overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRingSetup {
    /// Default layout used when the foreground application has no override.
    #[serde(default)]
    pub default: ActionRingLayout,
    /// Complete layout overrides keyed by foreground application identifier.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_app: BTreeMap<String, ActionRingLayout>,
}

impl ActionRingSetup {
    /// Resolve the complete layout for the foreground application in this setup.
    #[must_use]
    pub fn effective_layout(&self, aliases: &AppAliases, app_id: Option<&str>) -> ActionRingLayout {
        app_id
            .and_then(|app| overlay_for(&self.per_app, aliases, app))
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }

    /// The layout saved under `key` in this setup, or the setup's default layout.
    #[must_use]
    pub fn profile_layout(&self, key: Option<&str>) -> ActionRingLayout {
        key.and_then(|k| self.per_app.get(k))
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }
}

/// Why an Actions Ring setup operation failed.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ActionRingSetupError {
    /// The default setup cannot be renamed or deleted.
    #[error("cannot rename or delete the Default setup")]
    DefaultSetupReserved,
    /// A setup name cannot be empty or whitespace only.
    #[error("setup name cannot be empty")]
    EmptyName,
    /// A setup with this name already exists on the device.
    #[error("a setup named '{0}' already exists")]
    AlreadyExists(String),
    /// The specified setup was not found on the device.
    #[error("setup '{0}' not found")]
    NotFound(String),
}

/// Per-device Actions Ring settings and application-specific layouts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRingConfig {
    /// Whether `ShowActionsRing` opens this device's ring.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether ring hover and activation transitions play device haptics.
    #[serde(default = "default_true")]
    pub haptics: bool,
    /// Layout used when the foreground application has no override.
    #[serde(default)]
    pub default: ActionRingLayout,
    /// Complete layout overrides keyed by foreground application identifier.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_app: BTreeMap<String, ActionRingLayout>,
    /// Additional named setups for this device.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub setups: BTreeMap<String, ActionRingSetup>,
}

impl Default for ActionRingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            haptics: true,
            default: ActionRingLayout::default(),
            per_app: BTreeMap::new(),
            setups: BTreeMap::new(),
        }
    }
}

impl ActionRingConfig {
    /// Whether this value is exactly the implicit default and can be omitted
    /// from `config.toml`.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Resolve the complete layout for the foreground application.
    ///
    /// `per_app` takes the same selectors as `per_app_bindings` and is matched
    /// the same way ([`overlay_for`]): the exact key, then on Windows the
    /// `exe:<filename>` fallback a versioned install path needs, then the
    /// `app:<alias>` key of the alias in `aliases` that lists the application.
    #[must_use]
    pub fn effective_layout(&self, aliases: &AppAliases, app_id: Option<&str>) -> ActionRingLayout {
        app_id
            .and_then(|app| overlay_for(&self.per_app, aliases, app))
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }

    /// The key of the `per_app` layout [`Self::effective_layout`] resolves
    /// `app` to, or `None` when the default layout applies.
    #[must_use]
    pub fn profile_for<'a>(&'a self, aliases: &AppAliases, app: &str) -> Option<&'a str> {
        profile_key_for(&self.per_app, aliases, app)
    }

    /// The complete layout saved under the profile key `key`, or the default
    /// layout when `key` is `None` or has none: what an editor that addresses
    /// `key` shows. Unlike [`Self::effective_layout`], no other key's layout
    /// stands in, because the first edit creates `key`'s layout from the
    /// default, and the editor must show the layout that edit changes.
    #[must_use]
    pub fn profile_layout(&self, key: Option<&str>) -> ActionRingLayout {
        key.and_then(|key| self.per_app.get(key))
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }

    /// All available setup names, starting with [`DEFAULT_SETUP_NAME`] followed by custom setups in sorted order.
    #[must_use]
    pub fn setup_names(&self) -> Vec<&str> {
        let mut names = Vec::with_capacity(1 + self.setups.len());
        names.push(DEFAULT_SETUP_NAME);
        names.extend(self.setups.keys().map(String::as_str));
        names
    }

    /// Access a custom setup by name, or `None` if `name` is `None`, `"Default"`, or not found.
    #[must_use]
    pub fn setup(&self, name: Option<&str>) -> Option<&ActionRingSetup> {
        match name {
            Some(name) if name != DEFAULT_SETUP_NAME => self.setups.get(name),
            _ => None,
        }
    }

    /// Mutably access a custom setup by name, or `None` if `name` is `None`, `"Default"`, or not found.
    pub fn setup_mut(&mut self, name: Option<&str>) -> Option<&mut ActionRingSetup> {
        match name {
            Some(name) if name != DEFAULT_SETUP_NAME => self.setups.get_mut(name),
            _ => None,
        }
    }

    /// Resolve the complete layout for a specific setup and foreground application.
    /// Falls back to the default setup layout if `setup` is `None`, `"Default"`, or nonexistent.
    #[must_use]
    pub fn effective_layout_for(
        &self,
        setup: Option<&str>,
        aliases: &AppAliases,
        app_id: Option<&str>,
    ) -> ActionRingLayout {
        if let Some(custom) = self.setup(setup) {
            custom.effective_layout(aliases, app_id)
        } else {
            self.effective_layout(aliases, app_id)
        }
    }

    /// Resolve the profile layout for a specific setup and profile key.
    /// Falls back to the default setup layout if `setup` is `None`, `"Default"`, or nonexistent.
    #[must_use]
    pub fn profile_layout_for(&self, setup: Option<&str>, key: Option<&str>) -> ActionRingLayout {
        if let Some(custom) = self.setup(setup) {
            custom.profile_layout(key)
        } else {
            self.profile_layout(key)
        }
    }

    /// Create a new custom setup with default slots and no app overrides.
    pub fn create_setup(&mut self, name: &str) -> Result<(), ActionRingSetupError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ActionRingSetupError::EmptyName);
        }
        if trimmed == DEFAULT_SETUP_NAME {
            return Err(ActionRingSetupError::DefaultSetupReserved);
        }
        if self.setups.contains_key(trimmed) {
            return Err(ActionRingSetupError::AlreadyExists(trimmed.to_string()));
        }
        self.setups.insert(trimmed.to_string(), ActionRingSetup::default());
        Ok(())
    }

    /// Rename an existing custom setup.
    pub fn rename_setup(&mut self, old: &str, new: &str) -> Result<(), ActionRingSetupError> {
        if old == DEFAULT_SETUP_NAME {
            return Err(ActionRingSetupError::DefaultSetupReserved);
        }
        let trimmed_new = new.trim();
        if trimmed_new.is_empty() {
            return Err(ActionRingSetupError::EmptyName);
        }
        if trimmed_new == DEFAULT_SETUP_NAME {
            return Err(ActionRingSetupError::DefaultSetupReserved);
        }
        if self.setups.contains_key(trimmed_new) {
            return Err(ActionRingSetupError::AlreadyExists(trimmed_new.to_string()));
        }
        let setup = self
            .setups
            .remove(old)
            .ok_or_else(|| ActionRingSetupError::NotFound(old.to_string()))?;
        self.setups.insert(trimmed_new.to_string(), setup);
        Ok(())
    }

    /// Duplicate an existing setup (either default or a custom setup) to a new setup name.
    pub fn duplicate_setup(
        &mut self,
        source: Option<&str>,
        target: &str,
    ) -> Result<(), ActionRingSetupError> {
        let trimmed_target = target.trim();
        if trimmed_target.is_empty() {
            return Err(ActionRingSetupError::EmptyName);
        }
        if trimmed_target == DEFAULT_SETUP_NAME {
            return Err(ActionRingSetupError::DefaultSetupReserved);
        }
        if self.setups.contains_key(trimmed_target) {
            return Err(ActionRingSetupError::AlreadyExists(trimmed_target.to_string()));
        }

        let cloned = match source {
            None => ActionRingSetup {
                default: self.default.clone(),
                per_app: self.per_app.clone(),
            },
            Some(name) if name == DEFAULT_SETUP_NAME => ActionRingSetup {
                default: self.default.clone(),
                per_app: self.per_app.clone(),
            },
            Some(name) => {
                let src = self
                    .setups
                    .get(name)
                    .ok_or_else(|| ActionRingSetupError::NotFound(name.to_string()))?;
                src.clone()
            }
        };

        self.setups.insert(trimmed_target.to_string(), cloned);
        Ok(())
    }

    /// Delete an existing custom setup.
    pub fn delete_setup(&mut self, name: &str) -> Result<(), ActionRingSetupError> {
        if name == DEFAULT_SETUP_NAME {
            return Err(ActionRingSetupError::DefaultSetupReserved);
        }
        self.setups
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| ActionRingSetupError::NotFound(name.to_string()))
    }
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_populates_every_position() {
        let layout = ActionRingLayout::default();
        assert_eq!(layout.slots.len(), ActionRingSlot::ALL.len());
        assert!(
            ActionRingSlot::ALL
                .iter()
                .all(|slot| layout.slots.contains_key(slot))
        );
    }

    #[test]
    fn invalid_ring_actions_are_rejected() {
        assert_eq!(
            RingAction::new(Action::None),
            Err(RingActionError::EmptyAction)
        );
        assert_eq!(
            RingAction::new(Action::ShowActionsRing(None)),
            Err(RingActionError::RecursiveTrigger)
        );
        assert_eq!(
            RingAction::new(Action::ShowActionsRing(Some("Gaming".into()))),
            Err(RingActionError::RecursiveTrigger)
        );
    }

    #[test]
    fn ring_action_serializes_like_the_wrapped_action() {
        #[derive(Serialize)]
        struct Wrapper {
            action: RingAction,
        }

        let action = RingAction::new(Action::Copy).expect("copy must be a valid ring action");
        let encoded =
            toml::to_string(&Wrapper { action }).expect("could not serialize ring action");
        assert_eq!(encoded, "action = \"Copy\"\n");
    }

    #[test]
    fn custom_labels_roundtrip_and_survive_action_replacement() {
        let mut layout: ActionRingLayout = toml::from_str(
            r#"
            [slots]
            Top = { action = "Copy", label = "Copy Invoice" }
            "#,
        )
        .expect("could not deserialize labelled layout");
        assert_eq!(
            layout.slots[&ActionRingSlot::Top].custom_label(),
            Some("Copy Invoice")
        );

        let encoded = toml::to_string(&layout).expect("could not serialize labelled layout");
        let decoded = toml::from_str::<ActionRingLayout>(&encoded)
            .expect("could not deserialize labelled layout");
        assert_eq!(decoded, layout);

        // Like icons, a label sticks to its slot when the action is replaced.
        layout.set_action(
            ActionRingSlot::Top,
            Some(RingAction::new(Action::Paste).expect("paste must be a valid ring action")),
        );
        assert_eq!(
            layout.slots[&ActionRingSlot::Top].custom_label(),
            Some("Copy Invoice")
        );
    }

    #[test]
    fn unlabelled_entries_serialize_without_a_label_key() {
        let layout = ActionRingLayout::default();
        let encoded = toml::to_string(&layout).expect("could not serialize ring layout");
        assert!(!encoded.contains("label"));
    }

    #[test]
    fn clearing_a_slot_cannot_leave_an_orphan_icon() {
        let mut layout = ActionRingLayout::default();
        layout.set_icon(ActionRingSlot::Top, Some(ActionRingIcon::Keyboard));
        layout.set_action(ActionRingSlot::Top, None);
        assert!(!layout.slots.contains_key(&ActionRingSlot::Top));
    }

    #[test]
    fn custom_icons_roundtrip_without_changing_slot_actions() {
        let mut layout = ActionRingLayout::default();
        layout.set_icon(ActionRingSlot::Top, Some(ActionRingIcon::Keyboard));
        let encoded = toml::to_string(&layout).expect("could not serialize ring layout");
        let decoded = toml::from_str::<ActionRingLayout>(&encoded)
            .expect("could not deserialize ring layout");
        assert_eq!(decoded, layout);
        assert_eq!(decoded.slots[&ActionRingSlot::Top].action(), &Action::Cut);
        assert_eq!(
            decoded.slots[&ActionRingSlot::Top].custom_icon(),
            Some(ActionRingIcon::Keyboard)
        );
    }

    #[test]
    fn documented_inline_slots_deserialize() {
        let layout = toml::from_str::<ActionRingLayout>(
            r#"
[slots]
Top = { action = "Copy", icon = "Keyboard" }
Bottom = { action = { CustomShortcut = "Cmd+Shift+P" } }
"#,
        )
        .expect("documented ring layout failed");
        assert_eq!(layout.slots[&ActionRingSlot::Top].action(), &Action::Copy);
        assert_eq!(
            layout.slots[&ActionRingSlot::Top].custom_icon(),
            Some(ActionRingIcon::Keyboard)
        );
        assert!(matches!(
            layout.slots[&ActionRingSlot::Bottom].action(),
            Action::CustomShortcut(_)
        ));
    }

    #[test]
    fn recursive_action_fails_deserialization() {
        // A bare `"ShowActionsRing"` is not a TOML document, so the slot has to
        // be deserialized the way config.toml stores it — otherwise the parse
        // fails on syntax and never reaches the recursion guard.
        let error = match toml::from_str::<ActionRingEntry>("action = \"ShowActionsRing\"") {
            Ok(entry) => panic!("a ring slot must not recursively open the ring, got {entry:?}"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains(&RingActionError::RecursiveTrigger.to_string()),
            "expected the recursion guard to reject the slot, got: {error}"
        );
    }

    #[test]
    fn app_layout_replaces_the_default_layout() {
        let mut config = ActionRingConfig::default();
        let safari = ActionRingLayout {
            slots: BTreeMap::from([(
                ActionRingSlot::Top,
                ActionRingEntry::new(
                    RingAction::new(Action::NewTab).expect("new tab must be a valid ring action"),
                ),
            )]),
        };
        config
            .per_app
            .insert("com.apple.Safari".to_string(), safari.clone());

        let aliases = AppAliases::default();
        assert_eq!(
            config.effective_layout(&aliases, Some("com.apple.Safari")),
            safari
        );
        assert_eq!(
            config.effective_layout(&aliases, Some("other")),
            config.default
        );
        assert_eq!(
            config.profile_for(&aliases, "com.apple.Safari"),
            Some("com.apple.Safari")
        );
        assert_eq!(config.profile_for(&aliases, "other"), None);
    }

    #[test]
    fn a_ring_layout_keyed_by_executable_survives_a_versioned_install_path() {
        let mut config = ActionRingConfig::default();
        let sharex = ActionRingLayout {
            slots: BTreeMap::from([(
                ActionRingSlot::Top,
                ActionRingEntry::new(
                    RingAction::new(Action::Copy).expect("copy must be a valid ring action"),
                ),
            )]),
        };
        config
            .per_app
            .insert("exe:sharex.exe".to_string(), sharex.clone());

        // The same selector `per_app_bindings` honours: a Store app's path
        // changes with every update, its executable name does not.
        let aliases = AppAliases::default();
        assert_eq!(
            config.effective_layout(
                &aliases,
                Some(r"c:\program files\windowsapps\sharex_17.1_x64\sharex.exe")
            ),
            sharex
        );
        assert_eq!(
            config.effective_layout(&aliases, Some("com.getsharex.exe")),
            config.default,
            "a bundle id is never reinterpreted as an executable"
        );
    }

    #[test]
    fn an_editor_sees_only_the_layout_saved_under_its_key() {
        #[derive(Deserialize)]
        struct Apps {
            apps: AppAliases,
        }
        let Apps { apps } = toml::from_str(concat!(
            "[apps.sharex]\n",
            "macos = \"sharex-id\"\nlinux = \"sharex-id\"\nwindows = \"sharex-id\"\n",
        ))
        .expect("alias table");
        let top = |action| ActionRingLayout {
            slots: BTreeMap::from([(
                ActionRingSlot::Top,
                ActionRingEntry::new(RingAction::new(action).expect("a valid ring action")),
            )]),
        };
        let (copy, paste) = (top(Action::Copy), top(Action::Paste));
        let mut config = ActionRingConfig::default();
        config
            .per_app
            .insert("exe:sharex.exe".to_string(), copy.clone());
        config
            .per_app
            .insert("app:sharex".to_string(), paste.clone());
        let path = r"c:\tools\sharex.exe";

        // The application in front takes another key's layout when its own
        // key has none ...
        assert_eq!(config.effective_layout(&apps, Some(path)), copy);
        assert_eq!(config.effective_layout(&apps, Some("sharex-id")), paste);
        // ... but an editor shows what its first edit would start from.
        assert_eq!(config.profile_layout(Some(path)), config.default);
        assert_eq!(config.profile_layout(Some("sharex-id")), config.default);
        assert_eq!(config.profile_layout(Some("exe:sharex.exe")), copy);
        assert_eq!(config.profile_layout(Some("app:sharex")), paste);
        assert_eq!(config.profile_layout(None), config.default);
    }

    #[test]
    fn custom_setups_crud_and_default_reserved() {
        let mut config = ActionRingConfig::default();
        assert_eq!(config.setup_names(), vec!["Default"]);
        assert_eq!(config.setup(None), None);
        assert_eq!(config.setup(Some("Default")), None);

        // Creating a new setup
        config.create_setup("Gaming").expect("create Gaming setup");
        assert_eq!(config.setup_names(), vec!["Default", "Gaming"]);
        assert!(config.setup(Some("Gaming")).is_some());

        // Default name is reserved
        assert_eq!(
            config.create_setup("Default"),
            Err(ActionRingSetupError::DefaultSetupReserved)
        );
        // Duplicate name fails
        assert_eq!(
            config.create_setup("Gaming"),
            Err(ActionRingSetupError::AlreadyExists("Gaming".into()))
        );
        // Empty name fails
        assert_eq!(
            config.create_setup("   "),
            Err(ActionRingSetupError::EmptyName)
        );

        // Renaming "Default" is rejected
        assert_eq!(
            config.rename_setup("Default", "Primary"),
            Err(ActionRingSetupError::DefaultSetupReserved)
        );
        // Renaming to "Default" is rejected
        assert_eq!(
            config.rename_setup("Gaming", "Default"),
            Err(ActionRingSetupError::DefaultSetupReserved)
        );
        // Renaming custom setup succeeds
        config
            .rename_setup("Gaming", "Competitive")
            .expect("rename Gaming to Competitive");
        assert_eq!(config.setup(Some("Gaming")), None);
        assert!(config.setup(Some("Competitive")).is_some());
        assert_eq!(config.setup_names(), vec!["Default", "Competitive"]);

        // Duplicating from Default
        config
            .duplicate_setup(None, "FromDefault")
            .expect("duplicate from default");
        assert_eq!(
            config.setup(Some("FromDefault")).unwrap().default,
            config.default
        );

        // Duplicating custom setup
        config
            .duplicate_setup(Some("Competitive"), "CopyComp")
            .expect("duplicate custom setup");
        assert!(config.setup(Some("CopyComp")).is_some());

        // Deleting Default setup is rejected
        assert_eq!(
            config.delete_setup("Default"),
            Err(ActionRingSetupError::DefaultSetupReserved)
        );

        // Deleting custom setup succeeds
        config
            .delete_setup("CopyComp")
            .expect("delete custom setup");
        assert_eq!(config.setup(Some("CopyComp")), None);
    }

    #[test]
    fn setup_independent_layout_resolution() {
        let mut config = ActionRingConfig::default();
        config.create_setup("Gaming").expect("create Gaming");

        let (browser_back_entry, safari_entry, expected_gaming_default) = {
            let gaming_setup = config.setup_mut(Some("Gaming")).expect("gaming setup mut");
            let browser_back_entry = ActionRingEntry::new(
                RingAction::new(Action::BrowserBack).expect("browser back is valid"),
            );
            gaming_setup
                .default
                .slots
                .insert(ActionRingSlot::Top, browser_back_entry.clone());

            let safari_entry = ActionRingEntry::new(
                RingAction::new(Action::NewTab).expect("new tab is valid"),
            );
            gaming_setup.per_app.insert(
                "com.apple.Safari".to_string(),
                ActionRingLayout {
                    slots: BTreeMap::from([(ActionRingSlot::Top, safari_entry.clone())]),
                },
            );
            (browser_back_entry, safari_entry, gaming_setup.default.clone())
        };

        let aliases = AppAliases::default();
        // Resolving default setup
        assert_eq!(
            config.effective_layout_for(None, &aliases, Some("com.apple.Safari")),
            config.default
        );
        // Resolving Gaming setup with app override
        let gaming_safari = config.effective_layout_for(
            Some("Gaming"),
            &aliases,
            Some("com.apple.Safari"),
        );
        assert_eq!(
            gaming_safari.slots.get(&ActionRingSlot::Top),
            Some(&safari_entry)
        );
        // Resolving Gaming setup without app override uses Gaming's default
        let gaming_other = config.effective_layout_for(
            Some("Gaming"),
            &aliases,
            Some("com.other.App"),
        );
        assert_eq!(
            gaming_other.slots.get(&ActionRingSlot::Top),
            Some(&browser_back_entry)
        );
        // Nonexistent setup safely falls back to default setup
        assert_eq!(
            config.effective_layout_for(Some("Nonexistent"), &aliases, None),
            config.default
        );

        // Profile layout resolution
        assert_eq!(
            config.profile_layout_for(Some("Gaming"), Some("com.apple.Safari")),
            ActionRingLayout {
                slots: BTreeMap::from([(ActionRingSlot::Top, safari_entry)]),
            }
        );
        assert_eq!(
            config.profile_layout_for(Some("Gaming"), None),
            expected_gaming_default
        );
    }

    #[test]
    fn legacy_and_multi_setup_config_toml_roundtrip() {
        // Legacy single-setup config without `setups` table
        let legacy_toml = concat!(
            "enabled = true\n",
            "haptics = true\n",
            "\n",
            "[default.slots.Top]\n",
            "action = \"Cut\"\n",
        );
        let config: ActionRingConfig = toml::from_str(legacy_toml).expect("parse legacy config");
        assert_eq!(config.setups.len(), 0);
        let serialized = toml::to_string(&config).expect("serialize legacy config");
        assert!(!serialized.contains("setups"));

        // Multi-setup config serializes setups table
        let mut multi = config;
        multi.create_setup("Work").expect("create Work");
        let multi_serialized = toml::to_string(&multi).expect("serialize multi-setup config");
        assert!(multi_serialized.contains("setups.Work"));

        let deserialized_multi: ActionRingConfig =
            toml::from_str(&multi_serialized).expect("deserialize multi-setup config");
        assert_eq!(deserialized_multi, multi);
        assert!(deserialized_multi.setups.contains_key("Work"));
    }

    #[test]
    fn action_ring_setup_default() {
        let setup = ActionRingSetup::default();
        assert_eq!(setup.default, ActionRingLayout::default());
        assert!(setup.per_app.is_empty());
    }
}
