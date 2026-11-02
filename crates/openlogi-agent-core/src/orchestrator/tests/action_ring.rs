//! Action Ring session resolution tests across multiple setups and button bindings.

use super::*;
use openlogi_core::binding::{ActionRingSlot, RingAction};

#[test]
fn action_ring_session_resolves_default_setup_when_unspecified() {
    let mut config = Config::default();
    let mut dev_cfg = DeviceConfig::default();
    dev_cfg.action_ring.create_setup("Gaming").unwrap();
    let gaming = dev_cfg.action_ring.setup_mut(Some("Gaming")).unwrap();
    gaming.default.set_action(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::Copy).unwrap()),
    );
    config.devices.insert("mouse-a".to_string(), dev_cfg);

    let mut orch = orchestrator(config);
    orch.devices = vec![dev("mouse-a", 1, true)];

    let session = orch
        .action_ring_session(Some("mouse-a"), None)
        .expect("session must be active");
    assert_eq!(
        session
            .layout
            .slots
            .get(&ActionRingSlot::Top)
            .map(|s| s.action()),
        Some(&Action::Cut)
    );
}

#[test]
fn action_ring_session_resolves_named_setup() {
    let mut config = Config::default();
    let mut dev_cfg = DeviceConfig::default();
    dev_cfg.action_ring.create_setup("Gaming").unwrap();
    let gaming = dev_cfg.action_ring.setup_mut(Some("Gaming")).unwrap();
    gaming.default.set_action(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::Copy).unwrap()),
    );
    config.devices.insert("mouse-a".to_string(), dev_cfg);

    let mut orch = orchestrator(config);
    orch.devices = vec![dev("mouse-a", 1, true)];

    let session = orch
        .action_ring_session(Some("mouse-a"), Some("Gaming"))
        .expect("session must be active");
    assert_eq!(
        session
            .layout
            .slots
            .get(&ActionRingSlot::Top)
            .map(|s| s.action()),
        Some(&Action::Copy)
    );
}

#[test]
fn action_ring_session_falls_back_to_default_for_nonexistent_setup() {
    let mut config = Config::default();
    let mut dev_cfg = DeviceConfig::default();
    dev_cfg.action_ring.create_setup("Gaming").unwrap();
    let gaming = dev_cfg.action_ring.setup_mut(Some("Gaming")).unwrap();
    gaming.default.set_action(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::Copy).unwrap()),
    );
    config.devices.insert("mouse-a".to_string(), dev_cfg);

    let mut orch = orchestrator(config);
    orch.devices = vec![dev("mouse-a", 1, true)];

    let session = orch
        .action_ring_session(Some("mouse-a"), Some("Nonexistent"))
        .expect("session must be active");
    assert_eq!(
        session
            .layout
            .slots
            .get(&ActionRingSlot::Top)
            .map(|s| s.action()),
        Some(&Action::Cut)
    );
}

#[test]
fn two_distinct_buttons_with_different_setups_trigger_corresponding_layouts() {
    let mut config = Config::default();
    let mut dev_cfg = DeviceConfig::default();
    dev_cfg.bindings.insert(
        ButtonId::Back,
        Binding::Single(Action::ShowActionsRing(None)),
    );
    dev_cfg.bindings.insert(
        ButtonId::Forward,
        Binding::Single(Action::ShowActionsRing(Some("Gaming".to_string()))),
    );
    dev_cfg.action_ring.create_setup("Gaming").unwrap();
    let gaming = dev_cfg.action_ring.setup_mut(Some("Gaming")).unwrap();
    gaming.default.set_action(
        ActionRingSlot::Top,
        Some(RingAction::new(Action::Copy).unwrap()),
    );
    config.devices.insert("mouse-a".to_string(), dev_cfg);

    let mut orch = orchestrator(config);
    orch.devices = vec![dev("mouse-a", 1, true)];

    let back_action = orch.config.devices["mouse-a"].bindings[&ButtonId::Back].click_action();
    let fwd_action = orch.config.devices["mouse-a"].bindings[&ButtonId::Forward].click_action();

    let back_setup = match back_action {
        Action::ShowActionsRing(ref s) => s.as_deref(),
        _ => panic!("expected ShowActionsRing"),
    };
    let fwd_setup = match fwd_action {
        Action::ShowActionsRing(ref s) => s.as_deref(),
        _ => panic!("expected ShowActionsRing"),
    };

    let back_session = orch
        .action_ring_session(Some("mouse-a"), back_setup)
        .expect("session must be active for Back");
    assert_eq!(
        back_session
            .layout
            .slots
            .get(&ActionRingSlot::Top)
            .map(|s| s.action()),
        Some(&Action::Cut)
    );

    let fwd_session = orch
        .action_ring_session(Some("mouse-a"), fwd_setup)
        .expect("session must be active for Forward");
    assert_eq!(
        fwd_session
            .layout
            .slots
            .get(&ActionRingSlot::Top)
            .map(|s| s.action()),
        Some(&Action::Copy)
    );
}
