// Copyright (C) 2026  Braiins Systems s.r.o.

//! Routes widget action requests to hardware controllers.
//!
use std::collections::BTreeSet;
use std::str::FromStr;

use bmc_widget_protocol::ActionPayload;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::sleep_until;
use tracing::{info, warn};

use super::led_state::LedSceneManager;
use crate::compositor::{ActiveScene, LedRequestStatusEvent, WidgetAction};
use crate::led_coordinator::LedCoordinatorHandle;
use crate::sound::{SoundController, Sounds};

/// Compositor-facing channels the action handler owns for its lifetime.
pub(crate) struct CompositorIo {
    pub action_rx: mpsc::UnboundedReceiver<WidgetAction>,
    pub active_scene_rx: watch::Receiver<Option<ActiveScene>>,
    pub connected_widgets_rx: watch::Receiver<BTreeSet<crate::compositor::InstanceId>>,
    pub status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
}

/// Spawn a task that receives widget actions from the compositor and
/// dispatches them to the appropriate hardware controller.
///
/// Sound playback runs on a dedicated task — `play_sound` blocks until
/// the clip ends, and we don't want that to stall LED processing.
pub(crate) fn spawn_action_handler(
    io: CompositorIo,
    sound_controller: SoundController,
    led_coordinator: LedCoordinatorHandle,
    initial_widget_scene_map: crate::config::WidgetSceneMap,
    mut scenes_rx: broadcast::Receiver<crate::config::WidgetSceneMap>,
) {
    let CompositorIo {
        mut action_rx,
        mut active_scene_rx,
        mut connected_widgets_rx,
        status_tx,
    } = io;

    // Unbounded so a StopSound is never dropped under backpressure — the
    // sound manager drains fast (it only cancels and spawns), so it can't grow.
    let (sound_tx, sound_rx) = mpsc::unbounded_channel();
    spawn_sound_manager(sound_rx, sound_controller);

    tokio::spawn(async move {
        let mut led = LedSceneManager::new(led_coordinator, status_tx);
        led.on_config_snapshot(initial_widget_scene_map);

        // Seed from the compositor's current state (latest-value watches).
        let initial_scene = active_scene_rx.borrow_and_update().clone();
        if let Some(scene) = initial_scene {
            led.on_scene_changed(scene.scene_id);
        }
        let initial_connected = connected_widgets_rx.borrow_and_update().clone();
        led.reconcile_connected(&initial_connected);

        loop {
            let deadline = led.active_deadline();
            let expiry = async move {
                match deadline {
                    Some(d) => sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            };

            tokio::select! {
                biased;

                () = expiry => {
                    led.on_active_expiry();
                }
                action = action_rx.recv() => {
                    let Some(action) = action else {
                        break;
                    };
                    dispatch_widget_action(action, &sound_tx, &mut led);
                }
                changed = active_scene_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let active = active_scene_rx.borrow_and_update().clone();
                    match active {
                        Some(scene) => led.on_scene_changed(scene.scene_id),
                        None => led.on_active_scene_cleared(),
                    }
                }
                changed = connected_widgets_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let connected = connected_widgets_rx.borrow_and_update().clone();
                    led.reconcile_connected(&connected);
                }
                snapshot = scenes_rx.recv() => {
                    match snapshot {
                        Ok(snapshot) => led.on_config_snapshot(snapshot),
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(skipped = n, "action handler scenes_change receiver lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
        info!("action handler shutting down — channel closed");
    });
}

/// Route a single widget action to the sound channel or the LED manager.
fn dispatch_widget_action(
    action: WidgetAction,
    sound_tx: &mpsc::UnboundedSender<SoundCommand>,
    led: &mut LedSceneManager,
) {
    info!(widget = %action.instance_id, "handling widget action");
    match action.payload {
        ActionPayload::PlaySound { sound } => send_sound(sound_tx, SoundCommand::Play(sound)),
        ActionPayload::StopSound {} => send_sound(sound_tx, SoundCommand::Stop),
        ActionPayload::LedTemporary {
            request_id,
            effect,
            color,
            period_ms,
            duration_ms,
            scope,
        } => led.on_temporary(
            action.instance_id,
            request_id,
            effect,
            color,
            period_ms,
            duration_ms,
            scope,
        ),
        ActionPayload::LedEndless {
            request_id,
            effect,
            color,
            period_ms,
            scope,
        } => led.on_endless(
            action.instance_id,
            request_id,
            effect,
            color,
            period_ms,
            scope,
        ),
        ActionPayload::StopLed { request_id } => led.on_stop(&action.instance_id, request_id),
    }
}

enum SoundCommand {
    Play(String),
    Stop,
}

/// Forward a sound command to the sound manager task, logging if the task is
/// gone. The channel is unbounded, so this never drops under load — only a
/// dead manager (closed channel) can fail.
fn send_sound(tx: &mpsc::UnboundedSender<SoundCommand>, cmd: SoundCommand) {
    if tx.send(cmd).is_err() {
        warn!("sound manager gone; dropping sound command");
    }
}

/// Separate task that owns the cancellation token and serializes sound playback.
/// PlaySound cancels any in-progress sound before starting the new one.
/// StopSound cancels without starting a replacement.
fn spawn_sound_manager(mut rx: mpsc::UnboundedReceiver<SoundCommand>, controller: SoundController) {
    tokio::spawn(async move {
        let mut active_token: Option<tokio_util::sync::CancellationToken> = None;

        while let Some(cmd) = rx.recv().await {
            // Cancel any in-progress sound
            if let Some(token) = active_token.take() {
                token.cancel();
            }

            if let SoundCommand::Play(sound_name) = cmd {
                let Ok(sound) = Sounds::from_str(&sound_name) else {
                    warn!(sound = %sound_name, "unknown sound requested by widget");
                    continue;
                };
                let token = tokio_util::sync::CancellationToken::new();
                active_token = Some(token.clone());
                let controller = controller.clone();
                tokio::spawn(async move {
                    if let Err(e) = controller.play_sound(sound, token).await {
                        warn!(error = %e, "failed to play sound for widget action");
                    }
                });
            }
        }
    });
}
