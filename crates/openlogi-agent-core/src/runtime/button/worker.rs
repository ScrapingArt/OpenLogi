//! The button runtime's worker thread: draining commands and inputs, settling long presses, and emitting the events the owner subscribed to.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use super::{
    ActivePress, ButtonCommand, ButtonInput, ButtonRuntimeEvent, ButtonState, CancelReason,
    EVENT_QUEUE_CAPACITY, EndReason, PressBehavior, PressToken, SHUTDOWN_POLL_PERIOD,
    ShutdownRequest,
};

pub(super) fn run_worker(
    events: &mpsc::Receiver<ButtonCommand>,
    shutdown: &mpsc::Receiver<ShutdownRequest>,
    shared_generation: &AtomicU64,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) {
    let mut state = ButtonState::default();
    let mut generation = shared_generation.load(Ordering::Acquire);
    loop {
        if finish_shutdown_if_requested(shutdown, &mut state, emit) {
            return;
        }
        let command = match events.recv_timeout(SHUTDOWN_POLL_PERIOD) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if settle_due_long_presses(
                    events,
                    shutdown,
                    shared_generation,
                    &mut generation,
                    &mut state,
                    None,
                    emit,
                ) {
                    return;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                emit_canceled(state.cancel_all(), CancelReason::SourceEnded, emit);
                return;
            }
        };
        synchronize_generation(&mut state, shared_generation, &mut generation, emit);
        if state.has_due_long_press(Instant::now()) {
            if settle_due_long_presses(
                events,
                shutdown,
                shared_generation,
                &mut generation,
                &mut state,
                Some(command),
                emit,
            ) {
                return;
            }
            continue;
        }
        process_command(&mut state, command, generation, emit);
        if settle_due_long_presses(
            events,
            shutdown,
            shared_generation,
            &mut generation,
            &mut state,
            None,
            emit,
        ) {
            return;
        }
    }
}

fn process_command(
    state: &mut ButtonState,
    command: ButtonCommand,
    generation: u64,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) {
    match command {
        ButtonCommand::Input {
            generation: input_generation,
            input,
        } if input_generation == generation => process_input(state, input, emit),
        ButtonCommand::Input { .. } | ButtonCommand::Wake => {}
        ButtonCommand::CancelStalePress(token) => {
            if let Some(press) = state.cancel_press(&token) {
                emit(ButtonRuntimeEvent::Ended {
                    press,
                    reason: EndReason::Canceled(CancelReason::StaleHold),
                });
            }
        }
        ButtonCommand::CancelSource(source) => {
            emit_canceled(
                state.cancel_source(&source),
                CancelReason::SourceEnded,
                emit,
            );
        }
        ButtonCommand::CancelHooks => {
            emit_canceled(state.cancel_hooks(), CancelReason::SourceEnded, emit);
        }
        ButtonCommand::CancelPointerExcept(current) => {
            emit_canceled(
                state.cancel_pointer_except(current),
                CancelReason::Invalidated,
                emit,
            );
        }
    }
}

pub(super) fn settle_due_long_presses(
    events: &mpsc::Receiver<ButtonCommand>,
    shutdown: &mpsc::Receiver<ShutdownRequest>,
    shared_generation: &AtomicU64,
    generation: &mut u64,
    state: &mut ButtonState,
    first_command: Option<ButtonCommand>,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) -> bool {
    if finish_shutdown_if_requested(shutdown, state, emit) {
        return true;
    }
    // An event handler may have blocked while another thread invalidated
    // this generation. Observe that transition before any overdue timer.
    synchronize_generation(state, shared_generation, generation, emit);
    let due_presses = state.due_long_presses(Instant::now());
    if due_presses.is_empty() {
        if let Some(command) = first_command {
            process_command(state, command, *generation, emit);
        }
        return false;
    }

    // Before firing an overdue timer, settle one channel-capacity snapshot.
    // FIFO ordering guarantees that this includes every command that was
    // already queued at the checkpoint. State transitions are separated from
    // synchronous handlers so unrelated actions cannot delay the deadline.
    let mut deferred = Vec::new();
    let queued_limit = if let Some(command) = first_command {
        process_command(state, command, *generation, &mut |event| {
            deferred.push(event);
        });
        if finish_deferred_shutdown_if_requested(shutdown, state, &due_presses, &mut deferred, emit)
        {
            return true;
        }
        synchronize_generation(state, shared_generation, generation, &mut |event| {
            deferred.push(event);
        });
        EVENT_QUEUE_CAPACITY - 1
    } else {
        EVENT_QUEUE_CAPACITY
    };
    for _ in 0..queued_limit {
        if finish_deferred_shutdown_if_requested(shutdown, state, &due_presses, &mut deferred, emit)
        {
            return true;
        }
        synchronize_generation(state, shared_generation, generation, &mut |event| {
            deferred.push(event);
        });
        let command = match events.try_recv() {
            Ok(command) => command,
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                emit_canceled(
                    state.cancel_all(),
                    CancelReason::SourceEnded,
                    &mut |event| deferred.push(event),
                );
                emit_settled_events(deferred, &due_presses, emit);
                return true;
            }
        };
        process_command(state, command, *generation, &mut |event| {
            deferred.push(event);
        });
        if finish_deferred_shutdown_if_requested(shutdown, state, &due_presses, &mut deferred, emit)
        {
            return true;
        }
        synchronize_generation(state, shared_generation, generation, &mut |event| {
            deferred.push(event);
        });
    }

    emit_selected_long_presses(state, &due_presses, Instant::now(), &mut |event| {
        deferred.push(event);
    });
    emit_settled_events(deferred, &due_presses, emit);
    false
}

fn finish_deferred_shutdown_if_requested(
    shutdown: &mpsc::Receiver<ShutdownRequest>,
    state: &mut ButtonState,
    due_presses: &[PressToken],
    deferred: &mut Vec<ButtonRuntimeEvent>,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) -> bool {
    let Ok(request) = shutdown.try_recv() else {
        return false;
    };
    emit_canceled(state.cancel_all(), CancelReason::Shutdown, &mut |event| {
        deferred.push(event);
    });
    emit_settled_events(std::mem::take(deferred), due_presses, emit);
    let _ = request.done.send(());
    true
}

fn emit_settled_events(
    events: Vec<ButtonRuntimeEvent>,
    due_presses: &[PressToken],
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) {
    let (deadline_events, ordered_events): (Vec<_>, Vec<_>) =
        events.into_iter().partition(|event| match event {
            ButtonRuntimeEvent::Triggered { press, .. }
            | ButtonRuntimeEvent::Ended { press, .. } => {
                matches!(press.behavior, PressBehavior::LongPressFired)
                    && due_presses.contains(press.token())
            }
            ButtonRuntimeEvent::Started(_) => false,
        });
    for event in deadline_events.into_iter().chain(ordered_events) {
        emit(event);
    }
}

fn finish_shutdown_if_requested(
    shutdown: &mpsc::Receiver<ShutdownRequest>,
    state: &mut ButtonState,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) -> bool {
    let Ok(request) = shutdown.try_recv() else {
        return false;
    };
    emit_canceled(state.cancel_all(), CancelReason::Shutdown, emit);
    let _ = request.done.send(());
    true
}

fn synchronize_generation(
    state: &mut ButtonState,
    shared_generation: &AtomicU64,
    generation: &mut u64,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) {
    let current = shared_generation.load(Ordering::Acquire);
    if current != *generation {
        emit_canceled(state.cancel_all(), CancelReason::Invalidated, emit);
        *generation = current;
    }
}

pub(super) fn process_input(
    state: &mut ButtonState,
    input: ButtonInput,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) {
    match input {
        ButtonInput::Down(press) => {
            let (new_press, stale) =
                if let Some(existing) = state.take_awaiting_multi_click(&press.token.key) {
                    let next_count = existing.multi_click_count().saturating_add(1);
                    let binding = existing.multi_click_binding().unwrap().clone();
                    let hold_deadline = binding
                        .hold()
                        .map(|_| Instant::now() + openlogi_core::binding::LONG_PRESS_THRESHOLD);
                    let behavior = PressBehavior::MultiClickPending {
                        binding,
                        count: next_count,
                        hold_deadline,
                    };
                    let updated = ActivePress {
                        token: press.token.clone(),
                        behavior,
                        target: press.target.clone(),
                    };
                    (updated, None)
                } else {
                    let stale = state.press(press.clone());
                    (press.clone(), stale)
                };
            if let Some(stale) = stale {
                emit(ButtonRuntimeEvent::Ended {
                    press: stale,
                    reason: EndReason::Canceled(CancelReason::RepeatedDown),
                });
            }
            state.press(new_press.clone());
            emit(ButtonRuntimeEvent::Started(new_press));
        }
        ButtonInput::Up { key, released_at } => {
            if let Some(mut press) = state.release(&key) {
                if let PressBehavior::MultiClickPending { binding, count, .. } = &press.behavior {
                    let count = *count;
                    let binding = binding.clone();
                    if count == 1 {
                        let awaiting = ActivePress {
                            token: press.token.clone(),
                            behavior: PressBehavior::MultiClickAwaiting {
                                binding,
                                count: 1,
                                deadline: released_at
                                    + openlogi_core::binding::DOUBLE_CLICK_TIMEOUT,
                            },
                            target: press.target.clone(),
                        };
                        state.press(awaiting);
                        return;
                    } else if count == 2 {
                        if binding.triple().is_some() {
                            let awaiting = ActivePress {
                                token: press.token.clone(),
                                behavior: PressBehavior::MultiClickAwaiting {
                                    binding,
                                    count: 2,
                                    deadline: released_at
                                        + openlogi_core::binding::DOUBLE_CLICK_TIMEOUT,
                                },
                                target: press.target.clone(),
                            };
                            state.press(awaiting);
                            return;
                        }
                        emit(ButtonRuntimeEvent::Triggered {
                            press: press.clone(),
                            action: binding.double().clone(),
                        });
                        emit(ButtonRuntimeEvent::Ended {
                            press,
                            reason: EndReason::Released,
                        });
                        return;
                    } else {
                        let action = binding.triple().unwrap_or(binding.double()).clone();
                        emit(ButtonRuntimeEvent::Triggered {
                            press: press.clone(),
                            action,
                        });
                        emit(ButtonRuntimeEvent::Ended {
                            press,
                            reason: EndReason::Released,
                        });
                        return;
                    }
                }

                if let Some(action) = press.fire_long(released_at) {
                    emit(ButtonRuntimeEvent::Triggered {
                        press: press.clone(),
                        action,
                    });
                }
                emit_released(press, emit);
            }
        }
        ButtonInput::Pulse(press) => {
            if let Some(stale) = state.press(press.clone()) {
                emit(ButtonRuntimeEvent::Ended {
                    press: stale,
                    reason: EndReason::Canceled(CancelReason::RepeatedDown),
                });
            }
            emit(ButtonRuntimeEvent::Started(press.clone()));
            if let Some(press) = state.release(&press.token.key) {
                emit_released(press, emit);
            }
        }
        ButtonInput::TriggerWhilePressed { token, action } => {
            if let Some(press) = state.active(&token).cloned() {
                emit(ButtonRuntimeEvent::Triggered { press, action });
            }
        }
        ButtonInput::WheelWhilePressed { token } => state.wheel_moved(&token),
    }
}

pub(super) fn emit_selected_long_presses(
    state: &mut ButtonState,
    tokens: &[PressToken],
    now: Instant,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) {
    for (press, action, was_awaiting) in state.fire_selected_long_presses(tokens, now) {
        emit(ButtonRuntimeEvent::Triggered {
            press: press.clone(),
            action,
        });
        if was_awaiting {
            emit(ButtonRuntimeEvent::Ended {
                press,
                reason: EndReason::Released,
            });
        }
    }
}

fn emit_released(press: ActivePress, emit: &mut impl FnMut(ButtonRuntimeEvent)) {
    if let Some(action) = press.release_action().cloned() {
        emit(ButtonRuntimeEvent::Triggered {
            press: press.clone(),
            action,
        });
    }
    emit(ButtonRuntimeEvent::Ended {
        press,
        reason: EndReason::Released,
    });
}

pub(super) fn emit_canceled(
    presses: Vec<ActivePress>,
    reason: CancelReason,
    emit: &mut impl FnMut(ButtonRuntimeEvent),
) {
    for press in presses {
        emit(ButtonRuntimeEvent::Ended {
            press,
            reason: EndReason::Canceled(reason),
        });
    }
}
