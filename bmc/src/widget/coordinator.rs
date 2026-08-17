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
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;
use tokio::sync::{RwLock, broadcast, mpsc, watch};
use tracing::{debug, info, warn};

use crate::BmcManager;
use crate::compositor::{
    Compositor, CompositorError, HardwareCapabilities, Position, SceneLayout, Size, WidgetPlacement,
};
use crate::config::ConfigHandle;
use crate::config::LocalizationConfig;
use crate::credential;
use crate::scene::{Scene, SceneId, Widget, WidgetPosition};
use crate::secret_store::SecretStoreHandle;
use bmc_net::NetworkManager;

use super::manager::ChildObservation;
use super::{WidgetEvent, WidgetIdentity, WidgetManager, WidgetRegistry};

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

/// Minimum environment the spawner puts on a widget process.
///
/// Every piece of widget-specific configuration (instance id, size, params)
/// now flows through the `deck_widget_v1` Wayland protocol, so this struct
/// no longer carries any of that — the spawner only needs to know where
/// the Wayland socket lives and which instance id to attribute the spawn to for logging.
#[derive(Debug, Clone)]
pub struct WidgetEnv {
    pub instance_id: String,
    pub wayland_display: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpawnRecord {
    scene_id: SceneId,
    widget_uid: uuid::Uuid,
    identity: WidgetIdentity,
}

fn record_needs_reload(registry: &WidgetRegistry, record: &SpawnRecord) -> bool {
    registry
        .get(&record.widget_uid)
        .is_some_and(|installed| installed.identity != record.identity)
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
/// Re-resolving is cheap and a no-change push is dropped,
/// so over-firing costs nothing.
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

/// Apply widget lifecycle events from the manager to the compositor:
/// a self-exited process gets its pid association cleared
/// (a recycled pid cannot be mistaken for the dead widget),
/// and a respawned process gets its new pid bound,
/// so the compositor recognizes it when it reconnects.
/// `clear_pid` detaches the process but keeps the instance registered,
/// so the reconnect replays the same configure batch as the first attach
/// and the respawn only has to re-bind its pid.
/// Takes the compositor rather than the whole [`Coordinator`]: holding the
/// coordinator would keep its `WidgetManager`, and so the actor's command
/// sender, alive for as long as this task runs — while this task only ends
/// when the actor drops the event sender. Neither could ever finish.
pub fn start_widget_event_listener(
    compositor: Arc<dyn Compositor>,
    mut events: mpsc::UnboundedReceiver<WidgetEvent>,
) {
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                WidgetEvent::Exited { instance_id, pid } => {
                    if let Err(e) = compositor.clear_pid(&instance_id, pid) {
                        warn!(
                            widget_id = %instance_id,
                            pid,
                            error = %e,
                            "failed to detach the pid of an exited widget"
                        );
                    }
                }
                WidgetEvent::Respawned { instance_id, pid } => {
                    if let Err(e) = compositor.set_widget_pid(&instance_id, pid) {
                        warn!(
                            widget_id = %instance_id,
                            pid,
                            error = %e,
                            "failed to bind the new pid after a widget respawn"
                        );
                    }
                }
                WidgetEvent::Abandoned { instance_id } => {
                    let _ = compositor.unregister_widget(&instance_id);
                }
            }
        }
    });
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
        }
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

        let records: Vec<_> = self
            .spawn_records
            .read()
            .expect("BUG: widget spawn record lock poisoned")
            .iter()
            .map(|(instance_id, record)| (instance_id.clone(), record.clone()))
            .collect();

        for (instance_id, record) in records {
            if !record_needs_reload(&self.widget_registry, &record) {
                continue;
            }
            if self
                .spawn_records
                .read()
                .expect("BUG: widget spawn record lock poisoned")
                .get(&instance_id)
                != Some(&record)
            {
                continue;
            }
            // A pending respawn is skipped on purpose: supervision re-reads the
            // registry when its timer fires, so it brings the widget back on the
            // new binary without this reload replacing anything.
            match self.widget_manager.observe_child(&instance_id).await {
                ChildObservation::Running => {}
                ChildObservation::Exited | ChildObservation::Missing => continue,
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

    fn record_spawn(
        &self,
        instance_id: String,
        scene_id: SceneId,
        widget_uid: uuid::Uuid,
        identity: WidgetIdentity,
    ) {
        self.spawn_records
            .write()
            .expect("BUG: widget spawn record lock poisoned")
            .insert(
                instance_id,
                SpawnRecord {
                    scene_id,
                    widget_uid,
                    identity,
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
        scenes: &IndexMap<SceneId, Scene>,
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

        self.spawn_all_scene_widgets(scenes).await;

        info!("all scene widgets spawned");
    }

    /// Spawn the widgets of every supported enabled scene, refresh drag
    /// cycling and activate the first supported scene. Used at startup and
    /// to bring widgets back after a failed firmware upgrade (whose
    /// preparation stops them all).
    pub async fn spawn_all_scene_widgets(&self, scenes: &IndexMap<SceneId, Scene>) {
        let supported =
            supported_scenes(&self.widget_registry, &self.hardware_capabilities, scenes);
        info!(
            count = supported.len(),
            "spawning widgets for enabled scenes"
        );
        for scene in supported {
            self.spawn_scene_widgets(scene).await;
        }

        self.refresh_scene_cycling(scenes);

        if let Some(scene) =
            first_supported_active_scene(&self.widget_registry, &self.hardware_capabilities, scenes)
        {
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

    pub async fn spawn_scene_widgets(&self, scene: &Scene) {
        if !self.scene_supported(scene) {
            debug!(scene_id = %scene.id, "skipping unsupported scene at spawn");
            return;
        }

        info!(
            scene_id = %scene.id,
            widget_count = scene.widgets.len(),
            "spawning scene widgets"
        );

        for widget in scene.widgets.values() {
            self.spawn_widget(&scene.id, widget).await;
        }
    }

    pub async fn spawn_widget(&self, scene_id: &crate::scene::SceneId, widget: &Widget) {
        let instance_id = widget.id.as_uuid().to_string();

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

        // Register widget with compositor before spawning.
        // This call blocks until the compositor has stored the initial config
        // — otherwise a fast-starting widget could reach `get_widget_surface`
        // before the compositor knows what to emit.
        if let Err(e) =
            self.compositor
                .register_widget(instance_id.clone(), position, viewport, initial_config)
        {
            warn!(
                scene_id = %scene_id,
                widget_id = %instance_id,
                error = %e,
                "failed to register widget with compositor"
            );
            return;
        }

        let Some(wayland_display) = self.compositor.wayland_display() else {
            warn!(
                scene_id = %scene_id,
                widget_id = %instance_id,
                "compositor not started, cannot spawn widget"
            );
            let _ = self.compositor.unregister_widget(&instance_id);
            return;
        };

        let widget_env = WidgetEnv {
            instance_id: instance_id.clone(),
            wayland_display,
        };

        info!(
            scene_id = %scene_id,
            widget_id = %instance_id,
            widget_type = %widget.widget_type_id,
            placement = ?widget.placement,
            "spawning widget"
        );

        let spawned = match self
            .widget_manager
            .spawn_widget(widget.widget_type_id, widget_env)
            .await
        {
            Ok(spawned) => spawned,
            Err(e) => {
                warn!(
                    scene_id = %scene_id,
                    widget_id = %instance_id,
                    widget_type = %widget.widget_type_id,
                    error = %e,
                    "failed to spawn widget"
                );
                let _ = self.compositor.unregister_widget(&instance_id);
                return;
            }
        };

        let pid = spawned.pid;
        if let Err(e) = self.compositor.set_widget_pid(&instance_id, pid) {
            warn!(
                scene_id = %scene_id,
                widget_id = %instance_id,
                pid,
                error = %e,
                "failed to associate pid with widget; widget may not receive initial state"
            );
        }

        self.record_spawn(
            instance_id,
            *scene_id,
            widget.widget_type_id,
            spawned.identity,
        );
    }

    pub async fn stop_widget(&self, instance_id: &str) {
        self.forget_spawn(instance_id);
        self.widget_manager.stop_widget(instance_id).await;
        let _ = self.compositor.unregister_widget(&instance_id.to_owned());
    }

    pub fn update_widget_params(
        &self,
        instance_id: &str,
        params: &BTreeMap<ParamKey, ParamValue>,
    ) -> Result<(), CompositorError> {
        self.compositor
            .update_widget_params(&instance_id.to_owned(), params_to_json_map(params))
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
        self.compositor.update_widget_credentials(
            &widget.id.as_uuid().to_string(),
            resolved.view,
            resolved.secrets,
        )
    }

    pub async fn stop_scene_widgets(&self, scene: &Scene) {
        for widget in scene.widgets.values() {
            self.stop_widget(&widget.id.as_uuid().to_string()).await;
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
        self.widget_manager.stop_all().await;
        if let Err(e) = self.compositor.shutdown() {
            warn!(error = %e, "failed to shut down compositor");
        }
        info!("shutdown complete");
    }

    /// Stop all widget processes while keeping the compositor running,
    /// applying the same per-instance cleanup as [`Self::stop_widget`] so
    /// widgets respawned later do not see stale compositor registrations.
    pub async fn stop_all_widgets(&self) {
        self.forget_all_spawns();
        for instance_id in self.widget_manager.stop_all().await {
            let _ = self.compositor.unregister_widget(&instance_id);
        }
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
        self.coordinator.stop_all_widgets().await;
    }

    async fn restart_widgets(&self) {
        let config = self.config_handle.read().await;
        self.coordinator
            .spawn_all_scene_widgets(config.scenes())
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
    use crate::widget::{ViewportDescriptor, WidgetIdentity, WidgetInfo};
    use bmc_widget_manifest::ViewportShape;

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

    fn reload_registry(uid: uuid::Uuid, path: &str, version: u64) -> WidgetRegistry {
        let manifest = bmc_widget_manifest::Manifest {
            uid,
            version: semver::Version::new(version, 0, 0),
            name: "reload-test".to_owned(),
            subname: None,
            description: "reload test".to_owned(),
            config_help: None,
            author: None,
            binary: std::path::PathBuf::from("widget"),
            icon: None,
            category: bmc_widget_manifest::WidgetCategory::Misc,
            settings: vec![],
            supported_viewports: vec![],
            params: indexmap::IndexMap::new(),
            credentials: indexmap::IndexMap::new(),
        };
        WidgetRegistry::new(vec![WidgetInfo::for_test(
            manifest,
            std::path::PathBuf::from(path),
            std::path::PathBuf::from(path).join("widget"),
            None,
        )])
    }

    fn reload_record(scene_id: SceneId, uid: uuid::Uuid, path: &str, version: u64) -> SpawnRecord {
        SpawnRecord {
            scene_id,
            widget_uid: uid,
            identity: WidgetIdentity {
                canonical_dir: std::path::PathBuf::from(path),
                version: semver::Version::new(version, 0, 0),
            },
        }
    }

    #[test]
    fn only_a_present_changed_identity_requests_reload() {
        let uid = uuid::Uuid::new_v4();
        let scene_id = SceneId::generate();
        let unchanged = reload_record(scene_id, uid, "/widgets/current", 1);
        assert!(!record_needs_reload(
            &reload_registry(uid, "/widgets/current", 1),
            &unchanged
        ));
        assert!(record_needs_reload(
            &reload_registry(uid, "/widgets/replaced", 1),
            &unchanged
        ));
        assert!(record_needs_reload(
            &reload_registry(uid, "/widgets/current", 2),
            &unchanged
        ));
        assert!(!record_needs_reload(
            &WidgetRegistry::new(vec![]),
            &unchanged
        ));
    }

    #[test]
    fn reload_resolves_the_current_widget_from_its_recorded_scene_and_uid() {
        let uid = uuid::Uuid::new_v4();
        let mut scene = Scene::fullscreen(uid, BTreeMap::new());
        let widget = scene.widgets.values().next().expect("BUG: widget").clone();
        let instance_id = widget.id.to_string();
        let record = reload_record(scene.id, uid, "/widgets/old", 1);
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
