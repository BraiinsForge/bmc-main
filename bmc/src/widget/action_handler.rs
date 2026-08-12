// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Routes widget action requests to hardware controllers.
//!
use std::collections::BTreeSet;
use std::str::FromStr;

use bmc_widget_protocol::ActionPayload;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::sleep_until;
use tracing::{info, warn};

use super::led_state::LedSceneManager;
use crate::backlight::MIN_BRIGHTNESS_PCT;
use crate::compositor::{
    ActiveScene, Compositor, LedRequestStatusEvent, SettingsCommand, WidgetAction,
};
use crate::led_coordinator::LedCoordinatorHandle;
use crate::manager::BmcManager;
use crate::sound::{SoundController, Sounds};
use crate::system_manager::SystemManager;
use crate::system_upgrade::SystemUpgradeState;

/// Compositor-facing channels the action handler owns for its lifetime.
pub(crate) struct CompositorIo {
    pub action_rx: mpsc::UnboundedReceiver<WidgetAction>,
    pub settings_rx: mpsc::UnboundedReceiver<SettingsCommand>,
    pub active_scene_rx: watch::Receiver<Option<ActiveScene>>,
    pub connected_widgets_rx: watch::Receiver<BTreeSet<crate::compositor::InstanceId>>,
    pub status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    pub night_mode_active_rx: watch::Receiver<bool>,
    pub upgrade_state_rx: watch::Receiver<Option<SystemUpgradeState>>,
}

/// Spawn a task that receives widget actions from the compositor and
/// dispatches them to the appropriate hardware controller.
///
/// Sound playback runs on a dedicated task — `play_sound` blocks until
/// the clip ends, and we don't want that to stall LED processing.
#[expect(
    clippy::too_many_arguments,
    reason = "the handler collects independent subsystem handles; bundling them \
              into a wrapper struct just to satisfy the lint would hurt clarity"
)]
pub(crate) fn spawn_action_handler<T, U>(
    io: CompositorIo,
    sound_controller: SoundController,
    led_coordinator: LedCoordinatorHandle,
    initial_widget_scene_map: crate::config::WidgetSceneMap,
    mut scenes_rx: broadcast::Receiver<crate::config::WidgetSceneMap>,
    system_manager: SystemManager<U>,
    manager: std::sync::Arc<T>,
    compositor: std::sync::Arc<dyn Compositor>,
) where
    T: BmcManager,
    U: crate::backlight::DisplayBacklightDriver,
{
    let CompositorIo {
        mut action_rx,
        mut settings_rx,
        mut active_scene_rx,
        mut connected_widgets_rx,
        status_tx,
        night_mode_active_rx,
        upgrade_state_rx,
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
                cmd = settings_rx.recv() => {
                    let Some(cmd) = cmd else {
                        break;
                    };
                    dispatch_settings_command(
                        cmd,
                        &system_manager,
                        &manager,
                        &night_mode_active_rx,
                        &upgrade_state_rx,
                        &compositor,
                    )
                    .await;
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

/// Dispatch a settings command to the appropriate system handle.
///
/// `SystemManager::set_brightness` persists config AND notifies the
/// physical-backlight loop; a bare `set_config_brightness` would update
/// config without changing the actual backlight. Clamp to the product minimum.
///
/// When night mode is active the tray shows the night brightness, so a write
/// must persist to the night config via `set_night_mode_brightness`; otherwise
/// it persists to the day config via `set_brightness`. The same day/night split
/// applies to volume via `set_sound_volume_night_mode` / `set_sound_volume`.
async fn dispatch_settings_command<T, U>(
    cmd: SettingsCommand,
    system_manager: &SystemManager<U>,
    manager: &std::sync::Arc<T>,
    night_mode_active_rx: &watch::Receiver<bool>,
    upgrade_state_rx: &watch::Receiver<Option<SystemUpgradeState>>,
    compositor: &std::sync::Arc<dyn Compositor>,
) where
    T: BmcManager,
    U: crate::backlight::DisplayBacklightDriver,
{
    match cmd {
        SettingsCommand::SetBrightness(value) => {
            let v = value.clamp(MIN_BRIGHTNESS_PCT, 100);
            let result = if *night_mode_active_rx.borrow() {
                system_manager.set_night_mode_brightness(v).await
            } else {
                system_manager.set_brightness(v).await
            };
            if let Err(e) = result {
                warn!("settings overlay set_brightness failed: {e}");
            }
        }
        SettingsCommand::SetVolume(value) => {
            let v = value.min(100);
            let result = if *night_mode_active_rx.borrow() {
                system_manager.set_sound_volume_night_mode(v).await
            } else {
                system_manager.set_sound_volume(v).await
            };
            if let Err(e) = result {
                warn!("settings overlay set_volume failed: {e}");
            }
        }
        SettingsCommand::ToggleNightMode => {
            if let Err(e) = system_manager.toggle_night_mode().await {
                warn!("settings overlay toggle_night_mode failed: {e}");
            }
        }
        SettingsCommand::Restart => {
            let blocked = upgrade_state_rx
                .borrow()
                .as_ref()
                .is_some_and(SystemUpgradeState::blocks_restart);
            if blocked {
                info!("settings overlay restart declined: upgrade in progress");
                if let Err(e) = compositor.broadcast_restart_declined("upgrade in progress") {
                    warn!("broadcast_restart_declined failed: {e}");
                }
            } else if let Err(e) = manager.reboot().await {
                // Never leave the overlay hanging in its pending state: a
                // failed reboot call must surface as a decline.
                warn!("settings overlay restart failed: {e:#}");
                if let Err(e) = compositor.broadcast_restart_declined("restart failed") {
                    warn!("broadcast_restart_declined failed: {e}");
                }
            }
        }
        SettingsCommand::ReconfigureWifi => {
            if let Some(wifi) = manager.network_manager().wifi() {
                if let Err(e) = wifi.enter_wifi_reconfiguration().await {
                    warn!("settings overlay reconfigure_wifi failed: {e}");
                }
            } else {
                warn!("settings overlay reconfigure_wifi: no WiFi on this platform");
            }
        }
    }
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
