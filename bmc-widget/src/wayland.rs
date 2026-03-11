// Copyright (C) 2025  Braiins Systems s.r.o.

//! Wayland protocol helpers for widgets.
//!
//! This module provides helpers for widgets to communicate with the compositor
//! using the `deck_widget_v1` Wayland protocol extension.
//!
//! # Usage
//!
//! Widgets use a separate Wayland connection for this protocol, alongside
//! their rendering connection which handles `wl_compositor`, `xdg_shell`,
//! `wl_seat`/`wl_touch`, and DMA-BUF buffer management.

use bmc_widget_protocol::{
    ActionPayload, SettingUpdate,
    client::{
        deck_widget_manager_v1::DeckWidgetManagerV1,
        deck_widget_surface_v1::{ActionType, DeckWidgetSurfaceV1},
    },
    wayland_client::{
        Connection, Dispatch, EventQueue, QueueHandle,
        globals::{GlobalListContents, registry_queue_init},
        protocol::{wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_surface::WlSurface},
    },
};
use std::os::fd::AsFd;

/// Errors that can occur during Wayland protocol operations.
#[derive(Debug, thiserror::Error)]
pub enum WaylandError {
    #[error("failed to connect to Wayland display: {0}")]
    Connection(#[from] bmc_widget_protocol::wayland_client::ConnectError),

    #[error("global error: {0}")]
    Global(#[from] bmc_widget_protocol::wayland_client::globals::GlobalError),

    #[error("bind error: {0}")]
    Bind(#[from] bmc_widget_protocol::wayland_client::globals::BindError),

    #[error("deck_widget_manager_v1 global not available")]
    ManagerNotAvailable,

    #[error("protocol dispatch error: {0}")]
    Dispatch(#[from] bmc_widget_protocol::wayland_client::DispatchError),

    #[error("backend error: {0}")]
    Backend(#[from] bmc_widget_protocol::wayland_client::backend::WaylandError),
}

/// Callback trait for handling protocol events.
pub trait WidgetEventHandler {
    /// Called when a setting update is received.
    fn on_setting(&mut self, update: SettingUpdate);

    /// Called when shutdown is requested.
    fn on_shutdown(&mut self);
}

/// Client for the `deck_widget_v1` Wayland protocol.
///
/// This client manages a separate Wayland connection for BMC protocol communication.
/// It can be used alongside Slint or other Wayland clients that manage their own connections.
pub struct WidgetProtocolClient {
    connection: Connection,
    event_queue: EventQueue<WidgetState>,
    state: WidgetState,
}

impl std::fmt::Debug for WidgetProtocolClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetProtocolClient")
            .finish_non_exhaustive()
    }
}

struct WidgetState {
    compositor: Option<WlCompositor>,
    manager: Option<DeckWidgetManagerV1>,
    wl_surface: Option<WlSurface>,
    widget_surface: Option<DeckWidgetSurfaceV1>,
    pending_events: Vec<WidgetEvent>,
}

#[derive(Debug, Clone)]
enum WidgetEvent {
    Setting(SettingUpdate),
    Shutdown,
}

impl WidgetProtocolClient {
    /// Connect to the Wayland display and bind to `deck_widget_manager_v1`.
    pub fn connect() -> Result<Self, WaylandError> {
        let connection = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init::<WidgetState>(&connection)?;

        let mut state = WidgetState {
            compositor: None,
            manager: None,
            wl_surface: None,
            widget_surface: None,
            pending_events: Vec::new(),
        };

        // Bind to wl_compositor and deck_widget_manager_v1
        let qh = event_queue.handle();
        let compositor: WlCompositor = globals.bind(&qh, 1..=1, ())?;
        let manager: DeckWidgetManagerV1 = globals.bind(&qh, 1..=1, ())?;
        state.compositor = Some(compositor);
        state.manager = Some(manager);

        Ok(Self {
            connection,
            event_queue,
            state,
        })
    }

    /// Get the Wayland connection file descriptor for event loop integration.
    ///
    /// Use this to add the connection to your event loop (e.g., with `poll` or `epoll`).
    #[must_use]
    pub fn connection_fd(&self) -> impl AsFd + '_ {
        self.connection.as_fd()
    }

    /// Dispatch pending events (non-blocking).
    ///
    /// Call this when the connection fd is readable.
    pub fn dispatch_pending(&mut self) -> Result<(), WaylandError> {
        self.event_queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }

    /// Read events from the socket and dispatch them (non-blocking).
    ///
    /// This combines prepare_read + read_events + dispatch_pending for use in a polling loop.
    /// Returns Ok(true) if events were dispatched, Ok(false) if nothing was read.
    pub fn poll_events(&mut self) -> Result<bool, WaylandError> {
        // Try to prepare a read guard
        if let Some(guard) = self.event_queue.prepare_read() {
            // Try to read events (non-blocking via WouldBlock handling)
            match guard.read() {
                Ok(_) => {}
                Err(bmc_widget_protocol::wayland_client::backend::WaylandError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    return Ok(false);
                }
                Err(e) => return Err(e.into()),
            }
        }

        // Dispatch any pending events
        self.event_queue.dispatch_pending(&mut self.state)?;
        Ok(true)
    }

    /// Flush outgoing requests to the compositor.
    pub fn flush(&self) -> Result<(), WaylandError> {
        self.connection.flush()?;
        Ok(())
    }

    /// Block and wait for events, dispatching them.
    pub fn blocking_dispatch(&mut self) -> Result<(), WaylandError> {
        self.event_queue.blocking_dispatch(&mut self.state)?;
        Ok(())
    }

    /// Take pending events and process them with the handler.
    pub fn process_events<H: WidgetEventHandler>(&mut self, handler: &mut H) {
        for event in self.state.pending_events.drain(..) {
            match event {
                WidgetEvent::Setting(update) => handler.on_setting(update),
                WidgetEvent::Shutdown => handler.on_shutdown(),
            }
        }
    }

    /// Check if shutdown was requested.
    #[must_use]
    pub fn shutdown_requested(&self) -> bool {
        self.state
            .pending_events
            .iter()
            .any(|e| matches!(e, WidgetEvent::Shutdown))
    }

    /// Request a system action (sound, LED).
    pub fn request_action(&self, action: &ActionPayload) -> Result<(), WaylandError> {
        let Some(ref surface) = self.state.widget_surface else {
            return Ok(());
        };

        let (action_type, payload) = action_to_protocol(action);
        surface.request_action(action_type, payload);
        self.connection.flush()?;
        Ok(())
    }

    /// Get a reference to the widget manager.
    #[must_use]
    pub fn manager(&self) -> Option<&DeckWidgetManagerV1> {
        self.state.manager.as_ref()
    }

    /// Create a widget surface with the given instance_id.
    ///
    /// This creates a new `wl_surface` on this connection and assigns it the
    /// `deck_widget_surface_v1` role. This surface is only used for protocol
    /// events (settings, shutdown) - no rendering happens on it.
    ///
    /// The `instance_id` must match the `DECK_INSTANCE_ID` environment variable.
    /// The compositor matches this connection to the widget's rendering surface by PID.
    pub fn create_widget_surface(&mut self, instance_id: &str) {
        let compositor = self
            .state
            .compositor
            .as_ref()
            .expect("BUG: compositor not bound");
        let manager = self.state.manager.as_ref().expect("BUG: manager not bound");
        let qh = self.event_queue.handle();

        // Create a wl_surface on this connection (not used for rendering)
        let wl_surface = compositor.create_surface(&qh, ());
        self.state.wl_surface = Some(wl_surface.clone());

        // Assign the deck_widget_surface_v1 role
        let widget_surface =
            manager.get_widget_surface(&wl_surface, instance_id.to_owned(), &qh, ());
        self.state.widget_surface = Some(widget_surface);
    }
}

fn action_to_protocol(action: &ActionPayload) -> (ActionType, String) {
    match action {
        ActionPayload::PlaySound { sound } => (
            ActionType::PlaySound,
            serde_json::json!({ "sound": sound }).to_string(),
        ),
        ActionPayload::StopSound {} => (ActionType::StopSound, "{}".to_owned()),
        ActionPayload::Led {
            effect,
            color,
            duration,
        } => (
            ActionType::Led,
            serde_json::json!({
                "effect": effect,
                "color": color,
                "duration": duration
            })
            .to_string(),
        ),
        ActionPayload::StopLed {} => (ActionType::StopLed, "{}".to_owned()),
    }
}

pub(crate) fn setting_from_protocol(setting_type: u32, value: &str) -> Option<SettingUpdate> {
    use bmc_widget_protocol::client::deck_widget_surface_v1::SettingType;

    match SettingType::try_from(setting_type).ok()? {
        SettingType::Timezone => Some(SettingUpdate::Timezone(value.to_owned())),
        SettingType::Localization => {
            let loc = serde_json::from_str(value).ok()?;
            Some(SettingUpdate::Localization(loc))
        }
        SettingType::NightMode => {
            let night_mode = value == "true" || value == "1";
            Some(SettingUpdate::NightMode(night_mode))
        }
        _ => None,
    }
}

// Wayland dispatch implementations

impl Dispatch<WlRegistry, GlobalListContents> for WidgetState {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: <WlRegistry as bmc_widget_protocol::wayland_client::Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Registry events handled by GlobalList
    }
}

impl Dispatch<WlCompositor, ()> for WidgetState {
    fn event(
        _state: &mut Self,
        _proxy: &WlCompositor,
        _event: <WlCompositor as bmc_widget_protocol::wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Compositor has no events
    }
}

impl Dispatch<WlSurface, ()> for WidgetState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: <WlSurface as bmc_widget_protocol::wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // We don't render on this surface, so ignore enter/leave events
    }
}

impl Dispatch<DeckWidgetManagerV1, ()> for WidgetState {
    fn event(
        _state: &mut Self,
        _proxy: &DeckWidgetManagerV1,
        _event: <DeckWidgetManagerV1 as bmc_widget_protocol::wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Manager has no events
    }
}

impl Dispatch<DeckWidgetSurfaceV1, ()> for WidgetState {
    fn event(
        state: &mut Self,
        _proxy: &DeckWidgetSurfaceV1,
        event: <DeckWidgetSurfaceV1 as bmc_widget_protocol::wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use bmc_widget_protocol::client::deck_widget_surface_v1::Event;

        match event {
            Event::Setting {
                setting_type,
                value,
            } => {
                if let Some(update) = setting_from_protocol(setting_type.into(), &value) {
                    state.pending_events.push(WidgetEvent::Setting(update));
                }
            }
            Event::Shutdown => {
                state.pending_events.push(WidgetEvent::Shutdown);
            }
            _ => {}
        }
    }
}
