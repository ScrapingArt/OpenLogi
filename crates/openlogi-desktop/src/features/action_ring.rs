//! Eight-slot Actions Ring editor for the active device.

mod action_icons;
mod editor;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, prelude::FluentBuilder as _, px, rgb, svg,
};
use gpui_base::Button as BaseButton;
use gpui_component::{
    Icon, IconName, Selectable as _, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::DialogButtonProps,
    h_flex,
    input::InputState,
    menu::{DropdownMenu as _, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use openlogi_core::binding::{
    ActionRingConfig, ActionRingEntry, ActionRingIcon, ActionRingLayout, ActionRingSlot,
    DEFAULT_SETUP_NAME,
};
use openlogi_ui::action_icons::RING_CANCEL_ICON;

use self::action_icons::action_icon_path;
use self::editor::{PowerUserInputs, action_library};
use crate::features::keyboard::editors::{PowerUserKind, text_editor_placeholder};
use crate::state::{AppState, StateEvent, StateEvents};
use crate::ui::action::localized_action_label;
use crate::ui::components::{control_button, control_input, save_cancel_footer};
use crate::ui::theme::{self, Palette, SelectableStyle as _, Typography as _};

/// Stateful Actions Ring editor. Ring configuration itself lives in
/// [`AppState`]; this entity owns selection and editor input state.
pub struct ActionRingPanel {
    focus_handle: FocusHandle,
    selected_slot: ActionRingSlot,
    application_input: Option<Entity<InputState>>,
    shortcut_input: Option<Entity<InputState>>,
    type_text_input: Option<Entity<InputState>>,
    applescript_input: Option<Entity<InputState>>,
    shell_command_input: Option<Entity<InputState>>,
    library_scroll: ScrollHandle,
    #[expect(dead_code, reason = "held to keep the AppState subscription alive")]
    state_obs: Subscription,
}

impl ActionRingPanel {
    /// Create the editor and repaint it after any config/device change.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let state_obs =
            AppState::repaint_on(cx, |event| matches!(event, StateEvent::BindingsChanged(_)));
        Self {
            focus_handle: cx.focus_handle(),
            selected_slot: ActionRingSlot::Top,
            application_input: None,
            shortcut_input: None,
            type_text_input: None,
            applescript_input: None,
            shell_command_input: None,
            library_scroll: ScrollHandle::new(),
            state_obs,
        }
    }

    /// Get-or-create the three Power User text inputs, split out of `render`
    /// to keep it under clippy's line budget.
    fn power_user_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PowerUserInputs {
        PowerUserInputs {
            type_text: editor_input(
                &mut self.type_text_input,
                text_editor_placeholder(PowerUserKind::TypeText),
                window,
                cx,
            ),
            applescript: editor_input(
                &mut self.applescript_input,
                text_editor_placeholder(PowerUserKind::RunAppleScript),
                window,
                cx,
            ),
            shell_command: editor_input(
                &mut self.shell_command_input,
                text_editor_placeholder(PowerUserKind::RunShellCommand),
                window,
                cx,
            ),
        }
    }
}

impl Focusable for ActionRingPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ActionRingPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = theme::palette(cx);
        let (ring, layout) = action_ring_editor_state(cx);
        let active_setup = AppState::try_read(cx)
            .and_then(|state| state.editing_action_ring_setup().map(str::to_string));
        let haptics_supported = current_device_supports_haptics(cx);
        let application_input = editor_input(
            &mut self.application_input,
            tr!("action_ring.application_folder_path_or_url"),
            window,
            cx,
        );
        let shortcut_input = editor_input(
            &mut self.shortcut_input,
            tr!("action_ring.shortcut_e_g_cmd_plus_shift_plus_p"),
            window,
            cx,
        );
        let power_user = self.power_user_inputs(window, cx);
        let view = cx.entity();

        v_flex()
            .w_full()
            .gap_4()
            .tab_group()
            .track_focus(&self.focus_handle)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_subheading()
                            .child(tr!("action_ring.actions_ring")),
                    )
                    .child(
                        div()
                            .text_caption()
                            .text_color(pal.text_muted)
                            .child(tr!("action_ring.action_ring_description")),
                    ),
            )
            .child(setup_switcher_bar(&ring, active_setup.as_deref(), pal))
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .justify_center()
                    .gap_4()
                    .child(ring_preview(&layout, self.selected_slot, &view, pal))
                    .child(action_library(
                        self.selected_slot,
                        layout.slots.get(&self.selected_slot),
                        &application_input,
                        &shortcut_input,
                        &power_user,
                        &self.library_scroll,
                        pal,
                    )),
            )
            .child(enabled_row(ring.enabled, pal))
            .when(haptics_supported, |panel| {
                panel.child(haptics_row(ring.haptics, pal))
            })
    }
}

/// The ring on/off toggle row.
fn enabled_row(enabled: bool, pal: Palette) -> impl IntoElement {
    h_flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            v_flex()
                .child(div().text_body().child(tr!("action_ring.actions_ring")))
                .child(
                    div()
                        .text_caption()
                        .text_color(pal.text_muted)
                        .child(tr!("action_ring.open_at_the_current_cursor_position")),
                ),
        )
        .child(toggle_button(
            "ring-enabled",
            enabled,
            AppState::commit_action_ring_enabled,
        ))
}

/// The haptic-feedback toggle row, shown only when the device supports it.
fn haptics_row(haptics: bool, pal: Palette) -> impl IntoElement {
    h_flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            v_flex()
                .child(div().text_body().child(tr!("action_ring.haptic_feedback")))
                .child(
                    div()
                        .text_caption()
                        .text_color(pal.text_muted)
                        .child(tr!("action_ring.action_ring_haptic_description")),
                ),
        )
        .child(toggle_button(
            "ring-haptics",
            haptics,
            AppState::commit_action_ring_haptics,
        ))
}

fn action_ring_editor_state(cx: &Context<ActionRingPanel>) -> (ActionRingConfig, ActionRingLayout) {
    AppState::try_read(cx).map_or_else(
        || {
            let ring = ActionRingConfig::default();
            let layout = ring.default.clone();
            (ring, layout)
        },
        |state| {
            let ring = state.current_action_ring();
            let layout = state.current_action_ring_layout();
            (ring, layout)
        },
    )
}

fn editor_input(
    state: &mut Option<Entity<InputState>>,
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<ActionRingPanel>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let state = state
        .get_or_insert_with(|| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.clone()))
        })
        .clone();
    // Callers pass a per-render `tr!` string, so a cached input follows a live
    // language switch instead of keeping the placeholder it was built with.
    crate::ui::components::localize_placeholder(&state, placeholder, window, cx);
    state
}

fn current_device_supports_haptics(cx: &Context<ActionRingPanel>) -> bool {
    AppState::try_read(cx).is_some_and(|state| {
        state.current_record().is_some_and(|record| {
            record
                .capabilities
                .unwrap_or_else(|| {
                    openlogi_core::device::Capabilities::presumed_from_kind(record.kind)
                })
                .haptic_feedback
        })
    })
}

fn toggle_button(
    id: &'static str,
    enabled: bool,
    commit: impl Fn(&mut AppState, bool) -> StateEvents + 'static,
) -> Button {
    Button::new(id)
        .compact()
        .label(if enabled {
            tr!("common.on")
        } else {
            tr!("common.off")
        })
        .selected(enabled)
        .on_click(move |_, _, cx| AppState::apply(cx, |state| commit(state, !enabled)))
}

const PREVIEW_SIZE: f32 = 320.0;
const PREVIEW_RADIUS: f32 = 106.0;
const PREVIEW_SLOT_SIZE: f32 = 50.0;

fn ring_preview(
    layout: &ActionRingLayout,
    selected_slot: ActionRingSlot,
    view: &Entity<ActionRingPanel>,
    pal: Palette,
) -> impl IntoElement {
    div()
        .relative()
        .flex_none()
        .size(px(PREVIEW_SIZE))
        .child(
            div()
                .absolute()
                .left(px(24.0))
                .top(px(24.0))
                .size(px(PREVIEW_SIZE - 48.0))
                .rounded_full()
                .border_1()
                .border_color(pal.border)
                .bg(pal.panel),
        )
        .child(
            div()
                .absolute()
                .left(px(PREVIEW_SIZE / 2.0 - 24.0))
                .top(px(PREVIEW_SIZE / 2.0 - 24.0))
                .size(px(48.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(pal.muted)
                .text_color(pal.text_muted)
                .child(svg().path(RING_CANCEL_ICON).size(px(20.0)).flex_none()),
        )
        .children(ActionRingSlot::ALL.into_iter().map(|slot| {
            slot_button(
                slot,
                layout.slots.get(&slot),
                selected_slot == slot,
                view,
                pal,
            )
        }))
}

fn slot_button(
    slot: ActionRingSlot,
    entry: Option<&ActionRingEntry>,
    selected: bool,
    view: &Entity<ActionRingPanel>,
    pal: Palette,
) -> impl IntoElement {
    let index = slot.index();
    let (left, top) = slot.placement(PREVIEW_SIZE, PREVIEW_RADIUS, PREVIEW_SLOT_SIZE);
    let label = entry.map_or_else(
        || tr!("action_ring.empty_slot").to_string(),
        |entry| localized_action_label(entry.action()).to_string(),
    );
    let icon_path = entry.map(|entry| {
        entry.custom_icon().map_or_else(
            || action_icon_path(entry.action()),
            ActionRingIcon::asset_path,
        )
    });
    let accessible_label = label.clone();
    let selected_view = view.clone();

    BaseButton::new(("action-ring-slot", index))
        .selected(selected)
        .absolute()
        .left(px(left))
        .top(px(top))
        .size(px(PREVIEW_SLOT_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_2()
        .border_color(if selected {
            rgb(theme::ACCENT_BLUE).into()
        } else {
            pal.border
        })
        .bg(if selected {
            theme::accent_tint()
        } else {
            pal.control
        })
        .text_color(if selected {
            pal.text_primary
        } else {
            pal.text_muted
        })
        .cursor_pointer()
        .accessibility_label(accessible_label)
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .when_some(icon_path, |button, path| {
            button.child(svg().path(path).size(px(20.0)).text_color(if selected {
                pal.text_primary
            } else {
                pal.text_muted
            }))
        })
        .when(icon_path.is_none(), |button| {
            button.child(Icon::new(IconName::Plus).size_4())
        })
        .hover(move |button| {
            button.bg(if selected {
                theme::accent_tint_hover()
            } else {
                pal.control_hover
            })
        })
        .focus_visible(move |button| {
            button
                .border_color(rgb(theme::ACCENT_BLUE))
                .bg(if selected {
                    theme::accent_tint_hover()
                } else {
                    pal.control_hover
                })
        })
        .on_click(move |_, _, cx| {
            selected_view.update(cx, |panel, cx| {
                panel.selected_slot = slot;
                cx.notify();
            });
        })
}

fn setup_switcher_bar(
    ring: &ActionRingConfig,
    active_setup: Option<&str>,
    pal: Palette,
) -> impl IntoElement {
    let is_default_selected =
        active_setup.is_none() || active_setup == Some(DEFAULT_SETUP_NAME);
    let selected_custom_setup = if is_default_selected {
        None
    } else {
        active_setup.map(str::to_string)
    };

    h_flex()
        .flex_shrink_0()
        .w_full()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .px_4()
        .py_2()
        .child(
            div()
                .flex_none()
                .text_body()
                .text_color(pal.text_muted)
                .child(tr!("action_ring.setups")),
        )
        .child(
            h_flex()
                .id("action-ring:setups-scroll")
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .overflow_x_scroll()
                .child(
                    setup_tab(
                        "action-ring:setup:default",
                        tr!("common.default"),
                        is_default_selected,
                        pal,
                    )
                    .on_click(|_, _, cx| {
                        AppState::apply(cx, |state| state.set_editing_action_ring_setup(None));
                    }),
                )
                .children(custom_setup_tabs(ring, active_setup, pal))
                .child(
                    control_button("action-ring:add-setup")
                        .ghost()
                        .icon(IconName::Plus)
                        .tooltip(tr!("action_ring.new_setup"))
                        .on_click(|_, window, cx| {
                            open_new_setup_dialog(window, cx);
                        }),
                ),
        )
        .when_some(selected_custom_setup, |row, setup_name| {
            row.child(setup_options_menu(setup_name))
        })
}

fn custom_setup_tabs(
    ring: &ActionRingConfig,
    active_setup: Option<&str>,
    pal: Palette,
) -> Vec<BaseButton> {
    let active = active_setup.map(str::to_string);
    ring.setups
        .keys()
        .cloned()
        .map(move |name| {
            let selected = active.as_deref() == Some(&name);
            let setup_name = name.clone();
            setup_tab(format!("action-ring:setup:{name}"), name, selected, pal).on_click(
                move |_, _, cx| {
                    AppState::apply(cx, |state| {
                        state.set_editing_action_ring_setup(Some(setup_name.clone()))
                    });
                },
            )
        })
        .collect()
}

fn setup_tab(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    selected: bool,
    pal: Palette,
) -> BaseButton {
    let label = label.into();
    BaseButton::new(id)
        .role(Role::Tab)
        .selected(selected)
        .accessibility_label(label.clone())
        .aria_selected(selected)
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .h(px(theme::CONTROL_H))
        .px_2p5()
        .rounded(pal.control_radius)
        .cursor_pointer()
        .text_body()
        .text_color(pal.text_primary)
        .selected_fill(selected)
        .hover(move |tab| {
            tab.bg(if selected {
                theme::accent_tint_hover()
            } else {
                pal.control_hover
            })
        })
        .focus_visible(move |tab| {
            tab.bg(if selected {
                theme::accent_tint_hover()
            } else {
                pal.control_hover
            })
        })
        .child(label)
}

fn setup_options_menu(setup_name: String) -> impl IntoElement {
    control_button("action-ring:setup-options")
        .ghost()
        .label(tr!("action_ring.setup_options"))
        .icon(IconName::ChevronDown)
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let rename_name = setup_name.clone();
            let dup_name = setup_name.clone();
            let delete_name = setup_name.clone();
            menu.label(setup_name.clone())
                .item(
                    PopupMenuItem::new(tr!("action_ring.rename_setup_dialog")).on_click(
                        move |_, window, cx| {
                            let name = rename_name.clone();
                            window.defer(cx, move |window, cx| {
                                open_rename_setup_dialog(window, cx, &name);
                            });
                        },
                    ),
                )
                .item(
                    PopupMenuItem::new(tr!("action_ring.duplicate_setup_dialog")).on_click(
                        move |_, window, cx| {
                            let name = dup_name.clone();
                            window.defer(cx, move |window, cx| {
                                open_duplicate_setup_dialog(window, cx, Some(&name));
                            });
                        },
                    ),
                )
                .item(
                    PopupMenuItem::new(tr!("action_ring.delete_setup_dialog")).on_click(
                        move |_, window, cx| {
                            let name = delete_name.clone();
                            window.defer(cx, move |window, cx| {
                                open_delete_setup_dialog(window, cx, &name);
                            });
                        },
                    ),
                )
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupNameCheck {
    Valid,
    Empty,
    AlreadyExists,
}

fn check_setup_name(
    ring: &ActionRingConfig,
    name: &str,
    current_name: Option<&str>,
) -> SetupNameCheck {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        SetupNameCheck::Empty
    } else if current_name == Some(trimmed) {
        SetupNameCheck::Valid
    } else if trimmed == DEFAULT_SETUP_NAME || ring.setups.contains_key(trimmed) {
        SetupNameCheck::AlreadyExists
    } else {
        SetupNameCheck::Valid
    }
}

fn open_new_setup_dialog(window: &mut Window, cx: &mut App) {
    let input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr!("action_ring.setup_name"))
    });
    let tried = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx));
        let pal = theme::palette(cx);
        let ring = AppState::try_read(cx)
            .map(AppState::current_action_ring)
            .unwrap_or_default();
        let name = input.read(cx).value();
        let check = check_setup_name(&ring, &name, None);
        let error_caption = if tried.get() {
            match check {
                SetupNameCheck::Valid => None,
                SetupNameCheck::Empty => Some(tr!("action_ring.setup_name_cannot_be_empty")),
                SetupNameCheck::AlreadyExists => {
                    Some(tr!("action_ring.setup_name_already_exists"))
                }
            }
        } else {
            None
        };
        dialog
            .w(px(400.))
            .title(tr!("action_ring.new_setup"))
            .child(
                v_flex()
                    .gap_2()
                    .child(control_input(&input))
                    .when_some(error_caption, |flex, caption| {
                        flex.child(
                            div()
                                .text_caption()
                                .text_color(pal.text_muted)
                                .child(caption),
                        )
                    }),
            )
            .footer(save_cancel_footer())
            .on_ok({
                let input = input.clone();
                let tried = tried.clone();
                move |_, window, cx| {
                    let ring = AppState::try_read(cx)
                        .map(AppState::current_action_ring)
                        .unwrap_or_default();
                    let name = input.read(cx).value();
                    if check_setup_name(&ring, &name, None) != SetupNameCheck::Valid {
                        tried.set(true);
                        window.refresh();
                        return false;
                    }
                    let trimmed = name.trim().to_string();
                    AppState::apply(cx, |state| {
                        state
                            .create_action_ring_setup(&trimmed)
                            .unwrap_or_default()
                    });
                    true
                }
            })
    });
}

fn open_rename_setup_dialog(window: &mut Window, cx: &mut App, old_name: &str) {
    let old = old_name.to_string();
    let old_for_ok = old.clone();
    let old_for_check = old.clone();
    let input = cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(tr!("action_ring.setup_name"));
        input.set_value(old.clone(), window, cx);
        input
    });
    let tried = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx));
        let pal = theme::palette(cx);
        let ring = AppState::try_read(cx)
            .map(AppState::current_action_ring)
            .unwrap_or_default();
        let name = input.read(cx).value();
        let check = check_setup_name(&ring, &name, Some(&old_for_check));
        let error_caption = if tried.get() {
            match check {
                SetupNameCheck::Valid => None,
                SetupNameCheck::Empty => Some(tr!("action_ring.setup_name_cannot_be_empty")),
                SetupNameCheck::AlreadyExists => {
                    Some(tr!("action_ring.setup_name_already_exists"))
                }
            }
        } else {
            None
        };
        dialog
            .w(px(400.))
            .title(tr!("action_ring.rename_setup"))
            .child(
                v_flex()
                    .gap_2()
                    .child(control_input(&input))
                    .when_some(error_caption, |flex, caption| {
                        flex.child(
                            div()
                                .text_caption()
                                .text_color(pal.text_muted)
                                .child(caption),
                        )
                    }),
            )
            .footer(save_cancel_footer())
            .on_ok({
                let input = input.clone();
                let old = old_for_ok.clone();
                let tried = tried.clone();
                move |_, window, cx| {
                    let ring = AppState::try_read(cx)
                        .map(AppState::current_action_ring)
                        .unwrap_or_default();
                    let name = input.read(cx).value();
                    if check_setup_name(&ring, &name, Some(&old)) != SetupNameCheck::Valid {
                        tried.set(true);
                        window.refresh();
                        return false;
                    }
                    let trimmed = name.trim().to_string();
                    AppState::apply(cx, |state| {
                        state
                            .rename_action_ring_setup(&old, &trimmed)
                            .unwrap_or_default()
                    });
                    true
                }
            })
    });
}

fn open_duplicate_setup_dialog(window: &mut Window, cx: &mut App, source: Option<&str>) {
    let source = source.map(str::to_string);
    let source_for_ok = source.clone();
    let input = cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(tr!("action_ring.setup_name"));
        if let Some(ref src) = source {
            input.set_value(format!("{src} Copy"), window, cx);
        }
        input
    });
    let tried = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx));
        let pal = theme::palette(cx);
        let ring = AppState::try_read(cx)
            .map(AppState::current_action_ring)
            .unwrap_or_default();
        let name = input.read(cx).value();
        let check = check_setup_name(&ring, &name, None);
        let error_caption = if tried.get() {
            match check {
                SetupNameCheck::Valid => None,
                SetupNameCheck::Empty => Some(tr!("action_ring.setup_name_cannot_be_empty")),
                SetupNameCheck::AlreadyExists => {
                    Some(tr!("action_ring.setup_name_already_exists"))
                }
            }
        } else {
            None
        };
        dialog
            .w(px(400.))
            .title(tr!("action_ring.duplicate_setup"))
            .child(
                v_flex()
                    .gap_2()
                    .child(control_input(&input))
                    .when_some(error_caption, |flex, caption| {
                        flex.child(
                            div()
                                .text_caption()
                                .text_color(pal.text_muted)
                                .child(caption),
                        )
                    }),
            )
            .footer(save_cancel_footer())
            .on_ok({
                let input = input.clone();
                let source = source_for_ok.clone();
                let tried = tried.clone();
                move |_, window, cx| {
                    let ring = AppState::try_read(cx)
                        .map(AppState::current_action_ring)
                        .unwrap_or_default();
                    let name = input.read(cx).value();
                    if check_setup_name(&ring, &name, None) != SetupNameCheck::Valid {
                        tried.set(true);
                        window.refresh();
                        return false;
                    }
                    let trimmed = name.trim().to_string();
                    AppState::apply(cx, |state| {
                        state
                            .duplicate_action_ring_setup(source.as_deref(), &trimmed)
                            .unwrap_or_default()
                    });
                    true
                }
            })
    });
}

fn open_delete_setup_dialog(window: &mut Window, cx: &mut App, name: &str) {
    let name = name.to_string();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let name_for_ok = name.clone();
        alert
            .title(tr!("action_ring.delete_setup_question", name => name.clone()))
            .description(tr!("action_ring.delete_setup_description"))
            .button_props(
                DialogButtonProps::default()
                    .ok_text(tr!("action_ring.delete_setup"))
                    .ok_variant(ButtonVariant::Danger)
                    .cancel_text(tr!("common.cancel"))
                    .show_cancel(true),
            )
            .on_ok(move |_event, _window, cx| {
                AppState::apply(cx, |state| {
                    state
                        .delete_action_ring_setup(&name_for_ok)
                        .unwrap_or_default()
                });
                true
            })
    });
}
