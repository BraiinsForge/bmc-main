// Copyright (C) 2025  Braiins Systems s.r.o.

//! Compositor trait abstraction for widget rendering.
//!
//! The compositor runs in a separate thread (using calloop) while the main application runs
//! on tokio. Communication happens via channels.

use std::collections::BTreeSet;

use thiserror::Error;
use tokio::sync::{broadcast, mpsc, watch};

pub use crate::data::SceneCycling;
pub use bmc_platform::{DisplayInfo, DisplayShape, HardwareCapabilities, SlotGrid};
pub use bmc_widget_protocol::{
    ActionPayload, LedRequestId, LedRequestStatus, SettingUpdate, WidgetInitialConfig,
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
    /// rebuilding. `None` for tests / default values.
    pub scene_id: Option<crate::scene::SceneId>,
    /// Per-scene automatic cycling duration override. `None` uses the
    /// global scene-cycling default.
    pub cycle_duration: Option<std::time::Duration>,
    pub widgets: Vec<WidgetPlacement>,
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
    fn clear_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError>;

    /// Set the active scene layout (visible widgets and positions).
    fn set_active_scene(&self, layout: SceneLayout) -> Result<(), CompositorError>;

    /// Set all scene layouts for drag-based cycling between scenes.
    fn set_scene_cycling(&self, scenes: Vec<SceneLayout>) -> Result<(), CompositorError>;

    /// Set scene cycling behavior configuration.
    fn set_scene_cycling_config(&self, config: SceneCycling) -> Result<(), CompositorError>;

    /// Reset cycling back to the first scene.
    fn reset_scene_cycle(&self) -> Result<(), CompositorError>;

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

    /// Push fresh params to a single running widget without stopping
    /// its process. Only valid when geometry (size) is unchanged —
    /// callers route through `unregister_widget` + `register_widget`
    /// for size changes since the widget's EGL surface and Slint scene
    /// are sized at the initial configure.
    fn update_widget_params(
        &self,
        instance_id: &InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError>;

    /// Get a receiver for widget action requests (sound, LED).
    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction>;

    /// Get a receiver for one-shot settings commands (brightness, WiFi reconfigure).
    fn settings_receiver(&self) -> mpsc::UnboundedReceiver<SettingsCommand>;

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
