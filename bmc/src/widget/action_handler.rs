// Copyright (C) 2026  Braiins Systems s.r.o.

//! Routes widget action requests to hardware controllers.
//!
//! `LedTemporary` requests pass through a FIFO queue: the driver's
//! single temporary slot would otherwise let one widget clobber another
//! mid-flight. `LedEndless` bypasses the queue — the driver's persistent
//! slot already arbitrates against the temporary one.

use std::collections::VecDeque;
use std::str::FromStr;
use std::time::Duration;

use bmc_led::data::{LedCommand, LedEffect as HwLedEffect, LedScene, Rgb};
use bmc_widget_protocol::{
    ActionPayload, LED_REQUEST_ID_ALL, LedEffect as ProtoLedEffect, LedRequestId, LedRequestStatus,
    RgbColor,
};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tracing::{info, warn};

use crate::compositor::{InstanceId, LedRequestStatusEvent, WidgetAction};
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
    status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    sound_controller: SoundController,
    led_controller: LedController<T>,
) {
    let (sound_tx, sound_rx) = mpsc::channel(SOUND_CHANNEL_CAPACITY);
    spawn_sound_manager(sound_rx, sound_controller);

    tokio::spawn(async move {
        let mut led = LedQueueState::new(led_controller, status_tx);

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
                        } => {
                            led.on_temporary(
                                action.instance_id,
                                request_id,
                                effect,
                                color,
                                period_ms,
                                duration_ms,
                            );
                        }
                        ActionPayload::LedEndless {
                            request_id,
                            effect,
                            color,
                            period_ms,
                        } => {
                            led.on_endless(
                                action.instance_id,
                                request_id,
                                effect,
                                color,
                                period_ms,
                            );
                        }
                        ActionPayload::StopLed { request_id } => {
                            led.on_stop(&action.instance_id, request_id);
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

#[derive(Debug)]
struct TempEntry {
    instance_id: InstanceId,
    request_id: LedRequestId,
    scene: LedScene,
    duration: Duration,
}

#[derive(Debug)]
struct ActiveTemp {
    instance_id: InstanceId,
    request_id: LedRequestId,
    /// Deadline mirrors the driver's own temporary slot, so on advance
    /// we don't need to clear anything — the driver expires it for us.
    until: Instant,
}

#[derive(Debug)]
struct ActiveEndless {
    instance_id: InstanceId,
    request_id: LedRequestId,
}

struct LedQueueState<T: crate::BmcManager> {
    led_controller: LedController<T>,
    status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    queue: VecDeque<TempEntry>,
    active_temp: Option<ActiveTemp>,
    active_endless: Option<ActiveEndless>,
}

impl<T: crate::BmcManager> LedQueueState<T> {
    fn new(
        led_controller: LedController<T>,
        status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    ) -> Self {
        Self {
            led_controller,
            status_tx,
            queue: VecDeque::new(),
            active_temp: None,
            active_endless: None,
        }
    }

    fn active_deadline(&self) -> Option<Instant> {
        self.active_temp.as_ref().map(|a| a.until)
    }

    fn on_active_expiry(&mut self) {
        let Some(active) = self.active_temp.take() else {
            return;
        };
        self.emit(
            active.instance_id,
            active.request_id,
            LedRequestStatus::Completed,
        );
        self.advance_queue();
    }

    fn on_temporary(
        &mut self,
        instance_id: InstanceId,
        request_id: LedRequestId,
        effect: ProtoLedEffect,
        color: RgbColor,
        period_ms: u32,
        duration_ms: u32,
    ) {
        let scene = build_scene(effect, color, period_ms, Some(u64::from(duration_ms)));
        let duration = Duration::from_millis(u64::from(duration_ms));
        self.emit(instance_id.clone(), request_id, LedRequestStatus::Accepted);
        let entry = TempEntry {
            instance_id,
            request_id,
            scene,
            duration,
        };
        if self.active_temp.is_none() {
            self.start_temp(entry);
        } else {
            self.queue.push_back(entry);
        }
    }

    fn on_endless(
        &mut self,
        instance_id: InstanceId,
        request_id: LedRequestId,
        effect: ProtoLedEffect,
        color: RgbColor,
        period_ms: u32,
    ) {
        let scene = build_scene(effect, color, period_ms, None);
        if let Some(prev) = self.active_endless.take() {
            self.emit(
                prev.instance_id,
                prev.request_id,
                LedRequestStatus::Superseded,
            );
        }
        self.led_controller
            .send_command(LedCommand::SetEffect(scene));
        self.emit(instance_id.clone(), request_id, LedRequestStatus::Accepted);
        self.active_endless = Some(ActiveEndless {
            instance_id,
            request_id,
        });
    }

    /// Cancelling the active temporary advances our bookkeeping but
    /// does *not* interrupt the driver — `LedCommand` has no "clear
    /// temporary" today, so the strip keeps the cancelled effect until
    /// its natural duration elapses. Cancelling the active endless
    /// overwrites the driver's persistent slot with `None`.
    fn on_stop(&mut self, instance_id: &str, request_id: LedRequestId) {
        let cancel_all = request_id == LED_REQUEST_ID_ALL;
        let matches = |stored_instance: &str, stored_id: LedRequestId| -> bool {
            stored_instance == instance_id && (cancel_all || stored_id == request_id)
        };

        let mut to_supersede: Vec<(InstanceId, LedRequestId)> = Vec::new();

        if self
            .active_temp
            .as_ref()
            .is_some_and(|a| matches(&a.instance_id, a.request_id))
        {
            let active = self
                .active_temp
                .take()
                .expect("BUG: just verified active_temp matches");
            to_supersede.push((active.instance_id, active.request_id));
            self.advance_queue();
        }

        let kept: VecDeque<TempEntry> = std::mem::take(&mut self.queue)
            .into_iter()
            .filter_map(|entry| {
                if matches(&entry.instance_id, entry.request_id) {
                    to_supersede.push((entry.instance_id, entry.request_id));
                    None
                } else {
                    Some(entry)
                }
            })
            .collect();
        self.queue = kept;

        if self
            .active_endless
            .as_ref()
            .is_some_and(|e| matches(&e.instance_id, e.request_id))
        {
            let endless = self
                .active_endless
                .take()
                .expect("BUG: just verified active_endless matches");
            to_supersede.push((endless.instance_id, endless.request_id));
            self.led_controller
                .send_command(LedCommand::SetEffect(LedScene {
                    effect: HwLedEffect::None,
                    period: None,
                    duration: None,
                }));
        }

        for (instance_id, request_id) in to_supersede {
            self.emit(instance_id, request_id, LedRequestStatus::Superseded);
        }
    }

    fn advance_queue(&mut self) {
        if let Some(next) = self.queue.pop_front() {
            self.start_temp(next);
        }
    }

    fn start_temp(&mut self, entry: TempEntry) {
        self.led_controller
            .send_command(LedCommand::SetEffect(entry.scene));
        let until = Instant::now() + entry.duration;
        self.active_temp = Some(ActiveTemp {
            instance_id: entry.instance_id,
            request_id: entry.request_id,
            until,
        });
    }

    fn emit(&self, instance_id: InstanceId, request_id: LedRequestId, status: LedRequestStatus) {
        let _ = self.status_tx.send(LedRequestStatusEvent {
            instance_id,
            request_id,
            status,
        });
    }
}

fn build_scene(
    effect: ProtoLedEffect,
    color: RgbColor,
    period_ms: u32,
    duration_ms: Option<u64>,
) -> LedScene {
    LedScene {
        effect: proto_to_hw_effect(effect, color),
        period: (period_ms > 0).then(|| Duration::from_millis(u64::from(period_ms))),
        duration: duration_ms.map(Duration::from_millis),
    }
}

/// Wire `LedEffect` is unit-typed and the color travels separately, but the
/// hardware enum folds the color into each variant. Centralized here so any
/// future call site reuses the mapping instead of duplicating the match.
fn proto_to_hw_effect(effect: ProtoLedEffect, color: RgbColor) -> HwLedEffect {
    let rgb = Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    };
    match effect {
        ProtoLedEffect::Chase => HwLedEffect::Chase(rgb),
        ProtoLedEffect::KnightRider => HwLedEffect::KnightRider(rgb),
        ProtoLedEffect::Scan => HwLedEffect::Scan(rgb),
        ProtoLedEffect::Snake => HwLedEffect::Snake(rgb),
        ProtoLedEffect::Breathe => HwLedEffect::Breathe(rgb),
        ProtoLedEffect::Solid => HwLedEffect::Solid(rgb),
    }
}
