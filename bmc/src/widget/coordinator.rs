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
use bmc_widget_manifest::{Manifest, ParamKey, ParamValue};
use bmc_widget_protocol::{
    Localization, NextAlarm, SettingUpdate, ViewportShape, WidgetInitialConfig,
};
use futures::future::join_all;
use indexmap::IndexMap;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tracing::{debug, error, info, warn};

use crate::BmcManager;
use crate::compositor::{
    Compositor, CompositorError, CompositorReceipt, CredentialUpdateReceipt, HardwareCapabilities,
    InstanceId, Position, SceneLayout, Size, WidgetConnectionMode, WidgetInstanceKey,
    WidgetPlacement, WidgetRegistration,
};
use crate::config::ConfigHandle;
use crate::config::LocalizationConfig;
use crate::credential;
use crate::data::{Account, AccountId};
use crate::scene::{Scene, SceneId, Widget, WidgetPosition};
use crate::secret_store::SecretStoreHandle;
use bmc_net::NetworkManager;

use super::manager::{ManagedWidgetState, ManagerMode, StartError, StartPermit, WidgetLaunch};
use super::{WidgetInfo, WidgetManager, WidgetRegistry};

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

fn supported_shown_scenes<'a>(
    registry: &WidgetRegistry,
    caps: &HardwareCapabilities,
    scenes: &'a IndexMap<SceneId, Scene>,
    preview_scene_id: Option<SceneId>,
) -> Vec<&'a Scene> {
    scenes
        .values()
        .filter(|scene| {
            (scene.enabled || preview_scene_id == Some(scene.id))
                && scene_supported_with_registry(registry, scene, caps)
        })
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

fn widget_spawn_prerequisites_unchanged(current: &Widget, expected: &Widget) -> bool {
    current.id == expected.id
        && current.placement == expected.placement
        && current.widget_type_id == expected.widget_type_id
        && current.viewport_shape == expected.viewport_shape
        && current.credential_bindings == expected.credential_bindings
}

fn configured_scene_is_visible(scene: &Scene, preview_scene_id: Option<SceneId>) -> bool {
    scene.enabled || preview_scene_id == Some(scene.id)
}

pub struct Coordinator {
    widget_manager: WidgetManager,
    compositor: Arc<dyn Compositor>,
    wayland_display: Option<String>,
    widget_registry: Arc<WidgetRegistry>,
    hardware_capabilities: HardwareCapabilities,
    preview_scene_id: Arc<Mutex<Option<SceneId>>>,
    /// Read on every spawn to resolve the widget's credential bindings.
    /// Lock order: config before secrets. A spawner takes the config lock
    /// first or not at all, and never takes it while holding this one.
    secret_store: Arc<RwLock<SecretStoreHandle>>,
}

#[derive(Clone, Copy)]
enum RetainedWidgetStop {
    Deactivate,
    Unregister,
}

pub(crate) struct WidgetStopBatch {
    receipts: Vec<Result<CompositorReceipt, CompositorError>>,
    terminations: Vec<super::manager::TerminationHandle>,
}

pub(crate) struct PendingWidgetRegistration {
    widget_id: crate::scene::WidgetId,
    receipt: Result<CompositorReceipt, CompositorError>,
}

struct PendingCredentialUpdate {
    instance_id: InstanceId,
    receipt: CredentialUpdateReceipt,
}

/// Refresh retained credentials from one consistent configuration/account snapshot.
/// Commands are enqueued under the source locks, but compositor receipts are awaited after release.
pub fn start_credential_listener(
    coordinator: Arc<Coordinator>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    mut scenes_change_rx: broadcast::Receiver<crate::config::WidgetSceneMap>,
    mut accounts_change_rx: broadcast::Receiver<()>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = scenes_change_rx.recv() => {
                    if matches!(result, Err(broadcast::error::RecvError::Closed)) {
                        break;
                    }
                }
                result = accounts_change_rx.recv() => {
                    if matches!(result, Err(broadcast::error::RecvError::Closed)) {
                        break;
                    }
                }
            }

            let pending = {
                let config = config_handle.read().await;
                let accounts = coordinator.secret_store.read().await;
                while matches!(
                    scenes_change_rx.try_recv(),
                    Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_))
                ) {}
                while matches!(
                    accounts_change_rx.try_recv(),
                    Ok(()) | Err(broadcast::error::TryRecvError::Lagged(_))
                ) {}
                coordinator.enqueue_configured_widget_credentials(&config, accounts.accounts())
            };

            coordinator.finish_credential_refreshes(pending).await;
        }
    });
}

impl WidgetStopBatch {
    pub(crate) async fn wait(self) {
        let receipts = async {
            for result in wait_for_compositor_receipts(self.receipts).await {
                if let Err(error) = result {
                    warn!(%error, "failed to apply widget cutoff");
                }
            }
        };
        let terminations = async {
            for termination in self.terminations {
                termination.join().await;
            }
        };
        tokio::join!(receipts, terminations);
    }
}

async fn wait_for_compositor_receipts(
    receipts: Vec<Result<CompositorReceipt, CompositorError>>,
) -> Vec<Result<(), CompositorError>> {
    join_all(receipts.into_iter().map(|receipt| async {
        match receipt {
            Ok(receipt) => receipt.wait().await,
            Err(error) => Err(error),
        }
    }))
    .await
}

async fn wait_for_deactivations(
    receipts: Vec<Result<CompositorReceipt, CompositorError>>,
    phase: &'static str,
) {
    for result in wait_for_compositor_receipts(receipts).await {
        if let Err(error) = result {
            warn!(%error, phase, "failed to deactivate widget");
        }
    }
}

enum WidgetStartPreparation {
    Satisfied,
    Cutoff(WidgetStopBatch),
    WaitForTermination(super::manager::TerminationHandle),
    Registered {
        widget_id: crate::scene::WidgetId,
        registration: CompositorReceipt,
    },
    Ready(Box<PreparedWidgetStart>),
}

struct PreparedWidgetStart {
    widget: Widget,
    launch: WidgetLaunch,
    identity: super::WidgetIdentity,
    permit: StartPermit,
    registration: CompositorReceipt,
    activation: CompositorReceipt,
}

struct ResumeWidget {
    scene_id: SceneId,
    widget: Widget,
    launch: WidgetLaunch,
    identity: super::WidgetIdentity,
    permit: StartPermit,
}

struct PendingResumeWidget {
    widget: ResumeWidget,
    activation: Result<CompositorReceipt, CompositorError>,
}

enum StartValidation {
    Start(super::manager::StartHandle),
    Retry,
    Abort,
}

impl std::fmt::Debug for Coordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coordinator")
            .field("widget_manager", &self.widget_manager)
            .finish_non_exhaustive()
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
    #[cfg(test)]
    pub(crate) async fn running_widget_count(&self) -> usize {
        self.widget_manager
            .snapshot()
            .await
            .widgets
            .iter()
            .filter(|widget| widget.state == ManagedWidgetState::Running)
            .count()
    }

    #[cfg(test)]
    pub(crate) async fn shutdown_widget_manager(&self) {
        self.widget_manager.shutdown().await;
    }

    pub fn new(
        widget_manager: WidgetManager,
        compositor: Arc<dyn Compositor>,
        wayland_display: Option<String>,
        widget_registry: Arc<WidgetRegistry>,
        hardware_capabilities: HardwareCapabilities,
        secret_store: Arc<RwLock<SecretStoreHandle>>,
    ) -> Self {
        Self {
            widget_manager,
            compositor,
            wayland_display,
            widget_registry,
            hardware_capabilities,
            preview_scene_id: Arc::default(),
            secret_store,
        }
    }

    pub(crate) fn preview_scene_state(&self) -> Arc<Mutex<Option<SceneId>>> {
        Arc::clone(&self.preview_scene_id)
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

        let configured = {
            let config = config_handle.read().await;
            config
                .scenes()
                .values()
                .flat_map(|scene| {
                    scene
                        .widgets
                        .values()
                        .map(move |widget| (scene.id, widget.clone()))
                })
                .collect::<Vec<_>>()
        };

        for (scene_id, widget) in configured {
            let instance_id = widget.id.to_string();
            let Some(installed) = self.widget_registry.get(&widget.widget_type_id) else {
                continue;
            };
            let observed = manager_snapshot
                .widgets
                .iter()
                .find(|managed| managed.launch.config_key.instance_id == widget.id.as_uuid());
            match observed {
                Some(managed) if managed.state == ManagedWidgetState::Stopping => continue,
                Some(managed)
                    if managed.launch.config_key.scene_id == scene_id
                        && managed.launch.config_key.widget_uid == widget.widget_type_id
                        && managed.identity == installed.identity =>
                {
                    continue;
                }
                Some(_) => {
                    info!(%instance_id, widget_type = %widget.widget_type_id, "reloading changed widget");
                    self.replace_configured_widget(config_handle, scene_id, widget.id)
                        .await;
                }
                None => {
                    self.spawn_configured_widget(config_handle, scene_id, widget.id)
                        .await;
                }
            }
        }

        self.refresh_configured_widget_credentials(config_handle)
            .await;

        info!("finished handling widget reload");
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

        info!("finished initial widget reconciliation");
    }

    pub(crate) async fn spawn_all_configured_widgets(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
    ) {
        let scene_ids = {
            let config = config_handle.read().await;
            config.scenes().keys().copied().collect::<Vec<_>>()
        };
        for scene_id in scene_ids {
            self.spawn_configured_scene_widgets(config_handle, scene_id)
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
    ) {
        let widget_ids = {
            let config = config_handle.read().await;
            let Some(scene) = config.scenes().get(&scene_id) else {
                return;
            };
            scene.widgets.keys().copied().collect::<Vec<_>>()
        };
        for widget_id in widget_ids {
            self.spawn_configured_widget(config_handle, scene_id, widget_id)
                .await;
        }
    }

    pub(crate) async fn spawn_configured_widget(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        scene_id: SceneId,
        widget_id: crate::scene::WidgetId,
    ) {
        let mut mismatch_attempt = 0;
        loop {
            let prepared = match self
                .prepare_widget_start(config_handle, scene_id, widget_id, mismatch_attempt)
                .await
            {
                WidgetStartPreparation::Ready(prepared) => prepared,
                WidgetStartPreparation::Cutoff(cutoff) => {
                    mismatch_attempt += 1;
                    cutoff.wait().await;
                    continue;
                }
                WidgetStartPreparation::WaitForTermination(termination) => {
                    termination.join().await;
                    continue;
                }
                WidgetStartPreparation::Registered {
                    widget_id,
                    registration,
                } => {
                    if let Err(error) = registration.wait().await {
                        warn!(%scene_id, %widget_id, %error, "failed to apply widget registration");
                    }
                    return;
                }
                WidgetStartPreparation::Satisfied => return,
            };

            let (registration_result, activation_result) =
                tokio::join!(prepared.registration.wait(), prepared.activation.wait());
            if let Err(error) = registration_result.and(activation_result) {
                warn!(%scene_id, widget_id = %prepared.widget.id, %error, "failed to apply widget registration and activation");
                return;
            }

            let start = match self
                .enqueue_validated_start(
                    config_handle,
                    scene_id,
                    &prepared.widget,
                    &prepared.launch,
                    &prepared.identity,
                    prepared.permit,
                )
                .await
            {
                StartValidation::Start(start) => start,
                StartValidation::Retry => continue,
                StartValidation::Abort => return,
            };

            match start.join().await {
                Ok(()) | Err(StartError::Superseded) => return,
                Err(StartError::Occupied(_) | StartError::RegistryChanged) => continue,
                Err(StartError::PendingRestart(error)) => {
                    warn!(
                        %scene_id,
                        widget_id = %prepared.widget.id,
                        widget_type = %prepared.widget.widget_type_id,
                        %error,
                        "failed to spawn widget; retaining registration for retry"
                    );
                    return;
                }
                Err(error @ StartError::Mode(_)) => {
                    warn!(
                        %scene_id,
                        widget_id = %prepared.widget.id,
                        widget_type = %prepared.widget.widget_type_id,
                        %error,
                        "failed to spawn widget"
                    );
                    return;
                }
            }
        }
    }

    pub(crate) async fn finish_widget_registration(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        scene_id: SceneId,
        pending: PendingWidgetRegistration,
    ) {
        let widget_id = pending.widget_id;
        let result = match pending.receipt {
            Ok(receipt) => receipt.wait().await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            warn!(%scene_id, %widget_id, %error, "failed to apply widget registration");
            return;
        }
        self.spawn_configured_widget(config_handle, scene_id, widget_id)
            .await;
    }

    async fn prepare_widget_start(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        scene_id: SceneId,
        widget_id: crate::scene::WidgetId,
        mismatch_attempt: usize,
    ) -> WidgetStartPreparation {
        let preview = self.preview_scene_id.lock().await;
        let config = config_handle.read().await;
        let Some(scene) = config.scenes().get(&scene_id) else {
            return WidgetStartPreparation::Satisfied;
        };
        let Some(widget) = scene.widgets.get(&widget_id).cloned() else {
            return WidgetStartPreparation::Satisfied;
        };
        let Some((installed, viewport)) = self.widget_registration_prerequisites(&widget) else {
            return WidgetStartPreparation::Satisfied;
        };
        let identity = installed.identity.clone();
        let should_start = configured_scene_is_visible(scene, *preview)
            && self.scene_supported(scene)
            && self.widget_manager.mode() == ManagerMode::Running;
        if !should_start {
            return self
                .prepare_inactive_registration(scene_id, widget, &installed.manifest, viewport)
                .await;
        }
        let Some(wayland_display) = self.wayland_display.clone() else {
            warn!(%scene_id, widget_id = %widget.id, "compositor not started, cannot spawn widget");
            return WidgetStartPreparation::Satisfied;
        };
        let launch = widget_launch(scene_id, &widget, wayland_display);
        if let Some(managed) = self
            .widget_manager
            .snapshot()
            .await
            .widgets
            .into_iter()
            .find(|managed| managed.launch.config_key.instance_id == widget.id.as_uuid())
        {
            if managed.state == ManagedWidgetState::Stopping && managed.launch == launch {
                return WidgetStartPreparation::WaitForTermination(
                    self.widget_manager
                        .stop_widget(&widget.id.to_string())
                        .await,
                );
            }
            if managed.launch == launch && managed.identity == identity {
                return WidgetStartPreparation::Satisfied;
            }
            if mismatch_attempt > 0 {
                error!(widget_id = %widget.id, "widget launch remained occupied after bounded replacement retry");
                return WidgetStartPreparation::Satisfied;
            }
            return WidgetStartPreparation::Cutoff(
                self.enqueue_widget_stop(
                    WidgetInstanceKey::new(widget.id.as_uuid()),
                    RetainedWidgetStop::Deactivate,
                )
                .await,
            );
        }
        let Ok(permit) = self
            .widget_manager
            .prepare_start(&widget.id.to_string(), ManagerMode::Running)
            .await
        else {
            return self
                .prepare_inactive_registration(scene_id, widget, &installed.manifest, viewport)
                .await;
        };
        let registration = match self
            .enqueue_widget_registration(&widget, &installed.manifest, viewport)
            .await
        {
            Ok(registration) => registration,
            Err(error) => {
                warn!(%scene_id, widget_id = %widget.id, %error, "failed to enqueue widget registration");
                return WidgetStartPreparation::Satisfied;
            }
        };
        let activation = match self
            .compositor
            .enqueue_activate_widget(WidgetInstanceKey::new(widget.id.as_uuid()))
        {
            Ok(activation) => activation,
            Err(error) => {
                warn!(%scene_id, widget_id = %widget.id, %error, "failed to enqueue widget activation");
                return WidgetStartPreparation::Satisfied;
            }
        };
        WidgetStartPreparation::Ready(Box::new(PreparedWidgetStart {
            widget,
            launch,
            identity,
            permit,
            registration,
            activation,
        }))
    }

    async fn prepare_inactive_registration(
        &self,
        scene_id: SceneId,
        widget: Widget,
        manifest: &Manifest,
        viewport: Size,
    ) -> WidgetStartPreparation {
        match self
            .enqueue_widget_registration(&widget, manifest, viewport)
            .await
        {
            Ok(registration) => WidgetStartPreparation::Registered {
                widget_id: widget.id,
                registration,
            },
            Err(error) => {
                warn!(%scene_id, widget_id = %widget.id, %error, "failed to enqueue widget registration");
                WidgetStartPreparation::Satisfied
            }
        }
    }

    async fn enqueue_validated_start(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        scene_id: SceneId,
        expected: &Widget,
        launch: &WidgetLaunch,
        identity: &super::WidgetIdentity,
        expected_permit: StartPermit,
    ) -> StartValidation {
        let preview = self.preview_scene_id.lock().await;
        let config = config_handle.read().await;
        let Some(scene) = config.scenes().get(&scene_id) else {
            return StartValidation::Abort;
        };
        if !configured_scene_is_visible(scene, *preview) || !self.scene_supported(scene) {
            return StartValidation::Abort;
        }
        let Some(current) = scene.widgets.get(&expected.id) else {
            return StartValidation::Abort;
        };
        if self.widget_manager.mode() != ManagerMode::Running {
            return StartValidation::Abort;
        }
        if !widget_spawn_prerequisites_unchanged(current, expected) {
            return StartValidation::Retry;
        }
        if self
            .widget_registry
            .get(&current.widget_type_id)
            .is_none_or(|current| current.identity != *identity)
        {
            return StartValidation::Retry;
        }
        StartValidation::Start(
            self.widget_manager
                .enqueue_spawn_widget(launch.clone(), identity.clone(), expected_permit)
                .await,
        )
    }

    fn widget_registration_prerequisites(&self, widget: &Widget) -> Option<(WidgetInfo, Size)> {
        if widget.widget_type_id.is_nil() {
            info!(widget_id = %widget.id, "skipping widget with nil widget_type_id");
            return None;
        }
        let installed = self.widget_registry.get(&widget.widget_type_id)?;
        let descriptor = placement_viewport_descriptor(widget, &self.hardware_capabilities)?;
        if !installed.supports_viewport(&descriptor) {
            warn!(widget_id = %widget.id, "widget type is missing or does not support its configured viewport");
            return None;
        }
        let fullscreen_descriptor =
            fullscreen_descriptor_for_widget(widget, &self.hardware_capabilities);
        let viewport = placement_viewport_size(&widget.placement, &fullscreen_descriptor)?;
        Some((installed, viewport))
    }

    fn widget_registration_from_accounts(
        &self,
        widget: &Widget,
        manifest: &Manifest,
        viewport: Size,
        accounts: &IndexMap<AccountId, Account>,
    ) -> WidgetRegistration {
        let resolved = Self::resolve_credentials_for(widget, manifest, accounts);
        WidgetRegistration {
            key: WidgetInstanceKey::new(widget.id.as_uuid()),
            connection_mode: WidgetConnectionMode::Inactive,
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
        }
    }

    async fn enqueue_widget_registration(
        &self,
        widget: &Widget,
        manifest: &Manifest,
        viewport: Size,
    ) -> Result<CompositorReceipt, CompositorError> {
        let accounts = self.secret_store.read().await;
        self.compositor
            .enqueue_register_widget(self.widget_registration_from_accounts(
                widget,
                manifest,
                viewport,
                accounts.accounts(),
            ))
    }

    pub(crate) fn enqueue_configured_widget_registration(
        &self,
        config: &ConfigHandle,
        accounts: &IndexMap<AccountId, Account>,
        scene_id: SceneId,
        widget_id: crate::scene::WidgetId,
    ) -> Option<PendingWidgetRegistration> {
        let widget = config.scenes().get(&scene_id)?.widgets.get(&widget_id)?;
        let (installed, viewport) = self.widget_registration_prerequisites(widget)?;
        Some(PendingWidgetRegistration {
            widget_id,
            receipt: self.compositor.enqueue_register_widget(
                self.widget_registration_from_accounts(
                    widget,
                    &installed.manifest,
                    viewport,
                    accounts,
                ),
            ),
        })
    }

    pub(crate) fn enqueue_configured_scene_registrations(
        &self,
        config: &ConfigHandle,
        accounts: &IndexMap<AccountId, Account>,
        scene_id: SceneId,
    ) -> Vec<PendingWidgetRegistration> {
        let Some(scene) = config.scenes().get(&scene_id) else {
            return Vec::new();
        };
        scene
            .widgets
            .keys()
            .filter_map(|widget_id| {
                self.enqueue_configured_widget_registration(config, accounts, scene_id, *widget_id)
            })
            .collect()
    }

    async fn enqueue_widget_stop(
        &self,
        key: WidgetInstanceKey,
        stop: RetainedWidgetStop,
    ) -> WidgetStopBatch {
        let receipt = match stop {
            RetainedWidgetStop::Deactivate => self.compositor.enqueue_deactivate_widget(key),
            RetainedWidgetStop::Unregister => self.compositor.enqueue_unregister_widget(key),
        };
        let termination = self.widget_manager.stop_widget(&key.to_string()).await;
        WidgetStopBatch {
            receipts: vec![receipt],
            terminations: vec![termination],
        }
    }

    pub(crate) async fn enqueue_scene_stop(&self, scene: &Scene) -> WidgetStopBatch {
        self.enqueue_scene_cutoff(scene, RetainedWidgetStop::Deactivate)
            .await
    }

    pub(crate) async fn enqueue_scene_delete(&self, scene: &Scene) -> WidgetStopBatch {
        self.enqueue_scene_cutoff(scene, RetainedWidgetStop::Unregister)
            .await
    }

    async fn enqueue_scene_cutoff(
        &self,
        scene: &Scene,
        stop: RetainedWidgetStop,
    ) -> WidgetStopBatch {
        let mut receipts = Vec::with_capacity(scene.widgets.len());
        let mut terminations = Vec::with_capacity(scene.widgets.len());
        for widget in scene.widgets.values() {
            let batch = self
                .enqueue_widget_stop(WidgetInstanceKey::new(widget.id.as_uuid()), stop)
                .await;
            receipts.extend(batch.receipts);
            terminations.extend(batch.terminations);
        }
        WidgetStopBatch {
            receipts,
            terminations,
        }
    }

    pub(crate) async fn enqueue_widget_delete(&self, key: WidgetInstanceKey) -> WidgetStopBatch {
        self.enqueue_widget_stop(key, RetainedWidgetStop::Unregister)
            .await
    }

    pub(crate) async fn enqueue_widget_replacement(
        &self,
        key: WidgetInstanceKey,
    ) -> WidgetStopBatch {
        self.enqueue_widget_stop(key, RetainedWidgetStop::Deactivate)
            .await
    }

    pub(crate) async fn replace_configured_widget(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        scene_id: SceneId,
        widget_id: crate::scene::WidgetId,
    ) {
        let cutoff = {
            let config = config_handle.read().await;
            let Some(scene) = config.scenes().get(&scene_id) else {
                return;
            };
            let Some(widget) = scene.widgets.get(&widget_id) else {
                return;
            };
            self.enqueue_widget_replacement(WidgetInstanceKey::new(widget.id.as_uuid()))
                .await
        };
        cutoff.wait().await;
        self.spawn_configured_widget(config_handle, scene_id, widget_id)
            .await;
    }

    pub fn update_widget_params(
        &self,
        key: WidgetInstanceKey,
        params: &BTreeMap<ParamKey, ParamValue>,
    ) -> Result<(), CompositorError> {
        self.compositor
            .update_widget_params(key, params_to_json_map(params))
    }

    /// Ask a crash-looping widget to try the configuration just pushed to it,
    /// rather than sit out a delay earned against the configuration it replaced.
    pub async fn retry_pending_widget(&self, instance_id: &str) -> bool {
        self.widget_manager.retry_pending(instance_id).await
    }

    fn resolve_credentials_for(
        widget: &Widget,
        manifest: &Manifest,
        accounts: &IndexMap<AccountId, Account>,
    ) -> credential::Resolution {
        let (authorised, unauthorised) = credential::authorised_bindings(
            &widget.credential_bindings,
            &manifest.credentials,
            accounts,
        );
        for (slot, why) in unauthorised {
            warn!(
                widget_id = %widget.id,
                slot = slot.as_str(),
                reason = why.reason(),
                "withholding a credential the installed manifest no longer authorises"
            );
        }

        for (slot, account) in credential::dangling_bindings(&authorised, accounts) {
            warn!(
                widget_id = %widget.id,
                slot = slot.as_str(),
                account = %account,
                "credential slot bound to an account that no longer exists; treating it as unbound"
            );
        }

        credential::resolve(&authorised, accounts)
    }

    #[cfg(test)]
    pub(crate) async fn resolve_credentials(
        &self,
        widget: &Widget,
    ) -> Option<credential::Resolution> {
        let installed = self.widget_registry.get(&widget.widget_type_id)?;
        let store = self.secret_store.read().await;
        Some(Self::resolve_credentials_for(
            widget,
            &installed.manifest,
            store.accounts(),
        ))
    }

    fn enqueue_widget_credentials(
        &self,
        widget: &Widget,
        accounts: &IndexMap<AccountId, Account>,
    ) -> Result<Option<PendingCredentialUpdate>, CompositorError> {
        let Some(installed) = self.widget_registry.get(&widget.widget_type_id) else {
            warn!(
                widget_id = %widget.id,
                widget_type_id = %widget.widget_type_id,
                "skipping credential refresh because the installed manifest is unavailable"
            );
            return Ok(None);
        };
        let resolved = Self::resolve_credentials_for(widget, &installed.manifest, accounts);
        let instance_id = widget.id.as_uuid().to_string();
        let receipt = self.compositor.enqueue_update_widget_credentials(
            WidgetInstanceKey::new(widget.id.as_uuid()),
            resolved.view,
            resolved.secrets,
        )?;
        Ok(Some(PendingCredentialUpdate {
            instance_id,
            receipt,
        }))
    }

    async fn finish_widget_credentials_update(
        &self,
        pending: PendingCredentialUpdate,
    ) -> Result<Option<InstanceId>, CompositorError> {
        let changed = pending.receipt.wait().await?;
        Ok(changed.then_some(pending.instance_id))
    }

    fn enqueue_configured_widget_credentials(
        &self,
        config: &ConfigHandle,
        accounts: &IndexMap<AccountId, Account>,
    ) -> Vec<PendingCredentialUpdate> {
        let mut seen = HashSet::new();
        config
            .scenes()
            .values()
            .flat_map(|scene| scene.widgets.values())
            .filter(|widget| seen.insert(widget.id))
            .filter_map(
                |widget| match self.enqueue_widget_credentials(widget, accounts) {
                    Ok(update) => update,
                    Err(error) => {
                        warn!(
                            widget_id = %widget.id,
                            %error,
                            "failed to enqueue widget credential refresh"
                        );
                        None
                    }
                },
            )
            .collect()
    }

    async fn finish_credential_refreshes(
        &self,
        pending: Vec<PendingCredentialUpdate>,
    ) -> Vec<InstanceId> {
        let results = join_all(
            pending
                .into_iter()
                .map(|update| self.finish_widget_credentials_update(update)),
        )
        .await;
        let mut retried = Vec::new();
        for result in results {
            match result {
                Ok(Some(instance_id)) => {
                    if self.retry_pending_widget(&instance_id).await {
                        retried.push(instance_id);
                    }
                }
                Ok(None) => {}
                Err(error) => warn!(%error, "failed to finish widget credential refresh"),
            }
        }
        retried
    }

    async fn refresh_configured_widget_credentials(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
    ) {
        let pending = {
            let config = config_handle.read().await;
            let accounts = self.secret_store.read().await;
            self.enqueue_configured_widget_credentials(&config, accounts.accounts())
        };
        self.finish_credential_refreshes(pending).await;
    }

    /// Stop all widget processes and shut down the compositor.
    ///
    /// Widgets are stopped first (SIGTERM → 10s timeout → SIGKILL) because they
    /// need the Wayland display socket to clean up GPU resources (GEM/DMA-BUF).
    /// The compositor is shut down second.
    pub async fn stop_all(&self, config_handle: &Arc<RwLock<ConfigHandle>>) {
        info!("stopping all widgets and compositor");
        let shutdown = self.widget_manager.begin_shutdown().await;
        self.finish_lifecycle_stop(config_handle, shutdown, "shutdown")
            .await;
        if let Err(e) = self.compositor.shutdown() {
            warn!(error = %e, "failed to shut down compositor");
        }
        info!("shutdown complete");
    }

    /// Stop every widget process while retaining inactive compositor registrations.
    pub async fn stop_all_widgets(&self, config_handle: &Arc<RwLock<ConfigHandle>>) {
        let pause = self.widget_manager.begin_pause().await;
        self.finish_lifecycle_stop(config_handle, pause, "upgrade pause")
            .await;
    }

    async fn finish_lifecycle_stop(
        &self,
        config_handle: &Arc<RwLock<ConfigHandle>>,
        stop: super::manager::StopAllHandle,
        phase: &'static str,
    ) {
        let mut keys = stop
            .instance_ids()
            .iter()
            .filter_map(|instance_id| instance_id.parse::<WidgetInstanceKey>().ok())
            .collect::<BTreeSet<_>>();
        let receipts = {
            let config = config_handle.write().await;
            keys.extend(config.scenes().values().flat_map(|scene| {
                scene
                    .widgets
                    .values()
                    .map(|widget| WidgetInstanceKey::new(widget.id.as_uuid()))
            }));
            keys.into_iter()
                .map(|key| self.compositor.enqueue_deactivate_widget(key))
                .collect::<Vec<_>>()
        };
        tokio::join!(stop.join(), wait_for_deactivations(receipts, phase));
    }

    async fn prepare_resume_widgets(
        &self,
        config: &ConfigHandle,
        preview_scene_id: Option<SceneId>,
    ) -> Option<Vec<PendingResumeWidget>> {
        let Some(wayland_display) = self.wayland_display.clone() else {
            warn!("compositor not started, cannot resume widgets");
            return None;
        };
        let mut pending = Vec::new();
        for scene in supported_shown_scenes(
            &self.widget_registry,
            &self.hardware_capabilities,
            config.scenes(),
            preview_scene_id,
        ) {
            for widget in scene.widgets.values() {
                let Some((installed, _)) = self.widget_registration_prerequisites(widget) else {
                    continue;
                };
                let key = WidgetInstanceKey::new(widget.id.as_uuid());
                let Ok(permit) = self
                    .widget_manager
                    .prepare_start(&widget.id.to_string(), ManagerMode::Paused)
                    .await
                else {
                    return None;
                };
                pending.push(PendingResumeWidget {
                    widget: ResumeWidget {
                        scene_id: scene.id,
                        widget: widget.clone(),
                        launch: widget_launch(scene.id, widget, wayland_display.clone()),
                        identity: installed.identity,
                        permit,
                    },
                    activation: self.compositor.enqueue_activate_widget(key),
                });
            }
        }
        Some(pending)
    }

    fn resume_widgets_unchanged(
        &self,
        config: &ConfigHandle,
        preview_scene_id: Option<SceneId>,
        expected: &[(ResumeWidget, bool)],
    ) -> bool {
        let current = supported_shown_scenes(
            &self.widget_registry,
            &self.hardware_capabilities,
            config.scenes(),
            preview_scene_id,
        )
        .into_iter()
        .flat_map(|scene| {
            scene.widgets.values().filter_map(move |widget| {
                let (installed, _) = self.widget_registration_prerequisites(widget)?;
                Some((scene.id, widget, installed.identity))
            })
        })
        .collect::<Vec<_>>();
        current.len() == expected.len()
            && current
                .iter()
                .zip(expected)
                .all(|((scene_id, current, identity), (expected, _))| {
                    *scene_id == expected.scene_id
                        && widget_spawn_prerequisites_unchanged(current, &expected.widget)
                        && *identity == expected.identity
                })
    }

    pub(crate) async fn resume_all_widgets(&self, config_handle: &Arc<RwLock<ConfigHandle>>) {
        loop {
            let Some(pending) = ({
                let preview = self.preview_scene_id.lock().await;
                let config = config_handle.read().await;
                if self.widget_manager.mode() != ManagerMode::Paused {
                    return;
                }
                self.prepare_resume_widgets(&config, *preview).await
            }) else {
                return;
            };
            let prepared = futures::future::join_all(pending.into_iter().map(|pending| async {
                let applied = match pending.activation {
                    Ok(receipt) => receipt.wait().await,
                    Err(error) => Err(error),
                };
                let applied = match applied {
                    Ok(()) => true,
                    Err(error) => {
                        warn!(widget_id = %pending.widget.widget.id, %error, "failed to prepare widget resume");
                        false
                    }
                };
                (pending.widget, applied)
            }))
            .await;

            let starts = {
                let preview = self.preview_scene_id.lock().await;
                let config = config_handle.read().await;
                if self.widget_manager.mode() != ManagerMode::Paused {
                    return;
                }
                if !self.resume_widgets_unchanged(&config, *preview, &prepared) {
                    continue;
                }
                if self.widget_manager.resume().await != ManagerMode::Running {
                    return;
                }
                self.refresh_scene_cycling(config.scenes());
                if let Some(scene) = first_supported_active_scene(
                    &self.widget_registry,
                    &self.hardware_capabilities,
                    config.scenes(),
                ) {
                    self.set_active_scene(scene);
                }
                let mut starts = Vec::new();
                for (widget, applied) in prepared {
                    if applied {
                        starts.push((
                            widget.scene_id,
                            widget.widget.id,
                            self.widget_manager
                                .enqueue_spawn_widget(widget.launch, widget.identity, widget.permit)
                                .await,
                        ));
                    }
                }
                starts
            };
            for (scene_id, widget_id, start) in starts {
                match start.join().await {
                    Ok(()) => {}
                    Err(StartError::Occupied(_) | StartError::RegistryChanged) => {
                        self.spawn_configured_widget(config_handle, scene_id, widget_id)
                            .await;
                    }
                    Err(error) => {
                        warn!(%widget_id, %error, "failed to resume widget");
                    }
                }
            }
            return;
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
    operations: Arc<Mutex<()>>,
}

impl UpgradeWidgetLifecycle {
    pub(crate) fn new(
        coordinator: Arc<Coordinator>,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) -> Self {
        Self {
            coordinator,
            config_handle,
            operations: Arc::new(Mutex::new(())),
        }
    }
}

async fn run_lifecycle_operation(
    operations: Arc<Mutex<()>>,
    operation: impl Future<Output = ()> + Send + 'static,
) {
    let operation_guard = operations.lock_owned().await;
    let task = tokio::spawn(async move {
        let _operation = operation_guard;
        operation.await;
    });
    task.await
        .expect("BUG: widget lifecycle operation task panicked");
}

#[async_trait::async_trait]
impl crate::system_upgrade::WidgetLifecycle for UpgradeWidgetLifecycle {
    async fn stop_all_widgets(&self) {
        let coordinator = Arc::clone(&self.coordinator);
        let config = Arc::clone(&self.config_handle);
        run_lifecycle_operation(Arc::clone(&self.operations), async move {
            coordinator.stop_all_widgets(&config).await;
        })
        .await;
    }

    async fn restart_widgets(&self) {
        let coordinator = Arc::clone(&self.coordinator);
        let config = Arc::clone(&self.config_handle);
        run_lifecycle_operation(Arc::clone(&self.operations), async move {
            coordinator.resume_all_widgets(&config).await;
        })
        .await;
    }

    async fn refresh_widgets(&self) {
        self.coordinator
            .reload_changed_widgets(&self.config_handle)
            .await;
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
        let manager = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let registry = manager.registry();
        let compositor = Arc::new(crate::compositor::testing::RecordingCompositor::default());
        let secret_store = Arc::new(RwLock::new(
            SecretStoreHandle::init(&temp.path().join("config.json")).await,
        ));
        let coordinator = Coordinator::new(
            manager,
            Arc::clone(&compositor) as Arc<dyn Compositor>,
            Some("test-display".to_owned()),
            registry,
            bmc100_capabilities(),
            secret_store,
        );
        let scene = Scene::fullscreen(uid, BTreeMap::new());
        (temp, coordinator, compositor, scene)
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

    fn slow_termination_widget(
        ready: Option<&std::path::Path>,
        shutdown_delay: Duration,
    ) -> String {
        let ready = ready
            .map(|path| format!("touch {}\n", path.display()))
            .unwrap_or_default();
        format!(
            "#!/bin/sh\nchild=\ncleanup() {{\n  if [ -n \"$child\" ]; then\n    kill \"$child\" 2>/dev/null\n    wait \"$child\"\n  fi\n  sleep {}\n  exit 0\n}}\ntrap cleanup TERM\nsleep 30 &\nchild=$!\n{ready}wait \"$child\"\n",
            shutdown_delay.as_secs_f64(),
        )
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

    fn assert_widget_call_kinds(
        compositor: &crate::compositor::testing::RecordingCompositor,
        expected: &[&str],
        message: &str,
    ) {
        let calls = compositor.widget_calls();
        let actual = calls
            .iter()
            .map(|call| call.split_whitespace().next().expect("BUG: recorded call"))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{message}");
    }

    #[tokio::test]
    async fn configured_start_waits_for_active_receipts_before_process_spawn() {
        let (temp, coordinator, compositor, scene) =
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
                    .spawn_configured_scene_widgets(&config, scene.id)
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
    async fn cutoff_invalidates_a_start_waiting_for_activation() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let widget_id = scene
            .widgets
            .values()
            .next()
            .expect("BUG: test scene must contain a widget")
            .id;
        let key = WidgetInstanceKey::new(widget_id.as_uuid());
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let start = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(&config, scene.id)
                    .await;
            }
        });
        wait_for_widget_calls(&compositor, 2).await;

        let cutoff = coordinator.enqueue_widget_replacement(key).await;
        wait_for_widget_calls(&compositor, 3).await;
        compositor.release_widget_receipts();
        tokio::join!(start, cutoff.wait())
            .0
            .expect("BUG: configured start task");

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Running);
        assert_eq!(coordinator.running_widget_count().await, 0);
        assert_eq!(
            compositor.retained_mode(key),
            Some(WidgetConnectionMode::Inactive)
        );
        assert_widget_call_kinds(
            &compositor,
            &["register_retained", "activate", "deactivate"],
            "a cutoff must invalidate the start prepared before it",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn disabled_configured_widget_is_registered_inactive_without_process() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        scene.enabled = false;
        let widget_key = WidgetInstanceKey::new(
            scene
                .widgets
                .values()
                .next()
                .expect("BUG: test scene must contain a widget")
                .id
                .as_uuid(),
        );
        let config = config_with_scene(&temp, scene).await;

        coordinator.spawn_all_configured_widgets(&config).await;

        assert_widget_call_kinds(
            &compositor,
            &["register_retained"],
            "configuration membership must create an inactive retained record",
        );
        assert_eq!(
            compositor.retained_mode(widget_key),
            Some(WidgetConnectionMode::Inactive)
        );
        assert_eq!(coordinator.running_widget_count().await, 0);
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn reload_refreshes_disabled_inactive_registration() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        scene.enabled = false;
        let widget_id = scene
            .widgets
            .values()
            .next()
            .expect("BUG: test scene must contain a widget")
            .id;
        let config = config_with_scene(&temp, scene).await;
        coordinator.spawn_all_configured_widgets(&config).await;
        config
            .write()
            .await
            .scenes_mut()
            .values_mut()
            .next()
            .expect("BUG: configured scene")
            .widgets
            .get_mut(&widget_id)
            .expect("BUG: configured widget")
            .params
            .insert(
                ParamKey::try_new("current".to_owned()).expect("BUG: valid key"),
                ParamValue::Integer(23),
            );

        coordinator.reload_changed_widgets(&config).await;

        let key = WidgetInstanceKey::new(widget_id.as_uuid());
        assert_eq!(
            compositor
                .retained_params(key)
                .and_then(|params| params.get("current").cloned()),
            Some(serde_json::json!(23))
        );
        assert_eq!(
            compositor.retained_mode(key),
            Some(WidgetConnectionMode::Inactive)
        );
        assert_eq!(coordinator.running_widget_count().await, 0);
        assert_widget_call_kinds(
            &compositor,
            &["register_retained", "register_retained", "credentials"],
            "reload must refresh inactive retained data without activation",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn preview_start_survives_handoff_to_enabled_while_receipts_are_pending() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        scene.enabled = false;
        let scene_id = scene.id;
        let config = config_with_scene(&temp, scene).await;
        let coordinator = Arc::new(coordinator);
        let preview_scene = coordinator.preview_scene_state();
        *preview_scene.lock().await = Some(scene_id);
        compositor.hold_widget_receipts();
        let task = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(&config, scene_id)
                    .await;
            }
        });

        wait_for_widget_calls(&compositor, 2).await;
        config
            .write()
            .await
            .scenes_mut()
            .get_mut(&scene_id)
            .expect("BUG: fixture scene")
            .enabled = true;
        *preview_scene.lock().await = None;
        compositor.release_widget_receipts();
        task.await.expect("BUG: configured spawn task");

        assert!(matches!(
            coordinator.widget_manager.snapshot().await.widgets.as_slice(),
            [managed] if managed.state == ManagedWidgetState::Running
        ));
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn configured_start_revalidates_after_receipts_without_holding_config_lock() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let task = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(&config, scene.id)
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
    async fn registration_enqueue_keeps_the_account_snapshot_ordered() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let secret_store = Arc::clone(&coordinator.secret_store);
        compositor.observe_next_registration(move || {
            assert!(
                secret_store.try_write().is_err(),
                "an account change must not overtake the registration snapshot"
            );
        });

        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

        assert_eq!(coordinator.running_widget_count().await, 1);
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn deletion_invalidates_a_start_waiting_for_registration_receipts() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let widget_id = scene.widgets.values().next().expect("BUG: test widget").id;
        let key = WidgetInstanceKey::new(widget_id.as_uuid());
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let start = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(&config, scene.id)
                    .await;
            }
        });

        wait_for_widget_calls(&compositor, 2).await;
        let delete = {
            let mut config = config.write().await;
            config
                .scenes_mut()
                .get_mut(&scene.id)
                .expect("BUG: fixture scene")
                .widgets
                .shift_remove(&widget_id)
                .expect("BUG: fixture widget");
            coordinator.enqueue_widget_delete(key).await
        };
        assert_widget_call_kinds(
            &compositor,
            &["register_retained", "activate", "unregister_retained"],
            "deletion must follow the pending start preparation in FIFO order",
        );
        compositor.release_widget_receipts();
        tokio::join!(start, delete.wait())
            .0
            .expect("BUG: start task");

        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        assert_eq!(
            compositor.widget_calls().len(),
            3,
            "deleted configuration must not enqueue another lifecycle command"
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn second_occupied_mismatch_stops_after_the_bounded_retry() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let widget = scene.widgets.values().next().expect("BUG: test widget");
        let occupied = WidgetLaunch::new(
            SceneId::from(uuid::Uuid::new_v4()),
            widget.id.as_uuid(),
            widget.widget_type_id,
            "test-display".to_owned(),
        );
        coordinator
            .widget_manager
            .spawn_widget(occupied)
            .await
            .expect("BUG: spawn persistent mismatch");

        let prepared = coordinator
            .prepare_widget_start(&config, scene.id, widget.id, 1)
            .await;
        assert!(matches!(prepared, WidgetStartPreparation::Satisfied));
        assert!(
            compositor.widget_calls().is_empty(),
            "second mismatch must not enqueue another deactivation"
        );
        assert_eq!(
            coordinator.widget_manager.snapshot().await.widgets.len(),
            1,
            "bounded retry must leave the persistent occupant unchanged"
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn wedged_deactivation_receipt_does_not_delay_child_reap() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        let widget = scene.widgets.values().next().expect("BUG: test widget");
        compositor.hold_widget_receipts();
        let cutoff = coordinator
            .enqueue_widget_replacement(WidgetInstanceKey::new(widget.id.as_uuid()))
            .await;
        let wait = tokio::spawn(cutoff.wait());

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !coordinator
            .widget_manager
            .snapshot()
            .await
            .widgets
            .is_empty()
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "targeted child was not reaped while its deactivation receipt remained pending"
            );
            tokio::task::yield_now().await;
        }
        assert!(
            !wait.is_finished(),
            "child reap must not depend on deactivation acknowledgement"
        );
        tokio::time::timeout(
            crate::compositor::WIDGET_COMMAND_ACK_TIMEOUT + Duration::from_secs(1),
            wait,
        )
        .await
        .expect("wedged deactivation must finish at the compositor timeout")
        .expect("BUG: cutoff task");
        compositor.release_widget_receipts();
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn registry_build_swap_while_registration_is_pending_reprepares_before_start() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let spawn = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(&config, scene.id)
                    .await;
            }
        });

        wait_for_widget_calls(&compositor, 2).await;
        std::fs::write(
            temp.path().join("widget-package/manifest.json"),
            r#"{"uid":"550e8400-e29b-41d4-a716-446655440000","version":"2.0.0","name":"coordinator-test","description":"coordinator test","binary":"widget","supported_viewports":[{"type":"rectangular","min_width":1280,"max_width":1280,"min_height":480,"max_height":480}]}"#,
        )
        .expect("BUG: replace manifest build");
        coordinator
            .refresh_widgets()
            .await
            .expect("BUG: refresh replacement build");
        compositor.release_widget_receipts();
        spawn.await.expect("BUG: configured spawn task");

        assert_eq!(
            compositor.widget_calls().len(),
            4,
            "the current build must receive a fresh registration and activation"
        );
        assert!(matches!(
            coordinator.widget_manager.snapshot().await.widgets.as_slice(),
            [managed]
                if managed.state == ManagedWidgetState::Running
                    && managed.identity.version == semver::Version::new(2, 0, 0)
        ));
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn package_reload_replaces_a_pending_restart_with_the_current_build() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/definitely/missing/interpreter\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            let snapshot = coordinator.widget_manager.snapshot().await;
            if matches!(snapshot.widgets.as_slice(), [managed]
                if managed.state == ManagedWidgetState::PendingRestart)
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "failed widget did not enter pending restart"
            );
            tokio::task::yield_now().await;
        }

        std::fs::write(
            temp.path().join("widget-package/manifest.json"),
            r#"{"uid":"550e8400-e29b-41d4-a716-446655440000","version":"2.0.0","name":"coordinator-test","description":"coordinator test","binary":"widget","supported_viewports":[{"type":"rectangular","min_width":1280,"max_width":1280,"min_height":480,"max_height":480}]}"#,
        )
        .expect("BUG: replace manifest build");
        let binary = temp.path().join("widget-package/widget");
        std::fs::write(&binary, "#!/bin/sh\nexec sleep 30\n").expect("BUG: replace widget binary");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: make replacement executable");

        coordinator.reload_changed_widgets(&config).await;

        assert!(matches!(
            coordinator.widget_manager.snapshot().await.widgets.as_slice(),
            [managed]
                if managed.state == ManagedWidgetState::Running
                    && managed.identity.version == semver::Version::new(2, 0, 0)
        ));
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "register_retained",
                "activate",
                "credentials",
            ],
            "package reload must replace retained state before starting the new build",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn only_applied_changed_credentials_retry_an_actual_pending_widget() {
        let (temp, coordinator, _compositor, scene) =
            coordinator_with_widget(Some("#!/definitely/missing/interpreter\n")).await;
        let widget = scene
            .widgets
            .values()
            .next()
            .expect("BUG: test scene must contain a widget");
        let instance_id = widget.id.to_string();
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            let snapshot = coordinator.widget_manager.snapshot().await;
            if matches!(snapshot.widgets.as_slice(), [managed]
                if managed.state == ManagedWidgetState::PendingRestart)
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "failed widget did not enter pending restart"
            );
            tokio::task::yield_now().await;
        }

        assert!(
            coordinator
                .finish_credential_refreshes(vec![PendingCredentialUpdate {
                    instance_id: instance_id.clone(),
                    receipt: CredentialUpdateReceipt::completed(false),
                }])
                .await
                .is_empty()
        );
        let (changed, receipt) = CredentialUpdateReceipt::pending();
        drop(changed);
        assert!(
            coordinator
                .finish_credential_refreshes(vec![PendingCredentialUpdate {
                    instance_id: instance_id.clone(),
                    receipt,
                }])
                .await
                .is_empty(),
            "a dropped credential receipt must not retry the pending widget"
        );
        assert_eq!(
            coordinator
                .finish_credential_refreshes(vec![PendingCredentialUpdate {
                    instance_id: instance_id.clone(),
                    receipt: CredentialUpdateReceipt::completed(true),
                }])
                .await,
            vec![instance_id]
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn parameter_update_during_registration_still_starts_with_current_params() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let spawn = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(&config, scene.id)
                    .await;
            }
        });
        wait_for_widget_calls(&compositor, 2).await;
        let widget = config
            .read()
            .await
            .scenes()
            .get(&scene.id)
            .and_then(|scene| scene.widgets.values().next())
            .expect("BUG: test widget")
            .clone();
        let params = BTreeMap::from([(
            ParamKey::try_new("current".to_owned()).expect("BUG: valid key"),
            ParamValue::Integer(7),
        )]);
        {
            let mut config = config.write().await;
            config
                .scenes_mut()
                .get_mut(&scene.id)
                .expect("BUG: test scene")
                .widgets
                .get_mut(&widget.id)
                .expect("BUG: test widget")
                .params = params.clone();
            coordinator
                .update_widget_params(WidgetInstanceKey::new(widget.id.as_uuid()), &params)
                .expect("parameter update must enqueue");
        }
        compositor.release_widget_receipts();
        spawn.await.expect("BUG: spawn task");
        assert_eq!(coordinator.widget_manager.snapshot().await.widgets.len(), 1);
        assert_eq!(
            compositor
                .retained_params(WidgetInstanceKey::new(widget.id.as_uuid()))
                .and_then(|params| params.get("current").cloned()),
            Some(serde_json::json!(7))
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn parameter_update_before_registration_is_retained_by_start() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let widget = scene.widgets.values_mut().next().expect("BUG: test widget");
        widget.params.insert(
            ParamKey::try_new("current".to_owned()).expect("BUG: valid key"),
            ParamValue::Integer(9),
        );
        let key = WidgetInstanceKey::new(widget.id.as_uuid());
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        assert_eq!(
            compositor
                .retained_params(key)
                .and_then(|params| params.get("current").cloned()),
            Some(serde_json::json!(9))
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn quick_reenable_waits_for_stopping_child_before_starting_successor() {
        let body = slow_termination_widget(None, Duration::from_millis(200));
        let (temp, coordinator, _compositor, scene) = coordinator_with_widget(Some(&body)).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        let stop = {
            let _config = config.read().await;
            coordinator.enqueue_scene_stop(&scene).await
        };
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        stop.wait().await;
        assert!(matches!(
            coordinator.widget_manager.snapshot().await.widgets.as_slice(),
            [managed] if managed.state == ManagedWidgetState::Running
        ));
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn preview_teardown_orders_calls_without_holding_config_lock() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let ready = temp.path().join("preview-ready");
        std::fs::write(
            temp.path().join("widget-package/widget"),
            slow_termination_widget(Some(&ready), Duration::from_millis(200)),
        )
        .expect("BUG: replace preview fixture");
        scene.enabled = false;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        let preview_scene = coordinator.preview_scene_state();
        *preview_scene.lock().await = Some(scene.id);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !ready.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "preview fixture did not install its termination trap"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        compositor.hold_widget_receipts();
        let teardown = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            let preview_scene = Arc::clone(&preview_scene);
            let scene = scene.clone();
            async move {
                let mut preview = preview_scene.lock().await;
                let config_guard = config.read().await;
                let stop = coordinator.enqueue_scene_stop(&scene).await;
                drop(config_guard);
                stop.wait().await;
                let config_guard = config.read().await;
                coordinator.refresh_scene_cycling(config_guard.scenes());
                assert_eq!(preview.take(), Some(scene.id));
            }
        });
        wait_for_widget_calls(&compositor, 3).await;
        let reopen = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            let preview_scene = Arc::clone(&preview_scene);
            async move {
                let mut preview = preview_scene.lock().await;
                let config_guard = config.read().await;
                assert!(preview.replace(scene.id).is_none());
                drop(config_guard);
                drop(preview);
                coordinator
                    .spawn_configured_scene_widgets(&config, scene.id)
                    .await;
            }
        });

        let config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("receipt wait must release the configuration lock");
        drop(config_guard);
        assert!(compositor.widget_calls()[2].starts_with("deactivate "));

        compositor.release_widget_receipts();
        tokio::task::yield_now().await;
        assert!(!teardown.is_finished(), "widget reap must still be pending");
        let config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("widget reap must not retain the configuration lock");
        drop(config_guard);
        teardown.await.expect("BUG: preview teardown task");
        reopen.await.expect("BUG: preview reopen task");
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "register_retained",
                "activate",
            ],
            "preview reopen must follow teardown",
        );
        assert!(matches!(
            coordinator.widget_manager.snapshot().await.widgets.as_slice(),
            [managed] if managed.state == ManagedWidgetState::Running
        ));
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn upgrade_pause_rejects_pending_start_and_deactivates_configured_record() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        compositor.hold_widget_receipts();
        let spawn = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .spawn_configured_scene_widgets(&config, scene.id)
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
    async fn upgrade_pause_blocks_reload_until_authorized_resume() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        coordinator.stop_all_widgets(&config).await;

        std::fs::write(
            temp.path().join("widget-package/manifest.json"),
            r#"{"uid":"550e8400-e29b-41d4-a716-446655440000","version":"2.0.0","name":"coordinator-test","description":"coordinator test","binary":"widget","supported_viewports":[{"type":"rectangular","min_width":1280,"max_width":1280,"min_height":480,"max_height":480}]}"#,
        )
        .expect("BUG: replace widget manifest");
        coordinator.reload_changed_widgets(&config).await;

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Paused);
        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "register_retained",
                "credentials",
            ],
            "registry reload must refresh retained state without resuming a paused widget",
        );

        let resume_baseline = compositor.widget_calls().len();
        compositor.hold_widget_receipts();
        let restart = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                let lifecycle = UpgradeWidgetLifecycle::new(coordinator, config);
                crate::system_upgrade::WidgetLifecycle::restart_widgets(&lifecycle).await;
            }
        });
        wait_for_widget_calls(&compositor, resume_baseline + 1).await;

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Paused);
        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        assert!(
            !restart.is_finished(),
            "authorized resume must wait for retained activation"
        );

        compositor.release_widget_receipts();
        restart.await.expect("BUG: upgrade resume task");

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Running);
        assert_eq!(coordinator.running_widget_count().await, 1);
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "register_retained",
                "credentials",
                "activate",
            ],
            "authorized resume must reuse and reactivate the retained registration",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn upgrade_resume_restores_disabled_active_preview() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        scene.enabled = false;
        let config = config_with_scene(&temp, scene.clone()).await;
        let preview_scene = coordinator.preview_scene_state();
        *preview_scene.lock().await = Some(scene.id);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

        coordinator.stop_all_widgets(&config).await;
        coordinator.resume_all_widgets(&config).await;

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Running);
        assert_eq!(coordinator.running_widget_count().await, 1);
        assert_widget_call_kinds(
            &compositor,
            &["register_retained", "activate", "deactivate", "activate"],
            "upgrade resume must reactivate the retained disabled preview",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn preview_teardown_during_upgrade_resume_prevents_start() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        scene.enabled = false;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        let preview_scene = coordinator.preview_scene_state();
        *preview_scene.lock().await = Some(scene.id);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        coordinator.stop_all_widgets(&config).await;
        let paused_calls = compositor.widget_calls().len();

        compositor.hold_widget_receipts();
        let resume = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move { coordinator.resume_all_widgets(&config).await }
        });
        wait_for_widget_calls(&compositor, paused_calls + 1).await;
        let teardown = {
            let mut preview = preview_scene.lock().await;
            assert_eq!(preview.take(), Some(scene.id));
            let _config = config.read().await;
            coordinator.enqueue_scene_stop(&scene).await
        };
        wait_for_widget_calls(&compositor, paused_calls + 2).await;

        compositor.release_widget_receipts();
        tokio::join!(teardown.wait(), resume)
            .1
            .expect("BUG: resume task");

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Running);
        assert_eq!(coordinator.running_widget_count().await, 0);
        assert_eq!(
            compositor.retained_mode(WidgetInstanceKey::new(scene.widgets[0].id.as_uuid())),
            Some(WidgetConnectionMode::Inactive)
        );
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "activate",
                "deactivate",
            ],
            "preview teardown must be FIFO-final and suppress the resumed child",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn package_reload_replaces_disabled_active_preview() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        scene.enabled = false;
        let config = config_with_scene(&temp, scene.clone()).await;
        let preview_scene = coordinator.preview_scene_state();
        *preview_scene.lock().await = Some(scene.id);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        std::fs::write(
            temp.path().join("widget-package/manifest.json"),
            r#"{"uid":"550e8400-e29b-41d4-a716-446655440000","version":"2.0.0","name":"coordinator-test","description":"coordinator test","binary":"widget","supported_viewports":[{"type":"rectangular","min_width":1280,"max_width":1280,"min_height":480,"max_height":480}]}"#,
        )
        .expect("BUG: replace widget manifest");

        coordinator.reload_changed_widgets(&config).await;

        assert!(matches!(
            coordinator.widget_manager.snapshot().await.widgets.as_slice(),
            [managed]
                if managed.state == ManagedWidgetState::Running
                    && managed.identity.version == semver::Version::new(2, 0, 0)
        ));
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "register_retained",
                "activate",
                "credentials",
            ],
            "reload must replace the shown preview without losing its activation",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn replacement_refreshes_registration_after_preview_teardown() {
        let body = slow_termination_widget(None, Duration::from_millis(100));
        let (temp, coordinator, compositor, mut scene) = coordinator_with_widget(Some(&body)).await;
        scene.enabled = false;
        let widget = scene
            .widgets
            .values()
            .next()
            .expect("BUG: test scene must contain a widget")
            .clone();
        let config = config_with_scene(&temp, scene.clone()).await;
        let preview_scene = coordinator.preview_scene_state();
        *preview_scene.lock().await = Some(scene.id);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        config
            .write()
            .await
            .scenes_mut()
            .get_mut(&scene.id)
            .expect("BUG: configured scene")
            .widgets
            .get_mut(&widget.id)
            .expect("BUG: configured widget")
            .params
            .insert(
                ParamKey::try_new("current".to_owned()).expect("BUG: valid key"),
                ParamValue::Integer(17),
            );

        *preview_scene.lock().await = None;
        let teardown = coordinator.enqueue_scene_stop(&scene).await;
        let replacement = coordinator.replace_configured_widget(&config, scene.id, widget.id);
        tokio::join!(teardown.wait(), replacement);

        assert_eq!(
            compositor
                .retained_params(WidgetInstanceKey::new(widget.id.as_uuid()))
                .and_then(|params| params.get("current").cloned()),
            Some(serde_json::json!(17))
        );
        assert_eq!(
            compositor.retained_mode(WidgetInstanceKey::new(widget.id.as_uuid())),
            Some(WidgetConnectionMode::Inactive)
        );
        assert_eq!(coordinator.running_widget_count().await, 0);
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "deactivate",
                "register_retained",
            ],
            "preview teardown must not suppress the retained replacement update",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn replacement_refreshes_registration_while_paused() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let widget = scene
            .widgets
            .values()
            .next()
            .expect("BUG: test scene must contain a widget")
            .clone();
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        coordinator.stop_all_widgets(&config).await;

        coordinator
            .replace_configured_widget(&config, scene.id, widget.id)
            .await;

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Paused);
        assert_eq!(
            compositor.retained_mode(WidgetInstanceKey::new(widget.id.as_uuid())),
            Some(WidgetConnectionMode::Inactive)
        );
        assert_eq!(coordinator.running_widget_count().await, 0);
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "deactivate",
                "register_retained",
            ],
            "pause must not suppress the retained replacement update",
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn widget_added_while_paused_is_registered_without_activation() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        coordinator.stop_all_widgets(&config).await;
        let baseline = compositor.widget_calls().len();

        let added = scene
            .widgets
            .values()
            .next()
            .expect("BUG: test scene must contain a widget")
            .clone_with_new_id();
        let added_id = added.id;
        config
            .write()
            .await
            .scenes_mut()
            .get_mut(&scene.id)
            .expect("BUG: configured scene")
            .widgets
            .insert(added.id, added);

        coordinator
            .spawn_configured_widget(&config, scene.id, added_id)
            .await;

        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "register_retained",
            ],
            "paused configuration addition must only create an inactive retained record",
        );
        assert_eq!(compositor.widget_calls().len(), baseline + 1);
        assert_eq!(
            compositor.retained_mode(WidgetInstanceKey::new(added_id.as_uuid())),
            Some(WidgetConnectionMode::Inactive)
        );
        assert_eq!(coordinator.running_widget_count().await, 0);
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn cancelled_lifecycle_waiter_preserves_operation_order() {
        let operations = Arc::new(Mutex::new(()));
        let mode = Arc::new(std::sync::Mutex::new(ManagerMode::Running));
        let (first_started_tx, first_started_rx) = tokio::sync::oneshot::channel();
        let (release_first_tx, release_first_rx) = tokio::sync::oneshot::channel();
        let first = tokio::spawn(run_lifecycle_operation(Arc::clone(&operations), {
            let mode = Arc::clone(&mode);
            async move {
                first_started_tx.send(()).expect("BUG: first waiter alive");
                release_first_rx
                    .await
                    .expect("BUG: release first operation");
                *mode.lock().expect("BUG: lifecycle mode lock") = ManagerMode::Paused;
            }
        }));
        first_started_rx
            .await
            .expect("BUG: first lifecycle operation must start");
        first.abort();
        assert!(
            first
                .await
                .expect_err("cancelled lifecycle waiter must stop")
                .is_cancelled()
        );

        let (second_started_tx, mut second_started_rx) = tokio::sync::oneshot::channel();
        let second = tokio::spawn(run_lifecycle_operation(Arc::clone(&operations), {
            let mode = Arc::clone(&mode);
            async move {
                second_started_tx
                    .send(())
                    .expect("BUG: second waiter alive");
                let mut mode = mode.lock().expect("BUG: lifecycle mode lock");
                assert_eq!(*mode, ManagerMode::Paused);
                *mode = ManagerMode::Running;
            }
        }));
        tokio::task::yield_now().await;
        assert!(
            second_started_rx.try_recv().is_err(),
            "second lifecycle operation must wait for the cancelled first waiter’s work"
        );

        release_first_tx
            .send(())
            .expect("BUG: first lifecycle operation alive");
        second.await.expect("BUG: second lifecycle waiter");
        second_started_rx
            .await
            .expect("BUG: second lifecycle operation must start");
        assert_eq!(
            *mode.lock().expect("BUG: lifecycle mode lock"),
            ManagerMode::Running
        );
    }

    #[tokio::test]
    async fn resume_skips_widget_whose_manifest_rejects_its_viewport() {
        let (temp, coordinator, compositor, fullscreen) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let widget_type_id = fullscreen
            .widgets
            .values()
            .next()
            .expect("BUG: fullscreen fixture widget")
            .widget_type_id;
        let mut scene = Scene::combined();
        let widget = Widget::new(
            widget_type_id,
            BTreeMap::new(),
            crate::scene::WidgetPosition { row: 0, col: 0 },
            crate::scene::WidgetPlacement::SlotSpan(crate::scene::SlotSpan {
                columns: 1,
                rows: 1,
            }),
        );
        scene.widgets.insert(widget.id, widget);
        let config = config_with_scene(&temp, scene).await;
        coordinator.stop_all_widgets(&config).await;
        let paused_calls = compositor.widget_calls();

        tokio::time::timeout(
            Duration::from_secs(1),
            coordinator.resume_all_widgets(&config),
        )
        .await
        .expect("unsupported retained widget must not prevent upgrade recovery");

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::Running);
        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        assert_eq!(
            compositor.widget_calls(),
            paused_calls,
            "resume must not enqueue unsupported registration repeatedly"
        );
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn upgrade_pause_reaps_all_children_when_deactivation_receipts_time_out() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let second = scene.widgets[0].clone_with_new_id();
        scene.widgets.insert(second.id, second);
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        assert_eq!(coordinator.running_widget_count().await, 2);

        compositor.hold_widget_receipts();
        coordinator.stop_all_widgets(&config).await;

        let snapshot = coordinator.widget_manager.snapshot().await;
        assert_eq!(snapshot.mode, ManagerMode::Paused);
        assert!(
            snapshot.widgets.is_empty(),
            "every child must be reaped even when each deactivation times out"
        );
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "register_retained",
                "activate",
                "deactivate",
                "deactivate",
            ],
            "upgrade pause must attempt deactivation for every retained child",
        );
        compositor.release_widget_receipts();
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn deactivation_receipt_timeouts_run_concurrently() {
        let (_first, first) = CompositorReceipt::pending("first deactivation");
        let (_second, second) = CompositorReceipt::pending("second deactivation");
        let started = tokio::time::Instant::now();

        wait_for_deactivations(vec![Ok(first), Ok(second)], "test").await;

        assert_eq!(
            started.elapsed(),
            crate::compositor::WIDGET_COMMAND_ACK_TIMEOUT,
            "all deactivation timeouts must elapse concurrently"
        );
    }

    #[tokio::test]
    async fn terminal_shutdown_absorbs_upgrade_resume() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        coordinator.stop_all_widgets(&config).await;
        coordinator.stop_all(&config).await;
        let terminal_calls = compositor.widget_calls();

        let lifecycle = UpgradeWidgetLifecycle::new(Arc::clone(&coordinator), Arc::clone(&config));
        crate::system_upgrade::WidgetLifecycle::restart_widgets(&lifecycle).await;

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::ShuttingDown);
        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        assert_eq!(
            compositor.widget_calls(),
            terminal_calls,
            "an upgrade guard must not activate or start after terminal shutdown"
        );
    }

    #[tokio::test]
    async fn terminal_deactivation_follows_a_resume_waiting_for_receipts() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        coordinator.stop_all_widgets(&config).await;
        let paused_call_count = compositor.widget_calls().len();

        compositor.hold_widget_receipts();
        let resume = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move { coordinator.resume_all_widgets(&config).await }
        });
        wait_for_widget_calls(&compositor, paused_call_count + 1).await;
        let shutdown = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move { coordinator.stop_all(&config).await }
        });
        wait_for_widget_calls(&compositor, paused_call_count + 2).await;

        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::ShuttingDown);
        assert_eq!(compositor.shutdown_call_count(), 0);
        let calls = compositor.widget_calls();
        assert_eq!(
            calls[calls.len() - 2..]
                .iter()
                .map(|call| call.split_whitespace().next().expect("BUG: recorded call"))
                .collect::<Vec<_>>(),
            ["activate", "deactivate"],
            "terminal deactivation must be the final connection-mode command"
        );

        compositor.release_widget_receipts();
        resume.await.expect("BUG: upgrade resume task");
        shutdown.await.expect("BUG: terminal shutdown task");
        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::ShuttingDown);
        assert!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
        );
        assert_eq!(compositor.shutdown_call_count(), 1);
    }

    #[tokio::test]
    async fn terminal_shutdown_waits_for_deactivation_and_child_reap() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let ready = temp.path().join("ready");
        std::fs::write(
            temp.path().join("widget-package/widget"),
            slow_termination_widget(Some(&ready), Duration::from_millis(100)),
        )
        .expect("BUG: write slow widget");
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !ready.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "widget did not install its termination handler"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let call_count = compositor.widget_calls().len();
        compositor.hold_widget_receipts();
        let shutdown = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move { coordinator.stop_all(&config).await }
        });
        wait_for_widget_calls(&compositor, call_count + 1).await;
        assert_eq!(coordinator.widget_manager.mode(), ManagerMode::ShuttingDown);
        assert_eq!(compositor.shutdown_call_count(), 0);

        tokio::time::timeout(Duration::from_secs(1), async {
            while !coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .is_empty()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("terminal shutdown must reap children independently of compositor receipts");
        assert_eq!(compositor.shutdown_call_count(), 0);

        compositor.release_widget_receipts();
        shutdown.await.expect("BUG: terminal shutdown task");
        assert_eq!(compositor.shutdown_call_count(), 1);
    }

    #[tokio::test]
    async fn unsupported_configured_scene_never_registers_or_starts() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        for widget in scene.widgets.values_mut() {
            widget.viewport_shape = ViewportShape::Round;
        }
        let config = config_with_scene(&temp, scene.clone()).await;

        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
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
        let (temp, coordinator, compositor, scene) =
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
            .spawn_configured_scene_widgets(&config, scene.id)
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
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("not an executable\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

        let calls = compositor.widget_calls();
        assert!(
            !calls.iter().any(|call| call.starts_with("unregister ")),
            "retained retry must keep the compositor record: {calls:?}"
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
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn occupied_preflight_does_not_unregister_the_healthy_record() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nwhile :; do sleep 30; done\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;

        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        let calls = compositor.widget_calls();
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

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
        let (temp, coordinator, _compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nwhile :; do sleep 30; done\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

        let stop = {
            let _config = config.read().await;
            coordinator.enqueue_scene_stop(&scene).await
        };
        stop.wait().await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

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

    #[tokio::test]
    async fn occupied_mismatch_retries_with_the_current_launch() {
        let (temp, coordinator, _compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;

        let mut replacement = scene.clone();
        replacement.id = SceneId::generate();
        {
            let mut config = config.write().await;
            config.scenes_mut().clear();
            config
                .scenes_mut()
                .insert(replacement.id, replacement.clone());
        }
        coordinator
            .spawn_configured_scene_widgets(&config, replacement.id)
            .await;

        assert!(matches!(
            coordinator.widget_manager.snapshot().await.widgets.as_slice(),
            [super::super::manager::ManagedWidgetSnapshot { launch, state: ManagedWidgetState::Running, .. }]
                if launch.config_key.scene_id == replacement.id
        ));
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn missing_type_sends_nothing_and_unrelated_reload_recovers_it() {
        let (temp, coordinator, compositor, mut scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let restored_uid = uuid::Uuid::new_v4();
        scene
            .widgets
            .values_mut()
            .next()
            .expect("BUG: widget")
            .widget_type_id = restored_uid;
        let config = config_with_scene(&temp, scene.clone()).await;

        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        assert!(compositor.widget_calls().is_empty());

        std::fs::write(
            temp.path().join("widget-package/manifest.json"),
            format!(
                r#"{{"uid":"{restored_uid}","version":"1.0.0","name":"restored","description":"restored","binary":"widget","supported_viewports":[{{"type":"rectangular","min_width":1280,"max_width":1280,"min_height":480,"max_height":480}}]}}"#
            ),
        )
        .expect("BUG: replace manifest");
        coordinator.reload_changed_widgets(&config).await;

        assert_widget_call_kinds(
            &compositor,
            &["register_retained", "activate", "credentials"],
            "registry reload must refresh credentials after restoring the manifest",
        );
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

    #[tokio::test]
    async fn stopping_snapshot_is_not_started_by_reload() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let ready = temp.path().join("ready");
        std::fs::write(
            temp.path().join("widget-package/widget"),
            slow_termination_widget(Some(&ready), Duration::from_secs(1)),
        )
        .expect("BUG: replace widget executable");
        let config = config_with_scene(&temp, scene.clone()).await;
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !ready.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "widget did not install its termination trap"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let stop = {
            let _config = config.read().await;
            coordinator.enqueue_scene_stop(&scene).await
        };
        assert!(matches!(
            coordinator
                .widget_manager
                .snapshot()
                .await
                .widgets
                .as_slice(),
            [super::super::manager::ManagedWidgetSnapshot {
                state: ManagedWidgetState::Stopping,
                ..
            }]
        ));
        let calls = compositor.widget_calls();

        coordinator.reload_changed_widgets(&config).await;

        let reloaded_calls = compositor.widget_calls();
        assert_eq!(&reloaded_calls[..calls.len()], calls);
        assert_eq!(
            reloaded_calls[calls.len()..],
            [format!("credentials {}", scene.widgets[0].id)],
            "registry reload may refresh retained credentials but must not restart a stopping widget"
        );
        stop.wait().await;
        coordinator.widget_manager.shutdown().await;
    }

    #[tokio::test]
    async fn replacement_waits_without_holding_configuration_lock() {
        let (temp, coordinator, compositor, scene) =
            coordinator_with_widget(Some("#!/bin/sh\nexec sleep 30\n")).await;
        let ready = temp.path().join("replacement-ready");
        std::fs::write(
            temp.path().join("widget-package/widget"),
            slow_termination_widget(Some(&ready), Duration::from_secs(1)),
        )
        .expect("BUG: replace widget executable");
        let config = config_with_scene(&temp, scene.clone()).await;
        let coordinator = Arc::new(coordinator);
        coordinator
            .spawn_configured_scene_widgets(&config, scene.id)
            .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !ready.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "widget did not install its termination trap"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        compositor.hold_widget_receipts();
        let replacement = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let config = Arc::clone(&config);
            async move {
                coordinator
                    .replace_configured_widget(&config, scene.id, scene.widgets[0].id)
                    .await;
            }
        });
        wait_for_widget_calls(&compositor, 3).await;
        assert_widget_call_kinds(
            &compositor,
            &["register_retained", "activate", "deactivate"],
            "replacement must cut off the attached process before changing retained state",
        );

        let config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("replacement receipt wait must release the configuration lock");
        drop(config_guard);
        compositor.release_widget_receipts();
        tokio::task::yield_now().await;
        assert!(
            !replacement.is_finished(),
            "widget reap must still be pending"
        );
        let config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("replacement reap wait must release the configuration lock");
        drop(config_guard);
        replacement.await.expect("BUG: replacement task");
        assert_widget_call_kinds(
            &compositor,
            &[
                "register_retained",
                "activate",
                "deactivate",
                "register_retained",
                "activate",
            ],
            "the canonical start path must restore retained state after cutoff",
        );
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
