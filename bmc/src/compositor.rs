// Copyright (C) 2025  Braiins Systems s.r.o.

//! Compositor trait abstraction for widget rendering.
//!
//! The compositor runs in a separate thread (using calloop) while the main application runs
//! on tokio. Communication happens via channels.

use thiserror::Error;
use tokio::sync::mpsc;

pub use bmc_widget_protocol::{ActionPayload, SettingUpdate};

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

#[derive(Debug, Clone, Default)]
pub struct SceneLayout {
    pub widgets: Vec<WidgetPlacement>,
}

#[derive(Debug, Clone)]
pub struct WidgetAction {
    pub instance_id: InstanceId,
    pub payload: ActionPayload,
}

#[derive(Debug, Clone)]
pub enum CompositorEvent {
    WidgetReady { instance_id: InstanceId },
    WidgetDisconnected { instance_id: InstanceId },
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

    /// Register a widget before spawning its process.
    ///
    /// For widgets using `deck_widget_surface_v1` protocol, `pid` should be `None`
    /// as they identify themselves via the protocol's `instance_id` parameter.
    ///
    /// For third-party clients using `xdg_toplevel`, `pid` must be provided so
    /// the compositor can match the client by PID from Wayland credentials.
    fn register_widget(
        &self,
        instance_id: InstanceId,
        position: Position,
        size: Size,
        pid: Option<u32>,
    ) -> Result<(), CompositorError>;

    /// Unregister a widget when its process stops.
    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError>;

    /// Set the active scene layout (visible widgets and positions).
    fn set_active_scene(&self, layout: SceneLayout) -> Result<(), CompositorError>;

    /// Broadcast a setting update to all connected widgets.
    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError>;

    /// Get a receiver for widget action requests (sound, LED).
    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction>;

    /// Get a receiver for compositor events (widget ready, disconnected).
    fn event_receiver(&self) -> mpsc::UnboundedReceiver<CompositorEvent>;

    /// Shutdown the compositor.
    fn shutdown(&self) -> Result<(), CompositorError>;
}
