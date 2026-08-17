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

//! Compositor trait abstraction for widget rendering.
//!
//! The compositor runs in a separate thread (using calloop) while the main application runs
//! on tokio. Communication happens via channels.

use std::collections::BTreeSet;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{broadcast, mpsc, watch};

pub use crate::data::{SceneCycling, SceneCyclingTransition};
pub use bmc_platform::{DisplayInfo, DisplayShape, HardwareCapabilities, SlotGrid};
pub use bmc_widget_protocol::{
    ActionPayload, CredentialSecrets, LedRequestId, LedRequestStatus, SettingUpdate,
    WidgetInitialConfig,
};

pub type InstanceId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetPlacement {
    pub instance_id: InstanceId,
    pub position: Position,
    pub size: Size,
    pub visible: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneLayout {
    /// Identifies which scene this layout came from; the compositor matches
    /// it against its cycling list to update entries in place instead of
    /// rebuilding. `None` marks the no-scene sentinel — the layout the
    /// tracker falls back to with nothing configured — which
    /// [`SceneLayout::placeholder`] resolves to [`ScenePlaceholder::Logo`].
    pub scene_id: Option<crate::scene::SceneId>,
    /// Per-scene automatic cycling duration override. `None` uses the
    /// global scene-cycling default.
    pub cycle_duration: Option<std::time::Duration>,
    /// Combined scenes are drawn with a separator grid between grid cells;
    /// fullscreen scenes are not. See the renderer's grid drawing.
    pub combined: bool,
    pub widgets: Vec<WidgetPlacement>,
}

/// What a scene shows while none of its widgets has rendered content yet.
/// Derived by [`SceneLayout::placeholder`]; the renderer consults it only
/// for scenes without a committed widget buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenePlaceholder {
    /// Combined scene with no widgets configured: there is nothing to load,
    /// so it shows the separator grid with empty cells instead of the logo.
    Grid,
    /// The no-scene sentinel (`scene_id == None`): the branding logo alone —
    /// a caption would suggest a load that is not happening.
    Logo,
    /// A configured scene whose widgets have not painted their first frame:
    /// the branding logo with the "Loading scene…" caption below it.
    LogoWithCaption,
}

impl SceneLayout {
    /// The placeholder to show while this scene has no rendered widget.
    #[must_use]
    pub fn placeholder(&self) -> ScenePlaceholder {
        if self.combined && self.widgets.is_empty() {
            ScenePlaceholder::Grid
        } else if self.scene_id.is_some() {
            ScenePlaceholder::LogoWithCaption
        } else {
            ScenePlaceholder::Logo
        }
    }
}

#[derive(Debug, Clone)]
pub struct WidgetAction {
    pub instance_id: InstanceId,
    pub payload: ActionPayload,
}

/// LED request status flowing back to the originating widget: bmc
/// emits, the compositor relays as a `led_request_status` event.
#[derive(Debug, Clone)]
pub struct LedRequestStatusEvent {
    pub instance_id: InstanceId,
    pub request_id: LedRequestId,
    pub status: LedRequestStatus,
}

/// Latest active-scene snapshot, delivered over a `watch` channel so a
/// consumer always sees current truth instead of a lossy event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveScene {
    pub scene_id: crate::scene::SceneId,
    pub widget_ids: Vec<InstanceId>,
}

/// Process-local identity for one upgrade display run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UpgradeGeneration(usize);

impl UpgradeGeneration {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

/// Presentation category that selects the firmware or package upgrade surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeKind {
    Firmware,
    Packages,
}

/// Current stage of a system upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradePhase {
    FirmwareDownloading,
    FirmwareVerifying,
    FirmwareApplying,
    PackageRealizing,
    PackageVerifying,
    PackageBuilding,
    PackageActivating,
}

/// Download bytes the on-device display can present without inferring totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// Current upgrade view projected from the internal run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeDisplayState {
    Running {
        kind: UpgradeKind,
        phase: Option<UpgradePhase>,
        progress: Option<DownloadProgress>,
    },
    Succeeded {
        kind: UpgradeKind,
    },
    Failed {
        kind: UpgradeKind,
    },
}

/// Latest coherent display state for one upgrade generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeDisplaySnapshot {
    pub generation: UpgradeGeneration,
    pub state: UpgradeDisplayState,
}

#[derive(Debug, Clone)]
pub enum CompositorEvent {
    WidgetReady { instance_id: InstanceId },
    ScreenActivity,
}

/// Lossless settings commands routed through the compositor-owned mpsc channel
/// instead of the lossy broadcast stream.
#[derive(Debug)]
pub enum SettingsCommand {
    /// Display-brightness change requested by the settings-tray overlay (0-100).
    SetBrightness(u8),
    /// Sound-volume change requested by the settings-tray overlay (0-100).
    SetVolume(u8),
    /// Night-mode toggle requested by the settings-tray overlay.
    ToggleNightMode,
    /// Device restart requested by the settings-tray overlay; bmc may decline.
    Restart,
    /// WiFi-reconfiguration entry requested by the settings-tray overlay.
    ReconfigureWifi,
}

/// Lossless alarm commands routed through the compositor-owned mpsc channel
/// instead of the lossy broadcast stream.
#[derive(Debug)]
pub enum AlarmCommand {
    /// Alarm dismissal requested by the alarm overlay.
    Dismiss,
    /// Alarm snooze requested by the alarm overlay.
    Snooze,
}

#[derive(Debug, Error)]
pub enum CompositorError {
    #[error("compositor not started")]
    NotStarted,
    #[error("compositor already started")]
    AlreadyStarted,
    #[error("widget not found: {0}")]
    WidgetNotFound(InstanceId),
    #[error("widget already registered: {0}")]
    WidgetAlreadyRegistered(InstanceId),
    #[error("failed to send command to compositor: {0}")]
    SendError(String),
    #[error("compositor thread error: {0}")]
    ThreadError(String),
}

/// Trait for compositor implementations.
///
/// The compositor is responsible for:
/// - Managing the Wayland display socket
/// - Rendering widget surfaces to the screen
/// - Handling widget lifecycle (connect, disconnect)
/// - Routing settings updates to widgets
/// - Forwarding widget action requests to the main app
///
/// Implementations run in a separate thread due to calloop's blocking nature.
/// Communication with the main tokio runtime happens via channels.
pub trait Compositor: Send + Sync {
    /// Start the compositor and return the Wayland display socket name.
    fn start(&self) -> Result<String, CompositorError>;

    /// Get the Wayland display socket name. Returns `None` if not started.
    fn wayland_display(&self) -> Option<String>;

    /// Hardware-neutral display and feature capabilities for the active
    /// product, mapped from the hardware profile by the compositor.
    fn hardware_capabilities(&self) -> HardwareCapabilities;

    /// Publish the latest complete upgrade presentation snapshot.
    fn set_upgrade_state(&self, _state: UpgradeDisplaySnapshot) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Register a widget before spawning its process.
    ///
    /// Stores the widget's initial configuration (size, params) in the
    /// compositor so that when the widget connects and requests a
    /// `deck_widget_surface_v1`, the compositor can emit the matching
    /// `configure` + `param_*` events. Must return before the caller
    /// spawns the widget; otherwise a fast-starting widget could
    /// `get_widget_surface` before the compositor knows what to send.
    fn register_widget(
        &self,
        instance_id: InstanceId,
        position: Position,
        size: Size,
        initial_config: WidgetInitialConfig,
    ) -> Result<(), CompositorError>;

    /// Associate the spawned widget's process id with its instance.
    ///
    /// Called after `register_widget` and process spawn. The compositor
    /// uses `SO_PEERCRED` at `get_widget_surface` time to map the Wayland
    /// connection back to the registered instance.
    fn set_widget_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError>;

    /// Unregister a widget when its process stops.
    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError>;

    /// Clear pid association for a specific widget instance. Called when a
    /// widget process exits so that a recycled pid cannot be mistaken for
    /// the dead widget.
    ///
    /// Implementations must only disconnect when the instance currently maps
    /// to `pid`; stale exit notifications for a prior spawn of the same
    /// instance must be ignored.
    ///
    /// The instance stays registered — only the process is detached,
    /// so a respawn re-binds through `set_widget_pid` alone.
    /// `unregister_widget` is what ends an instance.
    fn clear_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError>;

    /// Set the active scene layout (visible widgets and positions).
    fn set_active_scene(&self, layout: SceneLayout) -> Result<(), CompositorError>;

    /// Set all scene layouts for drag-based cycling between scenes.
    fn set_scene_cycling(&self, scenes: Vec<SceneLayout>) -> Result<(), CompositorError>;

    /// Set scene cycling behavior configuration.
    fn set_scene_cycling_config(&self, config: SceneCycling) -> Result<(), CompositorError>;

    /// Gate automatic scene cycling without touching the user's configured
    /// `SceneCycling.enabled`, so resuming cannot override a user who turned
    /// cycling off themselves.
    fn set_scene_cycling_suspended(&self, suspended: bool) -> Result<(), CompositorError>;

    /// Jump the cycler back to the first scene.
    fn reset_to_first_scene(&self) -> Result<(), CompositorError>;

    /// Broadcast a setting update to all connected widgets.
    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError>;

    /// Report the effective display brightness (0-100) to the settings-tray
    /// overlay via the `deck_settings_v1` `brightness` event. Default: no-op
    /// (only the real compositor drives an overlay).
    fn broadcast_brightness(&self, _value: u8) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Report the effective sound volume (0-100) to the settings-tray overlay
    /// via the `deck_settings_v1` `volume` event. Default: no-op.
    fn broadcast_volume(&self, _value: u8) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Report the night-mode state and its "HH:MM" boundary to the
    /// settings-tray overlay via the `deck_settings_v1` `night_mode` event.
    /// `until` is `None` while night mode is disabled. Default: no-op.
    fn broadcast_night_mode(
        &self,
        _active: bool,
        _until: Option<&str>,
    ) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Report a declined restart request to the settings-tray overlay via the
    /// one-shot `deck_settings_v1` `restart_declined` event. Default: no-op.
    fn broadcast_restart_declined(&self, _reason: &str) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Report the WiFi setup-AP SSID to the settings-tray overlay via the
    /// `deck_settings_v1` `wifi_ap` event. `None` means setup mode is inactive.
    /// Default: no-op.
    fn broadcast_wifi_ap(&self, _ssid: Option<String>) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Broadcast that the alarm is ringing to the alarm-overlay via `deck_alarm_v1`
    /// `alarm_ringing` event.
    /// Default: no-op.
    fn broadcast_alarm_ring(
        &self,
        _time: String,
        _period: String,
        _label: String,
        _snooze_allowed: bool,
    ) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Broadcast that the alarm stopped to the alarm-overlay via `deck_alarm_v1`
    /// `alarm_stopped` event.
    /// Default: no-op.
    fn broadcast_alarm_stop(&self) -> Result<(), CompositorError> {
        Ok(())
    }

    /// Push fresh params to a single running widget without stopping
    /// its process. Only valid when geometry (size) is unchanged —
    /// callers route through `unregister_widget` + `register_widget`
    /// for size changes since the widget's EGL surface and Slint scene
    /// are sized at the initial configure.
    ///
    /// Implementations must also refresh the instance's stored initial config.
    /// A crash respawn re-execs the binary without re-reading any configuration,
    /// so whatever is stored here is what the widget comes back with.
    fn update_widget_params(
        &self,
        instance_id: &InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError>;

    /// Push a re-resolved credential set to a single running widget.
    ///
    /// The compositor drops the push when the values match
    /// what it already holds, since callers here react to
    /// a change hint and have no old value to compare against.
    ///
    /// Refreshes the stored initial config for the same reason
    /// [`Compositor::update_widget_params`] does.
    fn update_widget_credentials(
        &self,
        instance_id: &InstanceId,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: bmc_widget_protocol::CredentialSecrets,
    ) -> Result<(), CompositorError>;

    /// Get a receiver for widget action requests (sound, LED).
    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction>;

    /// Get a receiver for one-shot settings commands (brightness, WiFi reconfigure).
    fn settings_receiver(&self) -> mpsc::UnboundedReceiver<SettingsCommand>;

    /// Get a receiver for one-shot alarm commands (dismiss, snooze) sent by the
    /// alarm overlay over `deck_alarm_v1`.
    fn alarm_receiver(&self) -> mpsc::UnboundedReceiver<AlarmCommand>;

    /// Get a sender for LED request status updates flowing back to the
    /// originating widget. The compositor owns the receiver and emits
    /// `led_request_status` events on the matching widget surface.
    fn request_status_sender(&self) -> mpsc::UnboundedSender<LedRequestStatusEvent>;

    /// Subscribe to the compositor event stream (`WidgetReady`,
    /// `ScreenActivity`). Authoritative scene/connection state is on the
    /// `watch` channels below, not here.
    fn subscribe_events(&self) -> broadcast::Receiver<CompositorEvent>;

    /// Latest active scene as a `watch` channel; `None` means no scene is
    /// active. Latest-value delivery — a consumer never misses the current
    /// scene to a lagged stream.
    fn active_scene_watch(&self) -> watch::Receiver<Option<ActiveScene>>;

    /// Set of currently connected widget instance ids as a `watch`
    /// channel. Consumers reconcile against it (e.g. sweeping a
    /// disconnected widget's effects) instead of reacting to one-shot
    /// disconnect events that a lagged stream could drop.
    fn connected_widgets_watch(&self) -> watch::Receiver<BTreeSet<InstanceId>>;

    /// Shutdown the compositor.
    fn shutdown(&self) -> Result<(), CompositorError>;
}

/// Reset the cycler to the first scene each time auto-off blanks the panel, so
/// the frame is already on glass when the backlight returns.
pub(crate) async fn run_screen_blank_reset_task(
    mut screen_blanked_rx: broadcast::Receiver<()>,
    compositor: Arc<dyn Compositor>,
) {
    loop {
        // `Lagged` still means the panel went dark; the reset is idempotent, so
        // fall through instead of skipping it.
        match screen_blanked_rx.recv().await {
            Ok(()) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(skipped = n, "screen_blanked receiver lagged");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
        if let Err(err) = compositor.reset_to_first_scene() {
            tracing::warn!(error = %err, "Failed to reset to the first scene on blank");
        }
    }
}

/// Hold cycling off for as long as night mode lasts, and land on the first
/// scene when it starts.
pub(crate) async fn run_night_mode_cycling_task(
    mut night_mode_active_rx: watch::Receiver<bool>,
    compositor: Arc<dyn Compositor>,
) {
    // The sender may have written before this subscribe, so force the first
    // changed() to deliver the init state.
    night_mode_active_rx.mark_changed();

    // night_mode_active_rx.changed() fires also when the value was unchanged -
    // track the changes locally.
    let mut night_mode_was_active = false;
    loop {
        if night_mode_active_rx.changed().await.is_err() {
            tracing::info!("night mode watch channel closed; startup listener exiting");
            break;
        }

        let night_mode_active = *night_mode_active_rx.borrow();

        if let Err(err) = compositor.set_scene_cycling_suspended(night_mode_active) {
            tracing::warn!(error = %err, "Failed to suspend scene cycling on night mode");
        }

        if night_mode_active
            && !night_mode_was_active
            && let Err(err) = compositor.reset_to_first_scene()
        {
            tracing::warn!(error = %err, "Failed to reset to the first scene on night mode");
        }

        night_mode_was_active = night_mode_active;
    }
}

#[cfg(test)]
mod tests {
    use super::{SceneLayout, ScenePlaceholder, WidgetPlacement};

    fn placement() -> WidgetPlacement {
        WidgetPlacement {
            instance_id: "widget".to_owned(),
            position: super::Position { x: 0, y: 0 },
            size: super::Size {
                width: 317,
                height: 238,
            },
            visible: true,
        }
    }

    #[test]
    fn sentinel_layout_shows_logo_alone() {
        assert_eq!(SceneLayout::default().placeholder(), ScenePlaceholder::Logo);
    }

    #[test]
    fn empty_combined_scene_shows_grid() {
        let layout = SceneLayout {
            scene_id: Some(crate::scene::SceneId::generate()),
            combined: true,
            ..SceneLayout::default()
        };
        assert_eq!(layout.placeholder(), ScenePlaceholder::Grid);
    }

    #[test]
    fn unpainted_combined_scene_with_widgets_shows_loading_caption() {
        let layout = SceneLayout {
            scene_id: Some(crate::scene::SceneId::generate()),
            combined: true,
            widgets: vec![placement()],
            ..SceneLayout::default()
        };
        assert_eq!(layout.placeholder(), ScenePlaceholder::LogoWithCaption);
    }

    #[test]
    fn unpainted_fullscreen_scene_shows_loading_caption() {
        let layout = SceneLayout {
            scene_id: Some(crate::scene::SceneId::generate()),
            widgets: vec![placement()],
            ..SceneLayout::default()
        };
        assert_eq!(layout.placeholder(), ScenePlaceholder::LogoWithCaption);
    }
}
