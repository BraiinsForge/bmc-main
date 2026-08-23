// Copyright (C) 2025  Braiins Systems s.r.o.
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

use bmc_shared_time::time::Timezone;
use bmc_widget_manifest::{ParamKey, ParamValue};
use bmc_widget_protocol::{
    Localization, NextAlarm, SettingUpdate, ViewportShape, WidgetInitialConfig,
};
use indexmap::IndexMap;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;
#[cfg(test)]
use tokio::sync::mpsc;
use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tracing::{debug, info, warn};

use crate::BmcManager;
use crate::compositor::{
    Compositor, CompositorError, HardwareCapabilities, Position, SceneLayout, Size,
    WidgetConnectionMode, WidgetGeneration, WidgetInstanceKey, WidgetPlacement, WidgetRegistration,
};
use crate::config::ConfigHandle;
use crate::config::LocalizationConfig;
use crate::credential;
use crate::scene::{Scene, SceneId, Widget, WidgetPosition};
use crate::secret_store::SecretStoreHandle;
use bmc_net::NetworkManager;

#[cfg(test)]
use super::WidgetEvent;
use super::manager::{ManagedWidgetState, ManagerMode, StartError, WidgetLaunch};
use super::{WidgetManager, WidgetRegistry};

#[must_use]
pub(crate) fn fullscreen_descriptor_for_widget(
    widget: &Widget,
    caps: &HardwareCapabilities,
) -> crate::widget::ViewportDescriptor {
    crate::widget::ViewportDescriptor {
        viewport_shape: widget.viewport_shape,
        width: caps.display.width,
        height: caps.display.height,
        dpi: caps.display.dpi,
    }
}

fn placement_viewport_size(
    placement: &crate::scene::WidgetPlacement,
    fullscreen_descriptor: &crate::widget::ViewportDescriptor,
) -> Option<Size> {
    let desc = match placement {
        crate::scene::WidgetPlacement::Fullscreen => *fullscreen_descriptor,
        crate::scene::WidgetPlacement::SlotSpan(s) => {
            crate::widget::slot_span_descriptor(s.columns, s.rows)?
        }
    };
    Some(Size {
        width: desc.width,
        height: desc.height,
    })
}

fn placement_viewport_descriptor(
    widget: &Widget,
    caps: &HardwareCapabilities,
) -> Option<crate::widget::ViewportDescriptor> {
    match &widget.placement {
        crate::scene::WidgetPlacement::Fullscreen => {
            Some(fullscreen_descriptor_for_widget(widget, caps))
        }
        crate::scene::WidgetPlacement::SlotSpan(span) => {
            crate::widget::slot_span_descriptor(span.columns, span.rows)
        }
    }
}

/// Path-safe placement tag for the opaque instance token: `full` or `<cols>x<rows>`.
fn placement_tag(placement: &crate::scene::WidgetPlacement) -> String {
    match placement {
        crate::scene::WidgetPlacement::Fullscreen => "full".to_owned(),
        crate::scene::WidgetPlacement::SlotSpan(s) => format!("{}x{}", s.columns, s.rows),
    }
}

fn manifest_to_protocol_viewport_shape(shape: bmc_widget_manifest::ViewportShape) -> ViewportShape {
    match shape {
        bmc_widget_manifest::ViewportShape::Rectangular => ViewportShape::Rectangular,
        bmc_widget_manifest::ViewportShape::Round => ViewportShape::Round,
    }
}

fn platform_to_protocol_display(
    display: bmc_platform::DisplayInfo,
) -> bmc_widget_protocol::DisplayInfo {
    bmc_widget_protocol::DisplayInfo {
        width: display.width,
        height: display.height,
        shape: match display.shape {
            bmc_platform::DisplayShape::Rectangular => {
                bmc_widget_protocol::DisplayShape::Rectangular
            }
            bmc_platform::DisplayShape::Round => bmc_widget_protocol::DisplayShape::Round,
        },
        dpi: display.dpi,
    }
}

pub fn supported_scenes<'a>(
    registry: &WidgetRegistry,
    caps: &HardwareCapabilities,
    scenes: &'a IndexMap<SceneId, Scene>,
) -> Vec<&'a Scene> {
    scenes
        .values()
        .filter(|s| s.enabled && scene_supported_with_registry(registry, s, caps))
        .collect()
}

pub fn first_supported_active_scene<'a>(
    registry: &WidgetRegistry,
    caps: &HardwareCapabilities,
    scenes: &'a IndexMap<SceneId, Scene>,
) -> Option<&'a Scene> {
    scenes
        .values()
        .filter(|s| s.enabled)
        .find(|s| scene_supported_with_registry(registry, s, caps))
}

pub fn scene_supported_with_registry(
    registry: &WidgetRegistry,
    scene: &Scene,
    caps: &HardwareCapabilities,
) -> bool {
    match scene.kind {
        crate::scene::SceneKind::Combined => {
            if caps.slot_grid.is_none() {
                return false;
            }
            scene.widgets.values().all(|w| match &w.placement {
                crate::scene::WidgetPlacement::Fullscreen => false,
                crate::scene::WidgetPlacement::SlotSpan(s) => {
                    crate::widget::slot_span_descriptor(s.columns, s.rows).is_some()
                }
            })
        }
        crate::scene::SceneKind::Fullscreen => scene.widgets.values().next().is_some_and(|w| {
            let descriptor = fullscreen_descriptor_for_widget(w, caps);
            registry.supports_viewport(&w.widget_type_id, &descriptor)
        }),
    }
}

/// Build the compositor layout for a scene. Widgets the spawner can never
/// launch — nil or unregistered `widget_type_id`, the same conditions
/// `spawn_widget` skips or fails on — stay out of the layout, so their grid
/// cells render empty instead of leaving the scene stuck on the renderer's
/// "Loading scene…" placeholder waiting for a paint that cannot come.
fn scene_to_layout_with_registry(
    registry: &WidgetRegistry,
    caps: &HardwareCapabilities,
    scene: &Scene,
) -> SceneLayout {
    let widgets = scene
        .widgets
        .values()
        .filter_map(|widget| {
            if widget.widget_type_id.is_nil() || registry.get(&widget.widget_type_id).is_none() {
                debug!(
                    widget_id = %widget.id.as_uuid(),
                    widget_type = %widget.widget_type_id,
                    "skipping unspawnable widget in layout"
                );
                return None;
            }
            let fullscreen_descriptor = fullscreen_descriptor_for_widget(widget, caps);
            let Some(size) = placement_viewport_size(&widget.placement, &fullscreen_descriptor)
            else {
                debug!(
                    placement = ?widget.placement,
                    "skipping unsupported widget placement in layout"
                );
                return None;
            };
            Some(WidgetPlacement {
                instance_id: widget.id.as_uuid().to_string(),
                position: widget_to_position(caps, widget),
                size,
                visible: true,
            })
        })
        .collect();

    SceneLayout {
        scene_id: Some(scene.id),
        cycle_duration: scene.cycle_duration,
        combined: scene.kind == crate::scene::SceneKind::Combined,
        widgets,
    }
}

fn widget_to_position(caps: &HardwareCapabilities, widget: &Widget) -> Position {
    // Snap the grid cell to its logical-pixel origin. The pitch bakes in a
    // 4px separator gap (see `WidgetPosition::col_pitch`) and is derived
    // from this product's logical panel size, so widgets sit flush with
    // uniform gaps between them.
    let display = &caps.display;
    Position {
        x: u32::from(widget.position.col) * WidgetPosition::col_pitch(display.width),
        y: u32::from(widget.position.row) * WidgetPosition::row_pitch(display.height),
    }
}

fn widget_launch(scene_id: SceneId, widget: &Widget, wayland_display: String) -> WidgetLaunch {
    WidgetLaunch::new(
        scene_id,
        widget.id.as_uuid(),
        widget.widget_type_id,
        wayland_display,
    )
}

fn widget_configuration_unchanged(current: &Widget, expected: &Widget) -> bool {
    current.id == expected.id
        && current.position == expected.position
        && current.placement == expected.placement
        && current.widget_type_id == expected.widget_type_id
        && current.viewport_shape == expected.viewport_shape
        && current.params == expected.params
        && current.credential_bindings == expected.credential_bindings
}

pub struct Coordinator {
    widget_manager: WidgetManager,
    compositor: Arc<dyn Compositor>,
    widget_registry: Arc<WidgetRegistry>,
    hardware_capabilities: HardwareCapabilities,
    /// Read on every spawn to resolve the widget's credential bindings.
    /// Lock order: config before secrets. A spawner takes the config lock
    /// first or not at all, and never takes it while holding this one.
    secret_store: Arc<RwLock<SecretStoreHandle>>,
    spawn_records: StdRwLock<HashMap<String, SpawnRecord>>,
    /// Source of the [`WidgetGeneration`] stamped on each registration.
    /// Coordinator-owned: it is the only place that reaches the compositor
    /// and the manager, which have to agree on the value.
    next_generation: AtomicU64,
}

#[derive(Clone)]
pub(crate) enum ConfiguredSceneState {
    Enabled,
    Preview(Arc<Mutex<Option<SceneId>>>),
}

#[derive(Clone, Copy)]
enum RetainedWidgetStop {
    Deactivate,
    Unregister,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpawnRecord {
    scene_id: SceneId,
    widget_uid: uuid::Uuid,
}

fn current_widget_for_record(
    scenes: &IndexMap<SceneId, Scene>,
    instance_id: &str,
    record: &SpawnRecord,
) -> Option<Widget> {
    scenes
        .get(&record.scene_id)?
        .widgets
        .values()
        .find(|widget| {
            widget.id.to_string() == instance_id && widget.widget_type_id == record.widget_uid
        })
        .cloned()
}

impl std::fmt::Debug for Coordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coordinator")
            .field("widget_manager", &self.widget_manager)
            .finish_non_exhaustive()
    }
}

/// Re-resolve every configured widget's credentials whenever a binding
/// or an account changes, and push the result to the compositor.
///
/// Both wakes are bare hints: a scene save fires for any edit,
/// not just a rebind, and an account save fires for any account.
///
/// Deliberately unfiltered by liveness: a registered widget
/// whose surface has not attached yet must still see the change,
/// because the handshake replays the stored config —
/// and the compositor skips an instance it holds no record of.
///
/// Re-resolving is cheap and a no-change push is dropped,
/// so the fan-out itself costs nothing.
/// Re-arming a pending respawn is not free — it resets the tempo
/// the backoff ceiling bounds — so it happens only on a real change.
pub fn start_credential_listener(
    coordinator: Arc<Coordinator>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    mut scenes_change_rx: broadcast::Receiver<crate::config::WidgetSceneMap>,
    mut accounts_change_rx: broadcast::Receiver<()>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                res = scenes_change_rx.recv() => match res {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                res = accounts_change_rx.recv() => match res {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }

            let config = config_handle.read().await;
            for widget in config
                .scenes()
                .values()
                .flat_map(|scene| scene.widgets.values())
            {
                if let Err(err) = coordinator.update_widget_credentials(widget).await {
                    warn!(widget_id = %widget.id, error = %err, "failed to push credentials to widget");
                }
            }
        }
    });
}

#[cfg(test)]
async fn apply_widget_events(
    compositor: Arc<dyn Compositor>,
    mut events: mpsc::UnboundedReceiver<WidgetEvent>,
) {
    while let Some(event) = events.recv().await {
        match event {
            WidgetEvent::Exited {
                instance_id,
                generation,
                pid,
            } => {
                if let Err(e) = compositor.clear_pid(&instance_id, generation, pid) {
                    warn!(
                        widget_id = %instance_id,
                        pid,
                        error = %e,
                        "failed to detach the pid of an exited widget"
                    );
                }
            }
            WidgetEvent::Respawned {
                instance_id,
                generation,
                pid,
            } => {
                if let Err(e) = compositor.bind_respawned_pid(&instance_id, generation, pid) {
                    warn!(
                        widget_id = %instance_id,
                        pid,
                        error = %e,
                        "failed to bind the new pid after a widget respawn"
                    );
                }
            }
            WidgetEvent::Abandoned {
                instance_id,
                generation,
            } => {
                if let Err(e) = compositor.unregister_abandoned(&instance_id, generation) {
                    warn!(
                        widget_id = %instance_id,
                        error = %e,
                        "failed to end the registration of an abandoned widget"
                    );
                }
            }
        }
    }
}

/// Broadcast the effective display brightness to the settings-tray overlay
/// whenever it changes (a manual change or a night-mode transition). Effective
/// brightness mirrors `system_manager::set_current_brightness`: the night-mode
/// percentage while night mode is active, else the configured percentage.
pub fn start_brightness_listener(
    compositor: Arc<dyn Compositor>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    mut brightness_change_rx: broadcast::Receiver<()>,
    mut night_mode_active_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            let night_active = *night_mode_active_rx.borrow_and_update();
            let brightness = {
                let cfg = config_handle.read().await;
                if night_active {
                    cfg.night_mode().brightness_pct
                } else {
                    cfg.brightness_pct()
                }
            };
            if let Err(e) = compositor.broadcast_brightness(brightness) {
                warn!("broadcast_brightness failed: {e}");
            }
            tokio::select! {
                r = brightness_change_rx.recv() => match r {
                    // Lagged is recoverable (we just missed some notifications);
                    // re-broadcast the current value rather than killing the
                    // listener. Only Closed is terminal.
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                r = night_mode_active_rx.changed() => if r.is_err() { break },
            }
        }
    });
}

/// Broadcast the effective sound volume to the settings-tray overlay whenever
/// it changes (a manual change, a gRPC write, or a night-mode transition).
/// Effective volume is the night-mode percentage while night mode is active,
/// else the configured percentage.
pub fn start_volume_listener(
    compositor: Arc<dyn Compositor>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    mut sound_change_rx: broadcast::Receiver<()>,
    mut night_mode_active_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            let night_active = *night_mode_active_rx.borrow_and_update();
            let volume = {
                let cfg = config_handle.read().await;
                if night_active {
                    cfg.night_mode().sound_volume_pct
                } else {
                    cfg.sound_volume_pct()
                }
            };
            if let Err(e) = compositor.broadcast_volume(volume) {
                warn!("broadcast_volume failed: {e}");
            }
            tokio::select! {
                r = sound_change_rx.recv() => match r {
                    // Lagged is recoverable (we just missed some notifications);
                    // re-broadcast the current value rather than killing the
                    // listener. Only Closed is terminal.
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                r = night_mode_active_rx.changed() => if r.is_err() { break },
            }
        }
    });
}

/// Broadcast the night-mode state and its "HH:MM" boundary to the settings-tray
/// overlay. Recomputes on EVERY watch notification, not only on active-state
/// flips: the controller's set_enabled/set_interval send_replace the watch
/// unconditionally, so schedule edits refresh the boundary too.
///
/// Overlay state only. Night mode's effect on scene cycling is owned by the
/// listener in `startup`, which needs edge detection this loop deliberately
/// does not do.
pub fn start_night_mode_listener<U>(
    compositor: Arc<dyn Compositor>,
    system_manager: crate::system_manager::SystemManager<U>,
) where
    U: crate::backlight::DisplayBacklightDriver,
{
    let mut night_mode_rx = system_manager.subscribe_night_mode();
    tokio::spawn(async move {
        loop {
            let active = *night_mode_rx.borrow_and_update();
            let config = system_manager.night_mode_config().await;
            let until = if active {
                Some(config.to.format("%H:%M").to_string())
            } else if config.enabled {
                Some(config.from.format("%H:%M").to_string())
            } else {
                None
            };
            if let Err(e) = compositor.broadcast_night_mode(active, until.as_deref()) {
                warn!("broadcast_night_mode failed: {e}");
            }
            if night_mode_rx.changed().await.is_err() {
                break;
            }
        }
    });
}

/// Number of times [`resolve_setup_ap_ssid`] asks for the setup AP SSID.
const SETUP_AP_SSID_ATTEMPTS: u8 = 5;

/// Delay between [`resolve_setup_ap_ssid`] attempts.
const SETUP_AP_SSID_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Resolves the setup AP SSID, retrying briefly while it is still unset.
///
/// The listener only wakes on a setup-mode transition, so a single `None`
/// would leave the tray showing its previous value indefinitely. The manager
/// publishes setup mode only once the AP is up, making `None` unlikely; this
/// retry covers the remaining window where the UCI section is not yet
/// readable, so a slow AP degrades into a short delay rather than a stale tray.
async fn resolve_setup_ap_ssid(network_manager: &dyn NetworkManager) -> Option<String> {
    for attempt in 1..=SETUP_AP_SSID_ATTEMPTS {
        match network_manager.require_wifi() {
            Ok(wifi) => {
                if let Some(ssid) = wifi.ap_ssid().await {
                    return Some(ssid);
                }
            }
            Err(e) => {
                warn!("WiFi unsupported while in setup mode: {e}");
                return None;
            }
        }

        if attempt < SETUP_AP_SSID_ATTEMPTS {
            debug!("setup AP SSID not readable yet; attempt {attempt}/{SETUP_AP_SSID_ATTEMPTS}");
            tokio::time::sleep(SETUP_AP_SSID_RETRY_DELAY).await;
        }
    }

    None
}

/// Broadcast the WiFi setup-AP SSID to the settings-tray overlay on every
/// setup-mode transition. Resolves the SSID via the manager while in setup mode;
/// `None` clears the overlay back to idle.
pub fn start_wifi_reconfig_listener<M: BmcManager>(
    compositor: Arc<dyn Compositor>,
    manager: Arc<M>,
    mut reconfig_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            let in_setup = *reconfig_rx.borrow_and_update();
            if in_setup {
                // Deliberately `ap_ssid`, not `ssid`: the latter falls back to
                // the joined station network, and during the window in which
                // the setup AP is still coming up that would tell the user to
                // join the network the device was already on.
                let ssid = resolve_setup_ap_ssid(manager.network_manager()).await;
                if let Some(ssid) = ssid {
                    if let Err(e) = compositor.broadcast_wifi_ap(Some(ssid)) {
                        warn!("broadcast_wifi_ap failed: {e}");
                    }
                } else {
                    // No AP SSID must not masquerade as "setup inactive" (which
                    // an empty SSID signals): keep the overlay's last-known
                    // value until the next real transition.
                    warn!("no setup AP SSID available; keeping last SSID");
                }
            } else if let Err(e) = compositor.broadcast_wifi_ap(None) {
                warn!("broadcast_wifi_ap failed: {e}");
            }
            if reconfig_rx.changed().await.is_err() {
                break;
            }
        }
    });
}

impl Coordinator {
    pub fn new(
        widget_manager: WidgetManager,
        compositor: Arc<dyn Compositor>,
        widget_registry: Arc<WidgetRegistry>,
        hardware_capabilities: HardwareCapabilities,
        secret_store: Arc<RwLock<SecretStoreHandle>>,
    ) -> Self {
        Self {
            widget_manager,
            compositor,
            widget_registry,
            hardware_capabilities,
            secret_store,
            spawn_records: StdRwLock::new(HashMap::new()),
            next_generation: AtomicU64::new(0),
        }
    }

    fn next_generation(&self) -> WidgetGeneration {
        WidgetGeneration(self.next_generation.fetch_add(1, Ordering::Relaxed))
    }

    /// Subscribe the coordinator to runtime setting changes and forward
    /// each one to the compositor's `broadcast_setting`.
    ///
    /// Three upstream sources feed in, each with its own canonical
    /// channel — the coordinator is the translation layer that maps
    /// each one onto the wire-level [`SettingUpdate`] enum.
    ///
    /// All three subscriptions live for the lifetime of the program;
    /// if any upstream channel closes the listener exits.
    pub fn start_settings_listener(
        compositor: Arc<dyn Compositor>,
        mut localization_rx: broadcast::Receiver<LocalizationConfig>,
        mut night_mode_active_rx: watch::Receiver<bool>,
        mut timezone_rx: watch::Receiver<Timezone>,
        mut next_alarm_rx: watch::Receiver<Option<NextAlarm>>,
    ) {
        // `mark_changed` forces the first `.changed()` to fire
        // with the current value so the compositor cache sees
        // the bootstrap state even when the sender wrote
        // before this subscribe.
        night_mode_active_rx.mark_changed();
        timezone_rx.mark_changed();
        next_alarm_rx.mark_changed();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    res = localization_rx.recv() => match res {
                        Ok(config) => {
                            let proto_loc = Self::build_localization(&config);
                            for update in SettingUpdate::from_localization(&proto_loc) {
                                if let Err(e) = compositor.broadcast_setting(update.clone()) {
                                    warn!(error = %e, "failed to broadcast {update:?} to widgets");
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(
                                "localization receiver lagged, dropped {n} updates; widgets \
                                 may be out of sync until the next save"
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("localization channel closed; coordinator listener exiting");
                            return;
                        }
                    },
                    res = night_mode_active_rx.changed() => {
                        if res.is_err() {
                            info!("night mode watch channel closed; coordinator listener exiting");
                            return;
                        }
                        let active = *night_mode_active_rx.borrow();
                        if let Err(e) =
                            compositor.broadcast_setting(SettingUpdate::NightMode(active))
                        {
                            warn!(error = %e, "failed to broadcast NightMode({active}) to widgets");
                        }
                    },
                    res = timezone_rx.changed() => {
                        if res.is_err() {
                            info!("timezone watch channel closed; coordinator listener exiting");
                            return;
                        }
                        let tz = timezone_rx.borrow().clone();
                        if let Err(e) = compositor
                            .broadcast_setting(SettingUpdate::Timezone(tz.iana().to_owned()))
                        {
                            warn!(error = %e, "failed to broadcast timezone {tz} to widgets");
                        }
                    },
                    res = next_alarm_rx.changed() => {
                        if res.is_err() {
                            info!("next-alarm watch channel closed; coordinator listener exiting");
                            return;
                        }
                        let next = next_alarm_rx.borrow().clone();
                        if let Err(e) =
                            compositor.broadcast_setting(SettingUpdate::NextAlarm(next.clone()))
                        {
                            warn!(error = %e, "failed to broadcast NextAlarm({next:?}) to widgets");
                        }
                    },
                }
            }
        });
    }

    #[must_use]
    pub fn compositor(&self) -> Arc<dyn Compositor> {
        Arc::clone(&self.compositor)
    }

    /// Re-scan the widget registry so a widget installed at runtime becomes
    /// available without a restart.
    pub async fn refresh_widgets(&self) -> Result<(), crate::widget::RegistryError> {
        self.widget_manager.refresh().await
    }

    pub(crate) async fn reload_changed_widgets(&self, config_handle: &Arc<RwLock<ConfigHandle>>) {
        if let Err(error) = self.refresh_widgets().await {
            warn!(%error, "widget registry refresh failed; keeping running widgets");
            return;
        }
        let manager_snapshot = self.widget_manager.snapshot().await;

        let records: Vec<_> = self
            .spawn_records
            .read()
            .expect("BUG: widget spawn record lock poisoned")
            .iter()
            .map(|(instance_id, record)| (instance_id.clone(), record.clone()))
            .collect();

        for (instance_id, record) in records {
            let Some(installed) = self.widget_registry.get(&record.widget_uid) else {
                continue;
            };
            if self
                .spawn_records
                .read()
                .expect("BUG: widget spawn record lock poisoned")
                .get(&instance_id)
                != Some(&record)
            {
                continue;
            }
            // What the process is actually running decides, not what this spawn asked for:
            // a crash respawn re-reads the registry, so supervision may have already
            // brought the instance back on the new build.
            //
            // Skipping a pending respawn is deliberate for the same reason —
            // its own timer brings the widget back.
            let Some(running) = manager_snapshot.widgets.iter().find(|widget| {
                widget.launch.config_key.instance_id.to_string() == instance_id
                    && widget.state == ManagedWidgetState::Running
            }) else {
                continue;
            };
            if running.identity == installed.identity {
                continue;
            }

            let current = {
                let config = config_handle.read().await;
                current_widget_for_record(config.scenes(), &instance_id, &record)
            };
            let Some(widget) = current else {
                continue;
            };
            let Some(descriptor) =
                placement_viewport_descriptor(&widget, &self.hardware_capabilities)
            else {
                continue;
            };
            if !self
                .widget_registry
                .supports_viewport(&widget.widget_type_id, &descriptor)
            {
                warn!(%instance_id, "updated widget does not support its current viewport");
                continue;
            }

            info!(%instance_id, widget_type = %record.widget_uid, "reloading changed widget");
            self.stop_widget(&instance_id).await;
            self.spawn_widget(&record.scene_id, &widget).await;
        }

        info!("finished handling widget reload");
    }

    fn record_spawn(&self, instance_id: String, scene_id: SceneId, widget_uid: uuid::Uuid) {
        self.spawn_records
            .write()
            .expect("BUG: widget spawn record lock poisoned")
            .insert(
                instance_id,
                SpawnRecord {
                    scene_id,
                    widget_uid,
                },
            );
    }

    fn forget_spawn(&self, instance_id: &str) {
        self.spawn_records
            .write()
            .expect("BUG: widget spawn record lock poisoned")
            .remove(instance_id);
    }

    fn forget_all_spawns(&self) {
        self.spawn_records
            .write()
            .expect("BUG: widget spawn record lock poisoned")
            .clear();
    }

    pub async fn spawn_initial_widgets(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        localization: &LocalizationConfig,
        timezone: &Timezone,
        night_mode_active: bool,
        next_alarm: Option<NextAlarm>,
    ) {
        // Seed the compositor's setting cache so it can emit these
        // as part of the initial configure batch for every widget
        // that connects (and also propagate subsequent changes).
        let _ = self
            .compositor
            .broadcast_setting(SettingUpdate::Timezone(timezone.iana().to_owned()));
        let _ = self
            .compositor
            .broadcast_setting(SettingUpdate::NightMode(night_mode_active));
        let _ = self
            .compositor
            .broadcast_setting(SettingUpdate::NextAlarm(next_alarm));
        let loc = Self::build_localization(localization);
        for setting in SettingUpdate::from_localization(&loc) {
            let _ = self.compositor.broadcast_setting(setting);
        }

        self.spawn_all_configured_widgets(config_handle).await;

        info!("all scene widgets spawned");
    }

    pub(crate) async fn spawn_all_configured_widgets(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
    ) {
        let scene_ids = {
            let config = config_handle.read().await;
            supported_scenes(
                &self.widget_registry,
                &self.hardware_capabilities,
                config.scenes(),
            )
            .into_iter()
            .map(|scene| scene.id)
            .collect::<Vec<_>>()
        };
        for scene_id in scene_ids {
            self.spawn_configured_scene_widgets(
                config_handle,
                scene_id,
                ConfiguredSceneState::Enabled,
            )
            .await;
        }

        let config = config_handle.read().await;
        self.refresh_scene_cycling(config.scenes());
        if let Some(scene) = first_supported_active_scene(
            &self.widget_registry,
            &self.hardware_capabilities,
            config.scenes(),
        ) {
            self.set_active_scene(scene);
        }
    }

    /// Push the current enabled-scenes layout list to the compositor's
    /// drag-cycling state. Call after any scene-set or widget-layout
    /// mutation so swipe targets the post-mutation layouts.
    pub fn refresh_scene_cycling(&self, scenes: &IndexMap<SceneId, Scene>) {
        let layouts: Vec<_> =
            supported_scenes(&self.widget_registry, &self.hardware_capabilities, scenes)
                .into_iter()
                .map(|s| self.scene_to_layout(s))
                .collect();
        debug!(
            count = layouts.len(),
            "refreshing scene cycling on compositor"
        );
        if let Err(e) = self.compositor.set_scene_cycling(layouts) {
            warn!(error = %e, "failed to refresh scene cycling");
        }
    }

    pub(crate) async fn spawn_configured_scene_widgets(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        scene_id: SceneId,
        state: ConfiguredSceneState,
    ) {
        let widget_ids = {
            let config = config_handle.read().await;
            let Some(scene) = config.scenes().get(&scene_id) else {
                return;
            };
            if matches!(&state, ConfiguredSceneState::Enabled) && !scene.enabled {
                return;
            }
            if !self.scene_supported(scene) {
                return;
            }
            scene.widgets.keys().copied().collect::<Vec<_>>()
        };
        for widget_id in widget_ids {
            self.spawn_configured_widget(config_handle, scene_id, widget_id, state.clone())
                .await;
        }
    }

    pub(crate) async fn spawn_configured_widget(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        scene_id: SceneId,
        widget_id: crate::scene::WidgetId,
        state: ConfiguredSceneState,
    ) {
        let (widget, registration_receipt, activation_receipt) = {
            let _preview = match &state {
                ConfiguredSceneState::Enabled => None,
                ConfiguredSceneState::Preview(current_scene) => {
                    let preview = current_scene.lock().await;
                    if *preview != Some(scene_id) {
                        return;
                    }
                    Some(preview)
                }
            };
            let config = config_handle.read().await;
            let Some(scene) = config.scenes().get(&scene_id) else {
                return;
            };
            if matches!(&state, ConfiguredSceneState::Enabled) && !scene.enabled {
                return;
            }
            if !self.scene_supported(scene) {
                return;
            }
            let Some(widget) = scene.widgets.get(&widget_id).cloned() else {
                return;
            };
            let Some(registration) = self.widget_registration(&widget).await else {
                return;
            };
            let registration_receipt = match self.compositor.enqueue_register_widget(registration) {
                Ok(receipt) => receipt,
                Err(error) => {
                    warn!(%scene_id, widget_id = %widget.id, %error, "failed to enqueue widget registration");
                    return;
                }
            };
            let activation_receipt = match self
                .compositor
                .enqueue_activate_widget(WidgetInstanceKey::new(widget.id.as_uuid()))
            {
                Ok(receipt) => receipt,
                Err(error) => {
                    warn!(%scene_id, widget_id = %widget.id, %error, "failed to enqueue widget activation");
                    return;
                }
            };
            (widget, registration_receipt, activation_receipt)
        };

        let (registration_result, activation_result) =
            tokio::join!(registration_receipt.wait(), activation_receipt.wait());
        if let Err(error) = registration_result.and(activation_result) {
            warn!(%scene_id, widget_id = %widget.id, %error, "failed to apply widget registration and activation");
            return;
        }

        let (current, start) = {
            let _preview = match &state {
                ConfiguredSceneState::Enabled => None,
                ConfiguredSceneState::Preview(current_scene) => {
                    let preview = current_scene.lock().await;
                    if *preview != Some(scene_id) {
                        return;
                    }
                    Some(preview)
                }
            };
            let config = config_handle.read().await;
            let Some(scene) = config.scenes().get(&scene_id) else {
                return;
            };
            if matches!(&state, ConfiguredSceneState::Enabled) && !scene.enabled {
                return;
            }
            if !self.scene_supported(scene) {
                return;
            }
            let Some(current) = scene.widgets.get(&widget_id).cloned() else {
                return;
            };
            if !widget_configuration_unchanged(&current, &widget)
                || self.widget_manager.mode() != ManagerMode::Running
            {
                return;
            }
            let Some(wayland_display) = self.compositor.wayland_display() else {
                warn!(%scene_id, widget_id = %widget.id, "compositor not started, cannot spawn widget");
                return;
            };
            let generation = self.next_generation();
            let launch = widget_launch(scene_id, &current, wayland_display);
            let start = self
                .widget_manager
                .enqueue_spawn_widget(launch, generation)
                .await;
            (current, start)
        };

        let instance_id = current.id.to_string();
        match start.join().await {
            Ok(_) => self.record_spawn(instance_id, scene_id, current.widget_type_id),
            Err(error) => self.handle_start_error(scene_id, &current, instance_id, error),
        }
    }

    async fn widget_registration(&self, widget: &Widget) -> Option<WidgetRegistration> {
        if widget.widget_type_id.is_nil() {
            info!(widget_id = %widget.id, "skipping widget with nil widget_type_id");
            return None;
        }
        let position = widget_to_position(&self.hardware_capabilities, widget);
        let fullscreen_descriptor =
            fullscreen_descriptor_for_widget(widget, &self.hardware_capabilities);
        let viewport = placement_viewport_size(&widget.placement, &fullscreen_descriptor)?;
        let resolved = self.resolve_credentials(widget).await;
        Some(WidgetRegistration {
            key: WidgetInstanceKey::new(widget.id.as_uuid()),
            connection_mode: WidgetConnectionMode::Accepting,
            placement: WidgetPlacement {
                instance_id: widget.id.to_string(),
                position,
                size: viewport,
                visible: true,
            },
            initial_config: WidgetInitialConfig {
                width: viewport.width,
                height: viewport.height,
                viewport_shape: manifest_to_protocol_viewport_shape(widget.viewport_shape),
                display: platform_to_protocol_display(self.hardware_capabilities.display),
                params: params_to_json_map(&widget.params),
                credentials: resolved.view,
                credential_secrets: resolved.secrets,
                token: format!(
                    "{}-{}",
                    widget.id.as_uuid(),
                    placement_tag(&widget.placement)
                ),
            },
        })
    }

    fn handle_start_error(
        &self,
        scene_id: SceneId,
        widget: &Widget,
        instance_id: String,
        error: StartError,
    ) {
        match error {
            StartError::PendingRestart(error) => {
                warn!(
                    %scene_id,
                    widget_id = %instance_id,
                    widget_type = %widget.widget_type_id,
                    %error,
                    "failed to spawn widget; retaining registration for retry"
                );
                self.record_spawn(instance_id, scene_id, widget.widget_type_id);
            }
            StartError::Occupied(_) => {
                warn!(
                    %scene_id,
                    widget_id = %instance_id,
                    "widget became occupied while registering"
                );
            }
            error @ (StartError::Mode(_) | StartError::Spawn(_)) => {
                warn!(
                    %scene_id,
                    widget_id = %instance_id,
                    widget_type = %widget.widget_type_id,
                    %error,
                    "failed to spawn widget"
                );
            }
        }
    }

    async fn widget_occupied(&self, instance_id: &str) -> bool {
        self.widget_manager
            .snapshot()
            .await
            .widgets
            .iter()
            .any(|widget| {
                widget.launch.config_key.instance_id.to_string() == instance_id
                    && matches!(
                        widget.state,
                        ManagedWidgetState::Running | ManagedWidgetState::Stopping
                    )
            })
    }

    pub async fn spawn_widget(&self, scene_id: &crate::scene::SceneId, widget: &Widget) {
        if self.widget_manager.mode() != ManagerMode::Running {
            return;
        }
        let instance_id = widget.id.as_uuid().to_string();
        if self.widget_occupied(&instance_id).await {
            return;
        }

        // Defensive: a widget with a nil `widget_type_id` has no
        // manifest to run. The v0 config migration never
        // emits nil UIDs, but a hand-edited or malformed config could.
        // Skip it so the grid cell stays empty instead of logging a
        // not-found error on every scene.
        if widget.widget_type_id.is_nil() {
            info!(widget_id = %instance_id, "skipping widget with nil widget_type_id");
            return;
        }

        let position = widget_to_position(&self.hardware_capabilities, widget);
        let fullscreen_descriptor =
            fullscreen_descriptor_for_widget(widget, &self.hardware_capabilities);
        let Some(viewport) = placement_viewport_size(&widget.placement, &fullscreen_descriptor)
        else {
            warn!(
                placement = ?widget.placement,
                "skipping widget with unsupported slot span placement"
            );
            return;
        };
        let resolved = self.resolve_credentials(widget).await;
        let initial_config = WidgetInitialConfig {
            width: viewport.width,
            height: viewport.height,
            viewport_shape: manifest_to_protocol_viewport_shape(widget.viewport_shape),
            display: platform_to_protocol_display(self.hardware_capabilities.display),
            params: params_to_json_map(&widget.params),
            credentials: resolved.view,
            credential_secrets: resolved.secrets,
            token: format!(
                "{}-{}",
                widget.id.as_uuid(),
                placement_tag(&widget.placement)
            ),
        };

        info!(
            scene_id = %scene_id,
            widget_id = %instance_id,
            widget_type = %widget.widget_type_id,
            placement = ?widget.placement,
            "spawning widget"
        );

        let key = WidgetInstanceKey::new(widget.id.as_uuid());
        let registration = WidgetRegistration {
            key,
            connection_mode: WidgetConnectionMode::Accepting,
            placement: WidgetPlacement {
                instance_id: instance_id.clone(),
                position,
                size: viewport,
                visible: true,
            },
            initial_config,
        };
        let registration_receipt = match self.compositor.enqueue_register_widget(registration) {
            Ok(receipt) => receipt,
            Err(error) => {
                warn!(%scene_id, widget_id = %instance_id, %error, "failed to enqueue widget registration");
                return;
            }
        };
        if let Err(error) = registration_receipt.wait().await {
            warn!(%scene_id, widget_id = %instance_id, %error, "failed to apply widget registration");
            return;
        }
        let activation_receipt = match self.compositor.enqueue_activate_widget(key) {
            Ok(receipt) => receipt,
            Err(error) => {
                warn!(%scene_id, widget_id = %instance_id, %error, "failed to enqueue widget activation");
                return;
            }
        };
        if let Err(error) = activation_receipt.wait().await {
            warn!(%scene_id, widget_id = %instance_id, %error, "failed to apply widget activation");
            return;
        }
        let Some(wayland_display) = self.compositor.wayland_display() else {
            warn!(%scene_id, widget_id = %instance_id, "compositor not started, cannot spawn widget");
            return;
        };
        if self.widget_manager.mode() != ManagerMode::Running {
            return;
        }
        let generation = self.next_generation();
        let launch = widget_launch(*scene_id, widget, wayland_display);
        match self.widget_manager.spawn_widget(launch, generation).await {
            Ok(_) => {}
            Err(error) => {
                self.handle_start_error(*scene_id, widget, instance_id, error);
                return;
            }
        }

        self.record_spawn(instance_id, *scene_id, widget.widget_type_id);
    }

    pub async fn stop_widget(&self, instance_id: &str) {
        self.stop_retained_widget(instance_id, RetainedWidgetStop::Deactivate)
            .await;
    }

    pub async fn delete_widget(&self, instance_id: &str) {
        self.stop_retained_widget(instance_id, RetainedWidgetStop::Unregister)
            .await;
    }

    async fn stop_retained_widget(&self, instance_id: &str, stop: RetainedWidgetStop) {
        self.forget_spawn(instance_id);
        let receipt = match instance_id.parse::<WidgetInstanceKey>() {
            Ok(key) => Some(match stop {
                RetainedWidgetStop::Deactivate => self.compositor.enqueue_deactivate_widget(key),
                RetainedWidgetStop::Unregister => self.compositor.enqueue_unregister_widget(key),
            }),
            Err(error) => {
                match stop {
                    RetainedWidgetStop::Deactivate => {
                        warn!(widget_id = %instance_id, %error, "cannot deactivate malformed widget instance key");
                    }
                    RetainedWidgetStop::Unregister => {
                        warn!(widget_id = %instance_id, %error, "cannot unregister malformed widget instance key");
                    }
                }
                None
            }
        };
        let termination = self.widget_manager.stop_widget(instance_id).await;
        let ((), receipt_result) = tokio::join!(termination.join(), async {
            match receipt {
                Some(Ok(receipt)) => receipt.wait().await,
                Some(Err(error)) => Err(error),
                None => Ok(()),
            }
        });
        if let Err(error) = receipt_result {
            match stop {
                RetainedWidgetStop::Deactivate => {
                    warn!(widget_id = %instance_id, %error, "failed to deactivate stopped widget");
                }
                RetainedWidgetStop::Unregister => {
                    warn!(widget_id = %instance_id, %error, "failed to unregister deleted widget");
                }
            }
        }
    }

    pub fn update_widget_params(
        &self,
        instance_id: &str,
        params: &BTreeMap<ParamKey, ParamValue>,
    ) -> Result<(), CompositorError> {
        self.compositor
            .update_widget_params(&instance_id.to_owned(), params_to_json_map(params))
    }

    /// Ask a crash-looping widget to try the configuration just pushed to it,
    /// rather than sit out a delay earned against the configuration it replaced.
    pub async fn retry_pending_widget(&self, instance_id: &str) {
        self.widget_manager.retry_pending(instance_id).await;
    }

    /// Resolve the widget's bindings against the installed manifest
    /// and the stored accounts.
    ///
    /// Warns for any slot the manifest no longer authorises,
    /// and any whose account has since disappeared.
    ///
    /// Both spawn and hot-push resolve here.
    /// Neither can drift into checking the manifest while the other does not.
    pub(crate) async fn resolve_credentials(&self, widget: &Widget) -> credential::Resolution {
        let store = self.secret_store.read().await;
        let installed = self.widget_registry.get(&widget.widget_type_id);
        if installed.is_none() && !widget.credential_bindings.is_empty() {
            warn!(
                widget_id = %widget.id,
                widget_type_id = %widget.widget_type_id,
                "widget is not in the registry; honouring its stored bindings unchecked"
            );
        }

        let (authorised, unauthorised) = credential::authorised_bindings(
            &widget.credential_bindings,
            installed.as_ref().map(|info| &info.manifest.credentials),
            store.accounts(),
        );
        for (slot, why) in unauthorised {
            warn!(
                widget_id = %widget.id,
                slot = slot.as_str(),
                reason = why.reason(),
                "withholding a credential the installed manifest no longer authorises"
            );
        }

        for (slot, account) in credential::dangling_bindings(&authorised, store.accounts()) {
            warn!(
                widget_id = %widget.id,
                slot = slot.as_str(),
                account = %account,
                "credential slot bound to an account that no longer exists; treating it as unbound"
            );
        }

        credential::resolve(&authorised, store.accounts())
    }

    pub async fn update_widget_credentials(&self, widget: &Widget) -> Result<(), CompositorError> {
        let resolved = self.resolve_credentials(widget).await;
        let instance_id = widget.id.as_uuid().to_string();
        let changed = self.compositor.update_widget_credentials(
            &instance_id,
            resolved.view,
            resolved.secrets,
        )?;
        if changed {
            self.retry_pending_widget(&instance_id).await;
        }
        Ok(())
    }

    pub async fn stop_scene_widgets(&self, scene: &Scene) {
        for widget in scene.widgets.values() {
            self.stop_widget(&widget.id.as_uuid().to_string()).await;
        }
    }

    pub async fn delete_scene_widgets(&self, scene: &Scene) {
        for widget in scene.widgets.values() {
            self.delete_widget(&widget.id.as_uuid().to_string()).await;
        }
    }

    /// Stop all widget processes and shut down the compositor.
    ///
    /// Widgets are stopped first (SIGTERM → 10s timeout → SIGKILL) because they
    /// need the Wayland display socket to clean up GPU resources (GEM/DMA-BUF).
    /// The compositor is shut down second.
    pub async fn stop_all(&self) {
        info!("stopping all widgets and compositor");
        self.forget_all_spawns();
        self.widget_manager.shutdown().await;
        if let Err(e) = self.compositor.shutdown() {
            warn!(error = %e, "failed to shut down compositor");
        }
        info!("shutdown complete");
    }

    /// Stop every widget process while retaining inactive compositor registrations.
    pub async fn stop_all_widgets(&self, config_handle: &Arc<RwLock<ConfigHandle>>) {
        self.forget_all_spawns();
        let pause = self.widget_manager.begin_pause().await;
        let snapshot = self.widget_manager.snapshot().await;
        let mut keys = pause
            .instance_ids()
            .iter()
            .filter_map(|instance_id| instance_id.parse::<WidgetInstanceKey>().ok())
            .collect::<BTreeSet<_>>();
        keys.extend(
            snapshot
                .widgets
                .iter()
                .map(|widget| WidgetInstanceKey::new(widget.launch.config_key.instance_id)),
        );
        {
            let config = config_handle.read().await;
            keys.extend(config.scenes().values().flat_map(|scene| {
                scene
                    .widgets
                    .values()
                    .map(|widget| WidgetInstanceKey::new(widget.id.as_uuid()))
            }));
        }
        let receipts = keys
            .into_iter()
            .map(|key| self.compositor.enqueue_deactivate_widget(key))
            .collect::<Vec<_>>();
        tokio::join!(pause.join(), async {
            for receipt in receipts {
                let result = match receipt {
                    Ok(receipt) => receipt.wait().await,
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    warn!(%error, "failed to deactivate widget during upgrade pause");
                }
            }
        });
    }

    /// Sets the active scene layout on the compositor.
    pub fn set_active_scene(&self, scene: &Scene) {
        if !self.scene_supported(scene) {
            warn!(scene_id = %scene.id, "refusing to activate unsupported scene");
            return;
        }

        let layout = self.scene_to_layout(scene);
        info!(
            scene_id = %scene.id,
            widget_count = layout.widgets.len(),
            "setting active scene on compositor"
        );
        for widget in &layout.widgets {
            info!(
                instance_id = %widget.instance_id,
                x = widget.position.x,
                y = widget.position.y,
                width = widget.size.width,
                height = widget.size.height,
                visible = widget.visible,
                "scene widget placement"
            );
        }
        if let Err(e) = self.compositor.set_active_scene(layout) {
            warn!(scene_id = %scene.id, error = %e, "failed to set active scene");
        }
    }

    /// Pin the compositor to a single scene for the duration of a preview.
    ///
    /// A preview holds one scene on screen while the user edits it, so the
    /// compositor must show only that scene. Collapsing the cycling list to a
    /// single entry disables both manual drag and automatic cycling through
    /// the usual `scene_count < 2` gate, so a preview can never be swiped or
    /// cycled away. The full cycling list is restored when the preview ends.
    pub fn pin_preview_scene(&self, scene: &Scene) {
        if !self.scene_supported(scene) {
            warn!(scene_id = %scene.id, "refusing to preview unsupported scene");
            return;
        }

        let layout = self.scene_to_layout(scene);
        if let Err(e) = self.compositor.set_scene_cycling(vec![layout]) {
            warn!(scene_id = %scene.id, error = %e, "failed to pin preview scene");
        }
    }

    fn scene_supported(&self, scene: &Scene) -> bool {
        scene_supported_with_registry(&self.widget_registry, scene, &self.hardware_capabilities)
    }

    fn scene_to_layout(&self, scene: &Scene) -> SceneLayout {
        scene_to_layout_with_registry(&self.widget_registry, &self.hardware_capabilities, scene)
    }

    fn build_localization(config: &LocalizationConfig) -> Localization {
        Localization {
            date_format: config.date_format,
            time_format: config.time_system,
            number_format: config.number_format,
            temperature_unit: config.temperature_unit,
            first_day_of_week: config.first_day_of_week,
            unit_system: config.unit_system,
        }
    }
}

/// Widget lifecycle handle for firmware upgrades: pairs the coordinator with
/// the config so a failed upgrade can respawn the widgets of the currently
/// configured scenes.
#[derive(Debug)]
pub(crate) struct UpgradeWidgetLifecycle {
    coordinator: Arc<Coordinator>,
    config_handle: Arc<RwLock<ConfigHandle>>,
}

impl UpgradeWidgetLifecycle {
    pub(crate) fn new(
        coordinator: Arc<Coordinator>,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) -> Self {
        Self {
            coordinator,
            config_handle,
        }
    }
}

#[async_trait::async_trait]
impl crate::system_upgrade::WidgetLifecycle for UpgradeWidgetLifecycle {
    async fn stop_all_widgets(&self) {
        self.coordinator.stop_all_widgets(&self.config_handle).await;
    }

    async fn restart_widgets(&self) {
        if self.coordinator.widget_manager.resume().await != ManagerMode::Running {
            return;
        }
        self.coordinator
            .spawn_all_configured_widgets(&self.config_handle)
            .await;
    }

    async fn refresh_widgets(&self) {
        if let Err(error) = self.coordinator.refresh_widgets().await {
            warn!(%error, "failed to refresh widgets after package activation");
        }
    }
}

fn params_to_json_map(
    params: &BTreeMap<ParamKey, ParamValue>,
) -> serde_json::Map<String, serde_json::Value> {
    params
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::{DisplayInfo, DisplayShape as CompositorDisplayShape, SlotGrid};
    use crate::widget::ViewportDescriptor;
    use bmc_widget_manifest::ViewportShape;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    /// A swapped arm or a dropped pid here is a crash respawn that never comes back,
    /// and neither side can notice: the manager's tests end at the event,
    /// the compositor's begin at the call.
    #[tokio::test]
    async fn each_lifecycle_event_applies_its_own_compositor_call() {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let compositor = Arc::new(crate::compositor::testing::RecordingCompositor::default());

        for event in [
            WidgetEvent::Exited {
                instance_id: "alpha".to_owned(),
                generation: WidgetGeneration(1),
                pid: 100,
            },
            WidgetEvent::Respawned {
                instance_id: "alpha".to_owned(),
                generation: WidgetGeneration(1),
                pid: 200,
            },
            WidgetEvent::Abandoned {
                instance_id: "beta".to_owned(),
                generation: WidgetGeneration(4),
            },
        ] {
            events_tx.send(event).expect("BUG: queue a widget event");
        }
        // Closing the stream is what ends the listener, so awaiting it drains
        // the queue without a timeout.
        drop(events_tx);
        apply_widget_events(Arc::clone(&compositor) as Arc<dyn Compositor>, events_rx).await;

        assert_eq!(
            compositor.widget_calls(),
            [
                "clear_pid alpha g1 100",
                "bind_respawned alpha g1 200",
                "unregister_abandoned beta g4"
            ]
        );
    }

    fn bmc100_capabilities() -> HardwareCapabilities {
        HardwareCapabilities {
            display: DisplayInfo {
                width: 1280,
                height: 480,
                shape: CompositorDisplayShape::Rectangular,
                dpi: 1,
            },
            slot_grid: Some(SlotGrid {
                columns: 4,
                rows: 2,
            }),
        }
    }

    async fn coordinator_with_widget(
        binary: Option<&str>,
    ) -> (
        tempfile::TempDir,
        Coordinator,
        Arc<crate::compositor::testing::RecordingCompositor>,
        Scene,
        mpsc::UnboundedReceiver<WidgetEvent>,
    ) {
        let temp = tempfile::tempdir().expect("BUG: test tempdir");
        let widget_dir = temp.path().join("widget-package");
        std::fs::create_dir(&widget_dir).expect("BUG: create widget directory");
        let uid =
            uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: widget uid");
        std::fs::write(
            widget_dir.join("manifest.json"),
            r#"{"uid":"550e8400-e29b-41d4-a716-446655440000","version":"1.0.0","name":"coordinator-test","description":"coordinator test","binary":"widget","supported_viewports":[{"type":"rectangular","min_width":1280,"max_width":1280,"min_height":480,"max_height":480}]}"#,
        )
        .expect("BUG: write manifest");
        if let Some(body) = binary {
            let path = widget_dir.join("widget");
            std::fs::write(&path, body).expect("BUG: write widget");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("BUG: make widget executable");
        }
        let (manager, events) = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let registry = manager.registry();
        let compositor = Arc::new(crate::compositor::testing::RecordingCompositor::default());
        let secret_store = Arc::new(RwLock::new(
            SecretStoreHandle::init(&temp.path().join("config.json")).await,
        ));
        let coordinator = Coordinator::new(
            manager,
            Arc::clone(&compositor) as Arc<dyn Compositor>,
            registry,
            bmc100_capabilities(),
            secret_store,
        );
        let scene = Scene::fullscreen(uid, BTreeMap::new());
        (temp, coordinator, compositor, scene, events)
    }

    async fn config_with_scene(
        temp: &tempfile::TempDir,
        scene: Scene,
    ) -> Arc<RwLock<ConfigHandle>> {
        let (mut config, _accounts) = ConfigHandle::init(
            temp.path().join("settings.json"),
            50,
            50,
            50,
            50,
            bmc_platform::Product::Bmc100,
        )
        .await;
        config.scenes_mut().clear();
        config.scenes_mut().insert(scene.id, scene);
        Arc::new(RwLock::new(config))
    }

    async fn wait_for_widget_calls(
        compositor: &crate::compositor::testing::RecordingCompositor,
        expected: usize,
    ) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while compositor.widget_calls().len() < expected {
            assert!(
                tokio::time::Instant::now() < deadline,
                "registration commands were not enqueued"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn configured_start_waits_for_active_receipts_before_process_spawn() {
        let (temp, coordinator, compositor, scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let marker = temp.path().join("started");
        let body = format!("#!/bin/sh\ntouch {}\nexec sleep 30\n", marker.display());
        std::fs::write(temp.path().join("widget-package/widget"), body)
            .expect("BUG: update test widget");
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let task = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(
                        &config,
                        scene.id,
                        ConfiguredSceneState::Enabled,
                    )
                    .await;
            }
        });

        wait_for_widget_calls(&compositor, 2).await;
        assert!(!marker.exists(), "process started before active receipts");
        compositor.release_widget_receipts();
        task.await
            .expect("BUG: configured spawn task must complete");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !marker.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "process did not start after active receipts"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn configured_start_revalidates_after_receipts_without_holding_config_lock() {
        let (temp, coordinator, compositor, scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let task = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(
                        &config,
                        scene.id,
                        ConfiguredSceneState::Enabled,
                    )
                    .await;
            }
        });

        wait_for_widget_calls(&compositor, 2).await;
        config.write().await.scenes_mut().clear();
        compositor.release_widget_receipts();
        task.await
            .expect("BUG: configured spawn task must complete");
        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty(),
            "deleted configuration started after receipt wait"
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn preview_teardown_while_activation_is_pending_rejects_start() {
        let (temp, coordinator, compositor, scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        let preview_scene = Arc::new(Mutex::new(Some(scene.id)));
        compositor.hold_widget_receipts();
        let spawn = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            let preview_scene = Arc::clone(&preview_scene);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(
                        &config,
                        scene.id,
                        ConfiguredSceneState::Preview(preview_scene),
                    )
                    .await;
            }
        });

        wait_for_widget_calls(&compositor, 2).await;
        assert_eq!(preview_scene.lock().await.take(), Some(scene.id));
        let stop = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            async move { coordinator.stop_scene_widgets(&scene).await }
        });
        wait_for_widget_calls(&compositor, 3).await;
        compositor.release_widget_receipts();
        spawn.await.expect("BUG: preview spawn task must complete");
        stop.await.expect("BUG: preview stop task must complete");

        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        assert!(
            compositor.widget_calls()[2].starts_with("deactivate "),
            "teardown must order deactivation after pending activation"
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn upgrade_pause_rejects_pending_start_and_deactivates_configured_record() {
        let (temp, coordinator, compositor, scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let spawn = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(
                        &config,
                        scene.id,
                        ConfiguredSceneState::Enabled,
                    )
                    .await;
            }
        });

        wait_for_widget_calls(&compositor, 2).await;
        let pause = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move { coordinator.stop_all_widgets(&config).await }
        });
        wait_for_widget_calls(&compositor, 3).await;
        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Paused);
        compositor.release_widget_receipts();
        spawn.await.expect("BUG: pending spawn task must complete");
        pause.await.expect("BUG: upgrade pause task must complete");

        let snapshot = coordinator.widget_manager.snapshot().await;
        assert_eq!(snapshot.mode, ManagerMode::Paused);
        assert!(
            snapshot.widgets.is_empty(),
            "paused start must have no successor"
        );
        let widget_key = WidgetInstanceKey::new(
            scene
                .widgets
                .values()
                .next()
                .expect("BUG: test scene must contain a widget")
                .id
                .as_uuid(),
        );
        assert_eq!(
            compositor.retained_mode(widget_key),
            Some(WidgetConnectionMode::Inactive),
            "upgrade pause must leave every configured record inactive"
        );
        assert!(compositor.widget_calls()[2].starts_with("deactivate "));
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn unsupported_configured_scene_never_registers_or_starts() {
        let (temp, coordinator, compositor, mut scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        for widget in scene.widgets.values_mut() {
            widget.viewport_shape = ViewportShape::Round;
        }
        let config = config_with_scene(&temp, scene.clone()).await;

        coordinator
            .spawn_configured_scene_widgets(&config, scene.id, ConfiguredSceneState::Enabled)
            .await;

        assert!(compositor.widget_calls().is_empty());
        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn crash_respawn_has_no_compositor_lifecycle_traffic() {
        let (temp, coordinator, compositor, scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nexit 0\n")).await;
        let executions = temp.path().join("executions");
        std::fs::write(
            temp.path().join("widget-package/widget"),
            format!(
                "#!/bin/sh\nprintf 'started\\n' >> {}\nexit 0\n",
                executions.display()
            ),
        )
        .expect("BUG: respawn fixture must be writable");
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id, ConfiguredSceneState::Enabled)
            .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let execution_count =
                std::fs::read_to_string(&executions).map_or(0, |contents| contents.lines().count());
            if execution_count >= 2 {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "widget did not execute twice before the respawn deadline"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        assert_eq!(
            compositor.widget_calls(),
            [
                format!("register_retained {}", scene.widgets[0].id),
                format!("activate {}", scene.widgets[0].id),
            ],
            "crash supervision must not call compositor PID lifecycle operations"
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn retained_spawn_failure_keeps_registration_for_respawn() {
        let (temp, coordinator, compositor, scene, _events) =
            coordinator_with_widget(Some("not an executable\n")).await;
        let widget = scene.widgets.values().next().expect("BUG: widget");
        let instance_id = widget.id.to_string();

        coordinator.spawn_widget(&scene.id, widget).await;

        let calls = compositor.widget_calls();
        assert!(
            !calls.iter().any(|call| call.starts_with("unregister ")),
            "retained retry must keep the compositor record: {calls:?}"
        );
        assert!(
            coordinator
                .spawn_records
                .read()
                .expect("BUG: spawn records")
                .contains_key(&instance_id)
        );
        assert!(matches!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .as_slice(),
            [super::super::manager::ManagedWidgetSnapshot {
                state: ManagedWidgetState::PendingRestart,
                ..
            }]
        ));
        std::fs::write(
            temp.path().join("widget-package/widget"),
            "#!/bin/sh\nexec sleep 30\n",
        )
        .expect("BUG: replace widget executable");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while !matches!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .as_slice(),
            [super::super::manager::ManagedWidgetSnapshot {
                state: ManagedWidgetState::Running,
                ..
            }]
        ) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "retained widget did not recover"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let calls = compositor.widget_calls();
        assert!(
            !calls.iter().any(|call| call.starts_with("bind_respawned ")
                || call.starts_with("clear_pid ")
                || call.starts_with("unregister_abandoned ")),
            "manager recovery must not produce compositor lifecycle traffic: {calls:?}"
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn occupied_preflight_does_not_unregister_the_healthy_record() {
        let (_temp, coordinator, compositor, scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nwhile :; do sleep 30; done\n")).await;
        let widget = scene.widgets.values().next().expect("BUG: widget");

        coordinator.spawn_widget(&scene.id, widget).await;
        let calls = compositor.widget_calls();
        coordinator.spawn_widget(&scene.id, widget).await;

        assert_eq!(compositor.widget_calls(), calls);
        assert!(
            calls
                .iter()
                .any(|call| call.starts_with("register_retained "))
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn spawn_after_coordinator_stop_starts_only_after_reap() {
        let (_temp, coordinator, _compositor, scene, _events) =
            coordinator_with_widget(Some("#!/bin/sh\nwhile :; do sleep 30; done\n")).await;
        let widget = scene.widgets.values().next().expect("BUG: widget");
        let instance_id = widget.id.to_string();
        coordinator.spawn_widget(&scene.id, widget).await;

        coordinator.stop_widget(&instance_id).await;
        coordinator.spawn_widget(&scene.id, widget).await;

        assert!(matches!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .as_slice(),
            [super::super::manager::ManagedWidgetSnapshot {
                state: ManagedWidgetState::Running,
                ..
            }]
        ));
        coordinator.widget_manager.shutdown().await;
    }

    #[test]
    fn placement_viewport_for_fullscreen_is_active_display() {
        let desc = ViewportDescriptor {
            viewport_shape: ViewportShape::Rectangular,
            width: 1280,
            height: 480,
            dpi: 1,
        };
        let size = placement_viewport_size(&crate::scene::WidgetPlacement::Fullscreen, &desc)
            .expect("BUG: fullscreen viewport must derive");
        assert_eq!(size.width, 1280);
        assert_eq!(size.height, 480);
    }

    #[test]
    fn placement_viewport_for_slot_span_uses_allow_list() {
        let desc = ViewportDescriptor {
            viewport_shape: ViewportShape::Rectangular,
            width: 1280,
            height: 480,
            dpi: 1,
        };
        let size = placement_viewport_size(
            &crate::scene::WidgetPlacement::SlotSpan(crate::scene::SlotSpan {
                columns: 2,
                rows: 2,
            }),
            &desc,
        )
        .expect("BUG: slot span viewport must derive");
        assert_eq!(size.width, 638);
        assert_eq!(size.height, 480);
    }

    #[test]
    fn fullscreen_scene_with_unknown_widget_is_unsupported() {
        let registry = WidgetRegistry::new(vec![]);
        let scene = crate::scene::Scene::fullscreen(uuid::Uuid::new_v4(), BTreeMap::new());
        assert!(
            !scene_supported_with_registry(&registry, &scene, &bmc100_capabilities()),
            "BUG: fullscreen scene whose widget is absent from the registry must be unsupported",
        );
    }

    #[test]
    fn combined_scene_with_disallowed_span_is_unsupported() {
        let registry = WidgetRegistry::new(vec![]);
        let mut scene = crate::scene::Scene::combined();
        let widget = Widget::new(
            uuid::Uuid::new_v4(),
            BTreeMap::new(),
            crate::scene::WidgetPosition { row: 0, col: 0 },
            crate::scene::WidgetPlacement::SlotSpan(crate::scene::SlotSpan {
                columns: 3,
                rows: 1,
            }),
        );
        scene.widgets.insert(widget.id, widget);
        assert!(
            !scene_supported_with_registry(&registry, &scene, &bmc100_capabilities()),
            "BUG: combined scene with a non-allow-list span must be unsupported",
        );
    }

    #[test]
    fn platform_display_converts_to_protocol_display() {
        let platform = bmc_platform::DisplayInfo {
            width: 480,
            height: 320,
            shape: bmc_platform::DisplayShape::Rectangular,
            dpi: 7,
        };
        let proto = platform_to_protocol_display(platform);
        assert_eq!(proto.width, 480);
        assert_eq!(proto.height, 320);
        assert_eq!(proto.shape, bmc_widget_protocol::DisplayShape::Rectangular);
        assert_eq!(proto.dpi, 7);
        assert_ne!(proto, bmc_widget_protocol::DisplayInfo::BMC100);

        let round = bmc_platform::DisplayInfo {
            width: 480,
            height: 480,
            shape: bmc_platform::DisplayShape::Round,
            dpi: 7,
        };
        assert_eq!(
            platform_to_protocol_display(round).shape,
            bmc_widget_protocol::DisplayShape::Round
        );
    }

    #[test]
    fn combined_scene_with_allowed_span_is_supported() {
        let registry = WidgetRegistry::new(vec![]);
        let mut scene = crate::scene::Scene::combined();
        let widget = Widget::new(
            uuid::Uuid::new_v4(),
            BTreeMap::new(),
            crate::scene::WidgetPosition { row: 0, col: 0 },
            crate::scene::WidgetPlacement::SlotSpan(crate::scene::SlotSpan {
                columns: 1,
                rows: 1,
            }),
        );
        scene.widgets.insert(widget.id, widget);
        assert!(
            scene_supported_with_registry(&registry, &scene, &bmc100_capabilities()),
            "BUG: combined scene with an allow-list span must be supported",
        );
    }

    fn reload_record(scene_id: SceneId, uid: uuid::Uuid) -> SpawnRecord {
        SpawnRecord {
            scene_id,
            widget_uid: uid,
        }
    }

    #[test]
    fn reload_resolves_the_current_widget_from_its_recorded_scene_and_uid() {
        let uid = uuid::Uuid::new_v4();
        let mut scene = Scene::fullscreen(uid, BTreeMap::new());
        let widget = scene.widgets.values().next().expect("BUG: widget").clone();
        let instance_id = widget.id.to_string();
        let record = reload_record(scene.id, uid);
        let scenes = IndexMap::from([(scene.id, scene.clone())]);

        assert_eq!(
            current_widget_for_record(&scenes, &instance_id, &record)
                .expect("current widget")
                .id,
            widget.id
        );
        assert!(current_widget_for_record(&IndexMap::new(), &instance_id, &record).is_none());

        scene
            .widgets
            .get_mut(&widget.id)
            .expect("BUG: widget")
            .widget_type_id = uuid::Uuid::new_v4();
        let changed_scenes = IndexMap::from([(scene.id, scene)]);
        assert!(current_widget_for_record(&changed_scenes, &instance_id, &record).is_none());
    }
}

#[cfg(test)]
mod support_tests {
    use super::{
        fullscreen_descriptor_for_widget, scene_supported_with_registry,
        scene_to_layout_with_registry,
    };
    use crate::compositor::{DisplayInfo, DisplayShape, HardwareCapabilities, SlotGrid};
    use crate::scene::{Scene, Widget};
    use crate::widget::{ViewportDescriptor, WidgetInfo, WidgetRegistry};
    use bmc_widget_manifest::{Manifest, ViewportShape, WidgetCategory, WidgetViewportConstraint};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn caps(
        slot_grid: Option<SlotGrid>,
        width: u32,
        height: u32,
        display_shape: DisplayShape,
    ) -> HardwareCapabilities {
        HardwareCapabilities {
            display: DisplayInfo {
                width,
                height,
                shape: display_shape,
                dpi: 1,
            },
            slot_grid,
        }
    }

    fn registry_with_widget(uid: Uuid, constraint: WidgetViewportConstraint) -> WidgetRegistry {
        let manifest = Manifest {
            uid,
            version: semver::Version::new(1, 0, 0),
            name: "test-widget".to_owned(),
            subname: None,
            description: "Test widget".to_owned(),
            config_help: None,
            author: None,
            binary: PathBuf::from("bin/widget"),
            icon: None,
            category: WidgetCategory::Misc,
            settings: vec![],
            supported_viewports: vec![constraint],
            params: indexmap::IndexMap::new(),
            credentials: indexmap::IndexMap::new(),
        };
        WidgetRegistry::new(vec![WidgetInfo::for_test(
            manifest,
            PathBuf::from("/test/widgets/test-widget"),
            PathBuf::from("/test/widgets/test-widget/bin/widget"),
            None,
        )])
    }

    fn fullscreen_scene_with(widget_type_id: Uuid, viewport_shape: ViewportShape) -> Scene {
        let mut scene = Scene::fullscreen(widget_type_id, BTreeMap::new());
        for widget in scene.widgets.values_mut() {
            widget.viewport_shape = viewport_shape;
        }
        scene
    }

    fn fullscreen_widget(widget_type_id: Uuid, viewport_shape: ViewportShape) -> Widget {
        fullscreen_scene_with(widget_type_id, viewport_shape)
            .widgets
            .into_values()
            .next()
            .expect("BUG: Scene::fullscreen always contains one widget")
    }

    #[test]
    fn fullscreen_descriptor_for_widget_uses_widget_viewport_shape() {
        let widget = fullscreen_widget(Uuid::new_v4(), ViewportShape::Round);
        let bmc = caps(None, 1_280, 480, DisplayShape::Rectangular);
        assert_eq!(
            fullscreen_descriptor_for_widget(&widget, &bmc),
            ViewportDescriptor {
                viewport_shape: ViewportShape::Round,
                width: 1_280,
                height: 480,
                dpi: 1,
            },
        );
    }

    #[test]
    fn combined_scene_unsupported_without_slot_grid() {
        let scene = Scene::combined();
        let registry = WidgetRegistry::new(std::iter::empty());
        let no_grid = caps(None, 320, 240, DisplayShape::Rectangular);
        assert!(!scene_supported_with_registry(&registry, &scene, &no_grid));
    }

    #[test]
    fn combined_scene_supported_with_slot_grid() {
        let scene = Scene::combined();
        let registry = WidgetRegistry::new(std::iter::empty());
        let grid = caps(
            Some(SlotGrid {
                columns: 4,
                rows: 2,
            }),
            1_280,
            480,
            DisplayShape::Rectangular,
        );
        assert!(scene_supported_with_registry(&registry, &scene, &grid));
    }

    #[test]
    fn layout_excludes_nil_and_unregistered_widget_types() {
        let registered = Uuid::new_v4();
        let constraint = WidgetViewportConstraint {
            viewport_shape: ViewportShape::Rectangular,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            min_dpi: None,
            max_dpi: None,
        };
        let registry = registry_with_widget(registered, constraint);

        let mut scene = Scene::combined();
        let mut spawnable_instance_id = None;
        for (col, widget_type_id) in [registered, Uuid::new_v4(), Uuid::nil()]
            .into_iter()
            .enumerate()
        {
            let widget = Widget::new(
                widget_type_id,
                BTreeMap::new(),
                crate::scene::WidgetPosition {
                    row: 0,
                    col: u8::try_from(col).expect("BUG: test column fits u8"),
                },
                crate::scene::WidgetPlacement::SlotSpan(crate::scene::SlotSpan {
                    columns: 1,
                    rows: 1,
                }),
            );
            if widget_type_id == registered {
                spawnable_instance_id = Some(widget.id.as_uuid().to_string());
            }
            scene.widgets.insert(widget.id, widget);
        }

        let capabilities = caps(
            Some(SlotGrid {
                columns: 4,
                rows: 2,
            }),
            1_280,
            480,
            DisplayShape::Rectangular,
        );
        let layout = scene_to_layout_with_registry(&registry, &capabilities, &scene);
        assert_eq!(
            layout
                .widgets
                .iter()
                .map(|w| w.instance_id.clone())
                .collect::<Vec<_>>(),
            vec![spawnable_instance_id.expect("BUG: registered widget was inserted")],
            "layout must keep only widgets the spawner can launch",
        );
    }

    #[test]
    fn fullscreen_scene_unsupported_when_manifest_rejects_descriptor() {
        let widget_type_id = Uuid::new_v4();
        let constraint = WidgetViewportConstraint {
            viewport_shape: ViewportShape::Rectangular,
            min_width: Some(1_280),
            max_width: Some(1_280),
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        };
        let registry = registry_with_widget(widget_type_id, constraint);
        let scene = fullscreen_scene_with(widget_type_id, ViewportShape::Rectangular);
        let bmm = caps(None, 320, 240, DisplayShape::Rectangular);
        assert!(!scene_supported_with_registry(&registry, &scene, &bmm));
    }

    #[test]
    fn fullscreen_scene_supported_when_manifest_accepts_descriptor() {
        let widget_type_id = Uuid::new_v4();
        let constraint = WidgetViewportConstraint {
            viewport_shape: ViewportShape::Rectangular,
            min_width: Some(1_280),
            max_width: Some(1_280),
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        };
        let registry = registry_with_widget(widget_type_id, constraint);
        let scene = fullscreen_scene_with(widget_type_id, ViewportShape::Rectangular);
        let bmc = caps(None, 1_280, 480, DisplayShape::Rectangular);
        assert!(scene_supported_with_registry(&registry, &scene, &bmc));
    }

    #[test]
    fn fullscreen_descriptor_carries_widget_viewport_shape_to_matcher() {
        let widget_type_id = Uuid::new_v4();
        let constraint = WidgetViewportConstraint {
            viewport_shape: ViewportShape::Round,
            min_width: Some(480),
            max_width: Some(480),
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        };
        let registry = registry_with_widget(widget_type_id, constraint);
        let scene = fullscreen_scene_with(widget_type_id, ViewportShape::Round);
        let bfm = caps(None, 480, 480, DisplayShape::Round);
        assert!(scene_supported_with_registry(&registry, &scene, &bfm));
    }

    use super::{first_supported_active_scene, supported_scenes};
    use crate::scene::SceneId;
    use indexmap::IndexMap;

    #[test]
    fn filters_out_unsupported_scenes() {
        let widget_uid = Uuid::new_v4();
        let registry = registry_with_widget(
            widget_uid,
            WidgetViewportConstraint {
                viewport_shape: ViewportShape::Rectangular,
                min_width: Some(320),
                max_width: Some(320),
                min_height: Some(240),
                max_height: Some(240),
                min_dpi: None,
                max_dpi: None,
            },
        );
        let mut scenes: IndexMap<SceneId, Scene> = IndexMap::new();
        let combined = Scene::combined();
        let full = Scene::fullscreen(widget_uid, BTreeMap::new());
        scenes.insert(combined.id, combined);
        scenes.insert(full.id, full.clone());
        let bmm = caps(None, 320, 240, DisplayShape::Rectangular);
        let supported = supported_scenes(&registry, &bmm, &scenes);
        assert_eq!(supported.len(), 1);
        assert_eq!(supported[0].id, full.id);
    }

    #[test]
    fn no_active_scene_when_all_unsupported() {
        let mut scenes: IndexMap<SceneId, Scene> = IndexMap::new();
        let combined = Scene::combined();
        scenes.insert(combined.id, combined);
        let registry = WidgetRegistry::new(std::iter::empty());
        let bmm = caps(None, 320, 240, DisplayShape::Rectangular);
        assert!(first_supported_active_scene(&registry, &bmm, &scenes).is_none());
    }

    #[test]
    fn first_supported_scene_skips_unsupported() {
        let widget_uid = Uuid::new_v4();
        let registry = registry_with_widget(
            widget_uid,
            WidgetViewportConstraint {
                viewport_shape: ViewportShape::Rectangular,
                min_width: Some(320),
                max_width: Some(320),
                min_height: Some(240),
                max_height: Some(240),
                min_dpi: None,
                max_dpi: None,
            },
        );
        let mut scenes: IndexMap<SceneId, Scene> = IndexMap::new();
        let combined = Scene::combined();
        let full = Scene::fullscreen(widget_uid, BTreeMap::new());
        let combined_id = combined.id;
        let full_id = full.id;
        scenes.insert(combined_id, combined);
        scenes.insert(full_id, full);
        let bmm = caps(None, 320, 240, DisplayShape::Rectangular);
        let chosen = first_supported_active_scene(&registry, &bmm, &scenes)
            .expect("BUG: full scene supported on BMM");
        assert_eq!(chosen.id, full_id);
    }
}
