// Copyright (C) 2026  Braiins Systems s.r.o.

//! Routes widget action requests to hardware controllers.

use std::str::FromStr;
use std::time::Duration;

use bmc_led::data::{LedCommand, LedEffect as HwLedEffect, LedScene, Rgb};
use bmc_widget_protocol::ActionPayload;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::compositor::WidgetAction;
use crate::led::LedController;
use crate::sound::{SoundController, Sounds};

/// Channel capacity for sound commands sent to the sound manager task.
const SOUND_CHANNEL_CAPACITY: usize = 4;

/// Spawn a task that receives widget actions from the compositor and dispatches
/// them to the appropriate hardware controller.
///
/// Sound playback is delegated to a separate sound manager task so that
/// long-running `play_sound` calls don't block LED or other action processing.
pub(crate) fn spawn_action_handler<T: crate::BmcManager>(
    mut action_rx: mpsc::UnboundedReceiver<WidgetAction>,
    sound_controller: SoundController,
    led_controller: LedController<T>,
) {
    let (sound_tx, sound_rx) = mpsc::channel(SOUND_CHANNEL_CAPACITY);
    spawn_sound_manager(sound_rx, sound_controller);

    tokio::spawn(async move {
        while let Some(action) = action_rx.recv().await {
            info!(
                widget = %action.instance_id,
                "handling widget action"
            );
            match action.payload {
                ActionPayload::PlaySound { sound } => {
                    let _ = sound_tx.try_send(SoundCommand::Play(sound));
                }
                ActionPayload::StopSound {} => {
                    let _ = sound_tx.try_send(SoundCommand::Stop);
                }
                ActionPayload::LedTemporary {
                    effect,
                    color,
                    period_ms,
                    duration_ms,
                } => {
                    handle_led(
                        &led_controller,
                        effect,
                        color,
                        period_ms,
                        Some(u64::from(duration_ms)),
                    );
                }
                ActionPayload::LedEndless {
                    effect,
                    color,
                    period_ms,
                } => {
                    handle_led(&led_controller, effect, color, period_ms, None);
                }
                ActionPayload::StopLed {} => {
                    led_controller.send_command(LedCommand::SetEffect(LedScene {
                        effect: HwLedEffect::None,
                        period: None,
                        duration: None,
                    }));
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

fn handle_led<T: crate::BmcManager>(
    led_controller: &LedController<T>,
    effect: bmc_widget_protocol::LedEffect,
    color: bmc_widget_protocol::RgbColor,
    period_ms: u32,
    duration: Option<u64>,
) {
    led_controller.send_command(LedCommand::SetEffect(LedScene {
        effect: proto_to_hw_effect(effect, color),
        period: (period_ms > 0).then(|| Duration::from_millis(u64::from(period_ms))),
        duration: duration.map(Duration::from_millis),
    }));
}

/// Wire `LedEffect` is unit-typed and the color travels separately, but the
/// hardware enum folds the color into each variant. Centralized here so any
/// future call site reuses the mapping instead of duplicating the match.
fn proto_to_hw_effect(
    effect: bmc_widget_protocol::LedEffect,
    color: bmc_widget_protocol::RgbColor,
) -> HwLedEffect {
    let rgb = Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    };
    match effect {
        bmc_widget_protocol::LedEffect::Chase => HwLedEffect::Chase(rgb),
        bmc_widget_protocol::LedEffect::KnightRider => HwLedEffect::KnightRider(rgb),
        bmc_widget_protocol::LedEffect::Scan => HwLedEffect::Scan(rgb),
        bmc_widget_protocol::LedEffect::Snake => HwLedEffect::Snake(rgb),
        bmc_widget_protocol::LedEffect::Breathe => HwLedEffect::Breathe(rgb),
        bmc_widget_protocol::LedEffect::Solid => HwLedEffect::Solid(rgb),
    }
}
