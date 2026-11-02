//! Tests for multiple Actions Ring setups: creation, switching, slot edits,
//! per-app profile overrides, renaming, duplicating, and deletion.

use super::*;
use openlogi_core::binding::{ActionRingSetupError, DEFAULT_SETUP_NAME};

#[test]
fn create_action_ring_setup_creates_new_setup_and_switches_editor() {
    let config = Config::ephemeral();
    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    assert_eq!(state.editing_action_ring_setup(), None);
    assert_eq!(
        state.current_action_ring().setup_names(),
        vec![DEFAULT_SETUP_NAME]
    );

    let result = state.create_action_ring_setup("Gaming");
    let _ = result.unwrap();
    assert_eq!(state.editing_action_ring_setup(), Some("Gaming"));
    assert_eq!(
        state.current_action_ring().setup_names(),
        vec![DEFAULT_SETUP_NAME, "Gaming"]
    );
    assert!(state.current_action_ring().setups.contains_key("Gaming"));

    // Duplicate creation fails
    let dup_err = state.create_action_ring_setup("Gaming");
    assert_eq!(
        dup_err.unwrap_err(),
        ActionRingSetupError::AlreadyExists("Gaming".into())
    );

    // Empty name fails
    let empty_err = state.create_action_ring_setup("   ");
    assert_eq!(empty_err.unwrap_err(), ActionRingSetupError::EmptyName);

    // Default setup name fails
    let def_err = state.create_action_ring_setup(DEFAULT_SETUP_NAME);
    assert_eq!(
        def_err.unwrap_err(),
        ActionRingSetupError::DefaultSetupReserved
    );
}

#[test]
fn editing_slots_in_custom_setup_modifies_only_that_setup() {
    let config = Config::ephemeral();
    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    let _ = state.create_action_ring_setup("Productivity").unwrap();
    let _ = state.set_editing_action_ring_setup(Some("Productivity".into()));
    assert_eq!(state.editing_action_ring_setup(), Some("Productivity"));

    // Top slot in default setup is Cut
    assert_eq!(
        state.current_action_ring().default.slots[&ActionRingSlot::Top].action(),
        &Action::Cut
    );

    // Commit new slot action while Productivity setup is active
    let _ = state.commit_action_ring_slot(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::Screenshot).unwrap()),
    );

    // Active layout shows Screenshot
    assert_eq!(
        state.current_action_ring_layout().slots[&ActionRingSlot::Top].action(),
        &Action::Screenshot
    );
    // Productivity setup has Screenshot
    assert_eq!(
        state.current_action_ring().setups["Productivity"].default.slots[&ActionRingSlot::Top]
            .action(),
        &Action::Screenshot
    );
    // Default setup is untouched and still Cut
    assert_eq!(
        state.current_action_ring().default.slots[&ActionRingSlot::Top].action(),
        &Action::Cut
    );

    // Switch back to Default setup
    let _ = state.set_editing_action_ring_setup(None);
    assert_eq!(state.editing_action_ring_setup(), None);
    assert_eq!(
        state.current_action_ring_layout().slots[&ActionRingSlot::Top].action(),
        &Action::Cut
    );
}

#[test]
fn per_app_profile_overrides_in_custom_setup_isolate_from_default() {
    let config = Config::ephemeral();
    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    let _ = state.create_action_ring_setup("Media").unwrap();
    let _ = state.set_editing_action_ring_setup(Some("Media".into()));
    let _ = state.set_editing_action_ring_app(Some("Safari".into()));

    let _ = state.commit_action_ring_slot(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::PlayPause).unwrap()),
    );

    // Media setup has Safari override
    assert_eq!(
        state.current_action_ring_layout().slots[&ActionRingSlot::Top].action(),
        &Action::PlayPause
    );
    assert_eq!(
        state.current_action_ring().setups["Media"].per_app["Safari"].slots[&ActionRingSlot::Top]
            .action(),
        &Action::PlayPause
    );
    // Default setup has no Safari override
    assert!(!state.current_action_ring().per_app.contains_key("Safari"));

    // Resetting profile removes override on Media setup
    let _ = state.reset_action_ring_profile(&DeviceKey::from(KNOWN_MOUSE_KEY), "Safari");
    assert!(!state.current_action_ring().setups["Media"]
        .per_app
        .contains_key("Safari"));
    // Falls back to Media's default layout
    assert_eq!(
        state.current_action_ring_layout(),
        state.current_action_ring().setups["Media"].default
    );
}

#[test]
fn rename_action_ring_setup_updates_bindings_and_references() {
    let mut config = Config::ephemeral();
    let setup_action = Action::ShowActionsRing(Some("Gaming".into()));
    config.set_binding(
        KNOWN_MOUSE_KEY,
        ButtonId::Forward,
        Binding::Single(setup_action.clone()),
    );
    config.set_per_app_binding(
        KNOWN_MOUSE_KEY,
        "Safari",
        ButtonId::Back,
        Some(setup_action),
    );

    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    let _ = state.create_action_ring_setup("Gaming").unwrap();
    let _ = state.set_editing_action_ring_setup(Some("Gaming".into()));

    let _ = state
        .rename_action_ring_setup("Gaming", "Esports")
        .unwrap();

    assert_eq!(state.editing_action_ring_setup(), Some("Esports"));
    assert!(!state.current_action_ring().setups.contains_key("Gaming"));
    assert!(state.current_action_ring().setups.contains_key("Esports"));

    // Button bindings referencing "Gaming" cascaded to "Esports"
    assert_eq!(
        state.button_bindings()[&ButtonId::Forward],
        Action::ShowActionsRing(Some("Esports".into()))
    );

    let app_binding = state
        .config
        .devices
        .get(KNOWN_MOUSE_KEY)
        .and_then(|d| d.per_app_bindings.get("Safari"))
        .and_then(|b| b.get(&ButtonId::Back));
    assert_eq!(
        app_binding,
        Some(&Action::ShowActionsRing(Some("Esports".into())))
    );
}

#[test]
fn delete_action_ring_setup_resets_bindings_and_reverts_editor_to_default() {
    let mut config = Config::ephemeral();
    config.set_binding(
        KNOWN_MOUSE_KEY,
        ButtonId::Forward,
        Binding::Single(Action::ShowActionsRing(Some("Esports".into()))),
    );

    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    let _ = state.create_action_ring_setup("Esports").unwrap();
    let _ = state.set_editing_action_ring_setup(Some("Esports".into()));

    let _ = state.delete_action_ring_setup("Esports").unwrap();

    assert_eq!(state.editing_action_ring_setup(), None);
    assert!(!state.current_action_ring().setups.contains_key("Esports"));

    // Button bindings referencing deleted setup reverted to Default ShowActionsRing(None)
    assert_eq!(
        state.button_bindings()[&ButtonId::Forward],
        Action::ShowActionsRing(None)
    );

    // Deleting Default setup is rejected
    let del_err = state.delete_action_ring_setup(DEFAULT_SETUP_NAME);
    assert_eq!(
        del_err.unwrap_err(),
        ActionRingSetupError::DefaultSetupReserved
    );
}

#[test]
fn duplicate_action_ring_setup_clones_custom_or_default_setup() {
    let config = Config::ephemeral();
    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    // Duplicate default setup
    let _ = state
        .duplicate_action_ring_setup(None, "DefaultClone")
        .unwrap();
    assert_eq!(state.editing_action_ring_setup(), Some("DefaultClone"));
    assert_eq!(
        state.current_action_ring().setups["DefaultClone"].default,
        state.current_action_ring().default
    );
}

#[test]
fn remove_all_app_profiles_prunes_custom_setups() {
    let config = Config::ephemeral();
    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    let _ = state.create_action_ring_setup("Design").unwrap();
    let _ = state.set_editing_action_ring_setup(Some("Design".into()));
    let _ = state.set_editing_action_ring_app(Some("Figma".into()));
    let _ = state.commit_action_ring_slot(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::Undo).unwrap()),
    );

    assert!(
        state.current_action_ring().setups["Design"]
            .per_app
            .contains_key("Figma")
    );

    let _ = state.remove_all_app_profiles(&DeviceKey::from(KNOWN_MOUSE_KEY), "Figma");

    assert!(
        !state.current_action_ring().setups["Design"]
            .per_app
            .contains_key("Figma")
    );
}

#[test]
fn edit_action_ring_layout_with_default_setup_name_targets_default_layout() {
    let config = Config::ephemeral();
    let resolver = AssetResolver::new();
    let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = AppState::new(Sources {
        inventories: &[direct_inventory([0xa3, 0x93, 0xca, 0xe0])],
        persistence: ConfigPersistence::MemoryOnly,
        ..Sources::in_memory(config, &resolver, commands)
    });

    state
        .action_ring_editing_setups
        .insert(KNOWN_MOUSE_KEY.into(), DEFAULT_SETUP_NAME.into());

    let _ = state.commit_action_ring_slot(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::Paste).unwrap()),
    );

    assert!(
        !state
            .current_action_ring()
            .setups
            .contains_key(DEFAULT_SETUP_NAME)
    );
    assert_eq!(
        state.current_action_ring().default.slots[&ActionRingSlot::Top].action(),
        &Action::Paste
    );
}
