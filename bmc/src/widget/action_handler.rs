// Copyright (C) 2026  Braiins Systems s.r.o.

//! Routes widget action requests to hardware controllers.
//!
use std::str::FromStr;

use bmc_widget_protocol::ActionPayload;
use tokio::sync::{broadcast, mpsc};
use tokio::time::sleep_until;
use tracing::{info, warn};

use super::led_state::LedSceneManager;
use crate::compositor::{CompositorEvent, WidgetAction, WidgetRequestStatus};
use crate::led::LedController;
use crate::sound::{SoundController, Sounds};

/// Channel capacity for sound commands sent to the sound manager task.
const SOUND_CHANNEL_CAPACITY: usize = 4;

/// Spawn a task that receives widget actions from the compositor and
/// dispatches them to the appropriate hardware controller.
///
/// Sound playback runs on a dedicated task — `play_sound` blocks until
/// the clip ends, and we don't want that to stall LED processing.
pub(crate) fn spawn_action_handler<T: crate::BmcManager>(
    mut action_rx: mpsc::UnboundedReceiver<WidgetAction>,
    mut event_rx: broadcast::Receiver<CompositorEvent>,
    status_tx: mpsc::UnboundedSender<WidgetRequestStatus>,
    sound_controller: SoundController,
    led_controller: LedController<T>,
) {
    let (sound_tx, sound_rx) = mpsc::channel(SOUND_CHANNEL_CAPACITY);
    spawn_sound_manager(sound_rx, sound_controller);

    tokio::spawn(async move {
        let mut led = LedSceneManager::new(led_controller, status_tx);

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
                    info!(widget = %action.instance_id, "handling widget action");
                    match action.payload {
                        ActionPayload::PlaySound { sound } => {
                            let _ = sound_tx.try_send(SoundCommand::Play(sound));
                        }
                        ActionPayload::StopSound {} => {
                            let _ = sound_tx.try_send(SoundCommand::Stop);
                        }
                        ActionPayload::LedTemporary {
                            request_id,
                            effect,
                            color,
                            period_ms,
                            duration_ms,
                            scope,
                        } => {
                            led.on_temporary(
                                action.instance_id,
                                request_id,
                                effect,
                                color,
                                period_ms,
                                duration_ms,
                                scope,
                            );
                        }
                        ActionPayload::LedEndless {
                            request_id,
                            effect,
                            color,
                            period_ms,
                            scope,
                        } => {
                            led.on_endless(
                                action.instance_id,
                                request_id,
                                effect,
                                color,
                                period_ms,
                                scope,
                            );
                        }
                        ActionPayload::StopLed { request_id } => {
                            led.on_stop(&action.instance_id, request_id);
                        }
                    }
                }
                event = event_rx.recv() => {
                    match event {
                        Ok(CompositorEvent::ActiveSceneChanged { scene_id, widget_ids }) => {
                            led.on_scene_changed(scene_id, widget_ids);
                        }
                        Ok(CompositorEvent::WidgetDisconnected { instance_id }) => {
                            led.on_widget_disconnected(&instance_id);
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(skipped = n, "action handler compositor event receiver lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }
        info!("action handler shutting down — channel closed");
    });
}

enum SoundCommand {
    Play(String),
    Stop,
}

/// Separate task that owns the cancellation token and serializes sound playback.
/// PlaySound cancels any in-progress sound before starting the new one.
/// StopSound cancels without starting a replacement.
fn spawn_sound_manager(mut rx: mpsc::Receiver<SoundCommand>, controller: SoundController) {
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
