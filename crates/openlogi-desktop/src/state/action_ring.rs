//! Actions Ring settings and editor scope for the selected device.

use openlogi_core::binding::{
    ActionRingConfig, ActionRingIcon, ActionRingLayout, ActionRingSetupError, ActionRingSlot,
    DEFAULT_SETUP_NAME, RingAction,
};

use super::events::StateEvents;
use super::{AppState, DeviceKey, DeviceRecord, StateEvent};

impl AppState {
    /// Actions Ring settings for the active device, including its implicit
    /// default layout when nothing has been persisted yet.
    #[must_use]
    pub fn current_action_ring(&self) -> ActionRingConfig {
        self.current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(|key| self.config.action_ring(key))
            .unwrap_or_default()
    }

    /// Application profile open in the Actions Ring editor, or `None` for the
    /// default layout.
    #[must_use]
    pub fn editing_action_ring_app(&self) -> Option<&str> {
        let key = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)?;
        self.action_ring_editing_apps.get(key).map(String::as_str)
    }

    /// Open an application-specific Actions Ring layout, or the default
    /// layout with `None`. This window-local choice is not persisted.
    pub fn set_editing_action_ring_app(&mut self, app: Option<String>) -> StateEvents {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return events;
        };
        if let Some(app) = app {
            self.action_ring_editing_apps.insert(key, app);
        } else {
            self.action_ring_editing_apps.remove(&key);
        }
        events
    }

    /// Setup open in the Actions Ring editor, or `None` for the "Default" setup.
    #[must_use]
    pub fn editing_action_ring_setup(&self) -> Option<&str> {
        let key = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)?;
        self.action_ring_editing_setups.get(key).map(String::as_str)
    }

    /// Open a named Actions Ring setup, or the default setup with `None` (or "Default").
    /// This window-local choice is not persisted.
    pub fn set_editing_action_ring_setup(&mut self, setup: Option<String>) -> StateEvents {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return events;
        };
        match setup {
            Some(name) if name != DEFAULT_SETUP_NAME => {
                self.action_ring_editing_setups.insert(key, name);
            }
            _ => {
                self.action_ring_editing_setups.remove(&key);
            }
        }
        events
    }

    /// Complete layout shown by the Actions Ring editor: the layout saved under
    /// the active setup and profile key it edits.
    #[must_use]
    pub fn current_action_ring_layout(&self) -> ActionRingLayout {
        self.current_action_ring().profile_layout_for(
            self.editing_action_ring_setup(),
            self.editing_action_ring_app(),
        )
    }

    /// Replace or clear one slot in the open Actions Ring layout.
    pub fn commit_action_ring_slot(
        &mut self,
        slot: ActionRingSlot,
        action: Option<RingAction>,
    ) -> StateEvents {
        self.edit_action_ring_layout("Actions Ring slot", |layout| {
            layout.set_action(slot, action);
        })
    }

    /// Set or restore the action-derived icon for one slot in the open layout.
    pub fn commit_action_ring_icon(
        &mut self,
        slot: ActionRingSlot,
        icon: Option<ActionRingIcon>,
    ) -> StateEvents {
        self.edit_action_ring_layout("Actions Ring icon", |layout| {
            layout.set_icon(slot, icon);
        })
    }

    fn edit_action_ring_layout(
        &mut self,
        what: &str,
        edit: impl FnOnce(&mut ActionRingLayout),
    ) -> StateEvents {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return events;
        };
        let app = self.editing_action_ring_app().map(str::to_string);
        let setup = self.editing_action_ring_setup().map(str::to_string);
        self.config.edit(|config| {
            let ring = &mut config.devices.entry(key).or_default().action_ring;
            let layout = match setup.as_deref() {
                Some(setup_name) if setup_name != DEFAULT_SETUP_NAME => {
                    let custom = ring.setups.entry(setup_name.to_string()).or_default();
                    match app {
                        Some(app) => {
                            let inherited = custom.default.clone();
                            custom.per_app.entry(app).or_insert(inherited)
                        }
                        None => &mut custom.default,
                    }
                }
                _ => match app {
                    Some(app) => {
                        let inherited = ring.default.clone();
                        ring.per_app.entry(app).or_insert(inherited)
                    }
                    None => &mut ring.default,
                },
            };
            edit(layout);
        });
        self.persist_and_reload(what);
        events
    }

    /// Restore live default inheritance while keeping the application's editor open.
    pub fn reset_action_ring_profile(&mut self, key: &DeviceKey, app: &str) -> StateEvents {
        self.clear_action_ring_profile(key.as_str(), app);
        StateEvent::BindingsChanged(key.clone()).into()
    }

    /// Remove the named ring profile without touching button bindings or a
    /// different editor selection. Failed saves preserve the open editor.
    pub fn remove_action_ring_profile(&mut self, key: &DeviceKey, app: &str) -> StateEvents {
        if self.clear_action_ring_profile(key.as_str(), app)
            && self
                .action_ring_editing_apps
                .get(key.as_str())
                .is_some_and(|selected| selected == app)
        {
            self.action_ring_editing_apps.remove(key.as_str());
        }
        StateEvent::BindingsChanged(key.clone()).into()
    }

    fn clear_action_ring_profile(&mut self, key: &str, app: &str) -> bool {
        let setup = self.action_ring_editing_setups.get(key).cloned();
        let has_override = self
            .config
            .devices
            .get(key)
            .is_some_and(|device| {
                if let Some(ref setup_name) = setup {
                    device
                        .action_ring
                        .setup(Some(setup_name))
                        .is_some_and(|s| s.per_app.contains_key(app))
                } else {
                    device.action_ring.per_app.contains_key(app)
                }
            });
        if !has_override {
            return true;
        }
        self.config.edit(|config| {
            if let Some(device) = config.devices.get_mut(key) {
                if let Some(ref setup_name) = setup {
                    if let Some(custom) = device.action_ring.setup_mut(Some(setup_name)) {
                        custom.per_app.remove(app);
                    }
                } else {
                    device.action_ring.per_app.remove(app);
                }
            }
        });
        self.persist_and_reload("Actions Ring application profile")
    }

    /// Create a new custom Actions Ring setup and switch the editor to it.
    pub fn create_action_ring_setup(
        &mut self,
        name: &str,
    ) -> Result<StateEvents, ActionRingSetupError> {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return Ok(events);
        };
        let trimmed = name.trim().to_string();
        self.config.edit(|config| {
            let device = config.devices.entry(key.clone()).or_default();
            device.action_ring.create_setup(&trimmed)
        })?;
        self.persist_and_reload("Create Actions Ring setup");
        self.action_ring_editing_setups.insert(key.clone(), trimmed);
        self.action_ring_editing_apps.remove(&key);
        Ok(events)
    }

    /// Rename an existing custom Actions Ring setup and cascade to all button references.
    pub fn rename_action_ring_setup(
        &mut self,
        old: &str,
        new: &str,
    ) -> Result<StateEvents, ActionRingSetupError> {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return Ok(events);
        };
        let old_trimmed = old.trim();
        let new_trimmed = new.trim();
        self.config.edit(|config| {
            let device = config.devices.entry(key.clone()).or_default();
            device.action_ring.rename_setup(old_trimmed, new_trimmed)?;
            device.rename_setup_references(old_trimmed, new_trimmed);
            Ok::<(), ActionRingSetupError>(())
        })?;
        if self
            .action_ring_editing_setups
            .get(&key)
            .is_some_and(|cur| cur == old_trimmed)
        {
            self.action_ring_editing_setups
                .insert(key, new_trimmed.to_string());
        }
        self.persist_and_reload("Rename Actions Ring setup");
        self.refresh_binding_projections();
        Ok(events)
    }

    /// Duplicate a custom setup or the Default setup into a new setup and switch the editor to it.
    pub fn duplicate_action_ring_setup(
        &mut self,
        source: Option<&str>,
        name: &str,
    ) -> Result<StateEvents, ActionRingSetupError> {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return Ok(events);
        };
        let trimmed = name.trim().to_string();
        self.config.edit(|config| {
            let device = config.devices.entry(key.clone()).or_default();
            device.action_ring.duplicate_setup(source, &trimmed)
        })?;
        self.persist_and_reload("Duplicate Actions Ring setup");
        self.action_ring_editing_setups.insert(key.clone(), trimmed);
        self.action_ring_editing_apps.remove(&key);
        Ok(events)
    }

    /// Delete a custom setup and revert button references to Default.
    /// If the deleted setup was open in the editor, reverts the editor to Default.
    pub fn delete_action_ring_setup(
        &mut self,
        name: &str,
    ) -> Result<StateEvents, ActionRingSetupError> {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return Ok(events);
        };
        let trimmed = name.trim();
        self.config.edit(|config| {
            let device = config.devices.entry(key.clone()).or_default();
            device.action_ring.delete_setup(trimmed)?;
            device.delete_setup_references(trimmed);
            Ok::<(), ActionRingSetupError>(())
        })?;
        if self
            .action_ring_editing_setups
            .get(&key)
            .is_some_and(|cur| cur == trimmed)
        {
            self.action_ring_editing_setups.remove(&key);
            self.action_ring_editing_apps.remove(&key);
        }
        self.persist_and_reload("Delete Actions Ring setup");
        self.refresh_binding_projections();
        Ok(events)
    }

    /// Enable or disable the active device's Actions Ring.
    pub fn commit_action_ring_enabled(&mut self, enabled: bool) -> StateEvents {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return events;
        };
        self.config
            .edit(|config| config.set_action_ring_enabled(&key, enabled));
        self.persist_and_reload("Actions Ring enabled state");
        events
    }

    /// Enable or disable hover and activation haptics for the active ring.
    pub fn commit_action_ring_haptics(&mut self, enabled: bool) -> StateEvents {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return events;
        };
        self.config
            .edit(|config| config.set_action_ring_haptics(&key, enabled));
        self.persist_and_reload("Actions Ring haptics");
        events
    }
}
