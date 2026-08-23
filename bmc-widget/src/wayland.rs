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
    ActionPayload, NextAlarm, SettingUpdate, ViewportShape, WidgetInstanceKey,
    client::{
        deck_widget_manager_v2::DeckWidgetManagerV2, deck_widget_surface_v1::DeckWidgetSurfaceV1,
    },
    wayland_client::{
        Connection, Dispatch, EventQueue, QueueHandle,
        globals::{GlobalListContents, registry_queue_init},
        protocol::{wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_surface::WlSurface},
    },
    widget_key_from_env,
};
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

/// How long [`WidgetProtocolClient::wait_for_configure`] blocks before
/// giving up on the compositor's initial configure batch.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Initial configuration accumulated during the compositor's configure
/// batch. Returned from [`WidgetProtocolClient::wait_for_configure`].
///
/// `params` is the raw JSON object the compositor sent — each widget
/// deserializes it into its own `#[derive(Deserialize)]` struct that
/// mirrors its manifest. Even default values are sent over the wire,
/// so the widget does not handle non-optional values by defaults.
#[derive(Debug, Clone)]
pub struct ProtocolInitialState {
    pub width: u32,
    pub height: u32,
    pub viewport_shape: ViewportShape,
    pub display: bmc_widget_protocol::DisplayInfo,
    pub params: serde_json::Map<String, serde_json::Value>,
    pub settings: Vec<SettingUpdate>,
}

/// Errors that can occur during Wayland protocol operations.
#[derive(Debug, thiserror::Error)]
pub enum WaylandError {
    #[error("failed to connect to Wayland display: {0}")]
    Connection(#[from] bmc_widget_protocol::wayland_client::ConnectError),

    #[error("global error: {0}")]
    Global(#[from] bmc_widget_protocol::wayland_client::globals::GlobalError),

    #[error("bind error: {0}")]
    Bind(#[from] bmc_widget_protocol::wayland_client::globals::BindError),

    #[error("deck_widget_manager_v2 global not available")]
    ManagerNotAvailable,

    #[error("invalid widget instance key: {0}")]
    WidgetKey(#[from] bmc_widget_protocol::WidgetKeyEnvError),

    #[error("protocol dispatch error: {0}")]
    Dispatch(#[from] bmc_widget_protocol::wayland_client::DispatchError),

    #[error("backend error: {0}")]
    Backend(#[from] bmc_widget_protocol::wayland_client::backend::WaylandError),
}

/// Callback trait for handling protocol events.
pub trait WidgetEventHandler {
    /// Called when a setting update is received.
    fn on_setting(&mut self, update: SettingUpdate);

    /// Called when a widget-specific param blob is updated at runtime.
    fn on_param_update(&mut self, _params: serde_json::Map<String, serde_json::Value>);

    /// Called when shutdown is requested.
    fn on_shutdown(&mut self);

    /// Called when the compositor publishes a new lifecycle state for
    /// this widget. Default no-op so existing handlers that do not bind
    /// lifecycle compile unchanged.
    fn on_lifecycle(&mut self, _state: bmc_widget_protocol::LifecycleState) {}

    /// Called when automatic scene cycling will transition this widget
    /// on-screen soon. Default no-op so existing handlers compile unchanged.
    fn on_transition_incoming(&mut self) {}
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
    manager: Option<DeckWidgetManagerV2>,
    widget_key: WidgetInstanceKey,
    wl_surface: Option<WlSurface>,
    widget_surface: Option<DeckWidgetSurfaceV1>,
    pending_events: Vec<WidgetEvent>,

    // Initial configure batch accumulation.
    configure_done: bool,
    pending_size: Option<(ViewportShape, u32, u32)>,
    pending_display: Option<bmc_widget_protocol::DisplayInfo>,
    pending_params: serde_json::Map<String, serde_json::Value>,
    pending_initial_settings: Vec<SettingUpdate>,
}

#[derive(Debug, Clone)]
enum WidgetEvent {
    Setting(SettingUpdate),
    ParamUpdate(serde_json::Map<String, serde_json::Value>),
    Shutdown,
    Lifecycle(bmc_widget_protocol::LifecycleState),
    TransitionIncoming,
}

impl WidgetProtocolClient {
    /// Connect to the Wayland display and bind to `deck_widget_manager_v2`.
    pub fn connect() -> Result<Self, WaylandError> {
        let widget_key = widget_key_from_env()?;
        let connection = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init::<WidgetState>(&connection)?;

        let mut state = WidgetState {
            compositor: None,
            manager: None,
            widget_key,
            wl_surface: None,
            widget_surface: None,
            pending_events: Vec::new(),
            configure_done: false,
            pending_size: None,
            pending_display: None,
            pending_params: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
        };

        let qh = event_queue.handle();
        let compositor: WlCompositor = globals.bind(&qh, 1..=1, ())?;
        let manager: DeckWidgetManagerV2 = globals.bind(&qh, 1..=2, ())?;
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
            dispatch_event(handler, event);
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

        match action {
            ActionPayload::PlaySound { sound } => surface.play_sound(sound.clone()),
            ActionPayload::StopSound {} => surface.stop_sound(),
            ActionPayload::LedTemporary {
                request_id,
                effect,
                color,
                period_ms,
                duration_ms,
                scope,
            } => surface.led_temporary(
                *request_id,
                to_protocol::led_effect(*effect),
                u32::from(color.r),
                u32::from(color.g),
                u32::from(color.b),
                *period_ms,
                *duration_ms,
                to_protocol::led_scope(*scope),
            ),
            ActionPayload::LedEndless {
                request_id,
                effect,
                color,
                period_ms,
                scope,
            } => surface.led_endless(
                *request_id,
                to_protocol::led_effect(*effect),
                u32::from(color.r),
                u32::from(color.g),
                u32::from(color.b),
                *period_ms,
                to_protocol::led_scope(*scope),
            ),
            ActionPayload::StopLed { request_id } => surface.stop_led(*request_id),
        }
        self.connection.flush()?;
        Ok(())
    }

    /// Get a reference to the widget manager.
    #[must_use]
    pub fn manager(&self) -> Option<&DeckWidgetManagerV2> {
        self.state.manager.as_ref()
    }

    /// Block until the compositor has finished emitting its initial
    /// configure batch, then return the collected initial state.
    ///
    /// Call this right after [`create_widget_surface`](Self::create_widget_surface)
    /// so the widget can build its renderer against the returned
    /// dimensions and params before entering the main event loop.
    pub fn wait_for_configure(&mut self) -> Result<ProtocolInitialState, WaylandError> {
        use crate::poll::{PollOutcome, poll_dispatch};

        // `blocking_dispatch` has no timeout — push the deadline into
        // `poll(2)` via the shared `poll_dispatch` helper so a silent
        // compositor surfaces as `PollOutcome::Timeout` instead of an
        // indefinite hang.
        let timeout_err = || {
            WaylandError::Backend(
                bmc_widget_protocol::wayland_client::backend::WaylandError::Io(
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timed out waiting for configure_done",
                    ),
                ),
            )
        };
        let dispatch_err = |e: anyhow::Error| {
            WaylandError::Backend(
                bmc_widget_protocol::wayland_client::backend::WaylandError::Io(
                    std::io::Error::other(format!("{e:#}")),
                ),
            )
        };

        let deadline = Instant::now() + CONFIGURE_TIMEOUT;
        while !self.state.configure_done {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(timeout_err());
            }
            let remaining_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            let outcome = poll_dispatch(
                &self.connection,
                &mut self.event_queue,
                &mut self.state,
                remaining_ms,
            )
            .map_err(dispatch_err)?;
            if outcome == PollOutcome::Timeout {
                return Err(timeout_err());
            }
        }
        let (viewport_shape, width, height) = self.state.pending_size.ok_or_else(|| {
            WaylandError::Backend(
                bmc_widget_protocol::wayland_client::backend::WaylandError::Io(
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "compositor sent configure_done without a configure event",
                    ),
                ),
            )
        })?;
        Ok(ProtocolInitialState {
            width,
            height,
            viewport_shape,
            display: resolve_display(self.state.pending_display.take()),
            params: std::mem::take(&mut self.state.pending_params),
            settings: std::mem::take(&mut self.state.pending_initial_settings),
        })
    }

    /// Create a widget surface on this connection.
    ///
    /// This creates a new `wl_surface` on this connection and assigns it
    /// the `deck_widget_surface_v1` role. The surface is only used for
    /// protocol events (configure, params, settings, shutdown); no
    /// rendering happens on it.
    ///
    pub fn create_widget_surface(&mut self) {
        let compositor = self
            .state
            .compositor
            .as_ref()
            .expect("BUG: compositor not bound");
        let manager = self.state.manager.as_ref().expect("BUG: manager not bound");
        let qh = self.event_queue.handle();

        let wl_surface = compositor.create_surface(&qh, ());
        self.state.wl_surface = Some(wl_surface.clone());

        let widget_surface =
            manager.get_widget_surface(self.state.widget_key.to_string(), &wl_surface, &qh, ());
        self.state.widget_surface = Some(widget_surface);
    }
}

pub(crate) mod to_protocol {
    use bmc_widget_protocol::client::deck_widget_surface_v1 as p;
    use bmc_widget_protocol::{LedEffect, LedScope};

    pub fn led_effect(e: LedEffect) -> p::LedEffect {
        match e {
            LedEffect::Chase => p::LedEffect::Chase,
            LedEffect::KnightRider => p::LedEffect::KnightRider,
            LedEffect::Scan => p::LedEffect::Scan,
            LedEffect::Snake => p::LedEffect::Snake,
            LedEffect::Breathe => p::LedEffect::Breathe,
            LedEffect::Solid => p::LedEffect::Solid,
        }
    }

    pub fn led_scope(s: LedScope) -> p::LedScope {
        match s {
            LedScope::Local => p::LedScope::Local,
            LedScope::Global => p::LedScope::Global,
        }
    }
}

fn dispatch_event<H: WidgetEventHandler>(handler: &mut H, event: WidgetEvent) {
    match event {
        WidgetEvent::Setting(update) => handler.on_setting(update),
        WidgetEvent::ParamUpdate(params) => handler.on_param_update(params),
        WidgetEvent::Shutdown => handler.on_shutdown(),
        WidgetEvent::Lifecycle(s) => handler.on_lifecycle(s),
        WidgetEvent::TransitionIncoming => handler.on_transition_incoming(),
    }
}

pub(crate) mod from_protocol {
    use bmc_widget_protocol::client::deck_widget_surface_v1 as p;
    use bmc_widget_protocol::wayland_client::WEnum;
    use bmc_widget_protocol::{
        DateFormat, NumberFormat, TemperatureUnit, TimeSystem, UnitSystem, WeekDay,
    };

    pub fn night_mode(w: WEnum<p::NightModeState>) -> Option<bool> {
        match w.into_result().ok()? {
            p::NightModeState::Off => Some(false),
            p::NightModeState::On => Some(true),
            _ => None,
        }
    }

    pub fn date_format(w: WEnum<p::DateFormat>) -> Option<DateFormat> {
        match w.into_result().ok()? {
            p::DateFormat::DdMmYyyyDot => Some(DateFormat::DdMmYyyyDot),
            p::DateFormat::DdMmYyyySlash => Some(DateFormat::DdMmYyyySlash),
            p::DateFormat::DMYyyySlash => Some(DateFormat::DMYyyySlash),
            p::DateFormat::MDYyyySlash => Some(DateFormat::MDYyyySlash),
            p::DateFormat::DdMmYyyyDash => Some(DateFormat::DdMmYyyyDash),
            p::DateFormat::YyyyMDSlash => Some(DateFormat::YyyyMDSlash),
            p::DateFormat::YyyyMmDdDot => Some(DateFormat::YyyyMmDdDot),
            p::DateFormat::YyyyMmDdDash => Some(DateFormat::YyyyMmDdDash),
            _ => None,
        }
    }

    pub fn time_format(w: WEnum<p::TimeFormat>) -> Option<TimeSystem> {
        match w.into_result().ok()? {
            p::TimeFormat::Hour12 => Some(TimeSystem::Hour12),
            p::TimeFormat::Hour24 => Some(TimeSystem::Hour24),
            _ => None,
        }
    }

    pub fn number_format(w: WEnum<p::NumberFormat>) -> Option<NumberFormat> {
        match w.into_result().ok()? {
            p::NumberFormat::SpaceGroupCommaDecimal => Some(NumberFormat::SpaceGroupCommaDecimal),
            p::NumberFormat::CommaGroupDotDecimal => Some(NumberFormat::CommaGroupDotDecimal),
            p::NumberFormat::DotGroupCommaDecimal => Some(NumberFormat::DotGroupCommaDecimal),
            p::NumberFormat::SpaceGroupDotDecimal => Some(NumberFormat::SpaceGroupDotDecimal),
            _ => None,
        }
    }

    pub fn temperature_unit(w: WEnum<p::TemperatureUnit>) -> Option<TemperatureUnit> {
        match w.into_result().ok()? {
            p::TemperatureUnit::Celsius => Some(TemperatureUnit::Celsius),
            p::TemperatureUnit::Fahrenheit => Some(TemperatureUnit::Fahrenheit),
            _ => None,
        }
    }

    pub fn weekday(w: WEnum<p::Weekday>) -> Option<WeekDay> {
        w.into_result().ok().map(WeekDay::from)
    }

    pub fn unit_system(w: WEnum<p::UnitSystem>) -> Option<UnitSystem> {
        match w.into_result().ok()? {
            p::UnitSystem::Metric => Some(UnitSystem::Metric),
            p::UnitSystem::Imperial => Some(UnitSystem::Imperial),
            _ => None,
        }
    }

    /// `present` discriminator for the `next_alarm` event.
    /// Returns `true` for `present`, `false` for `absent`;
    /// unknown variants resolve to `None` so the caller
    /// can drop the event.
    pub fn presence(w: WEnum<p::Presence>) -> Option<bool> {
        match w.into_result().ok()? {
            p::Presence::Absent => Some(false),
            p::Presence::Present => Some(true),
            _ => None,
        }
    }

    pub fn lifecycle_state(w: WEnum<p::LifecycleState>) -> Option<p::LifecycleState> {
        match w.into_result() {
            Ok(s) => Some(s),
            Err(raw) => {
                tracing::warn!("Unknown deck_widget_v1 lifecycle_state value: {raw}");
                None
            }
        }
    }
}

fn resolve_display(
    pending: Option<bmc_widget_protocol::DisplayInfo>,
) -> bmc_widget_protocol::DisplayInfo {
    pending.unwrap_or(bmc_widget_protocol::DisplayInfo::BMC100)
}

fn apply_configure_event(
    state: &mut WidgetState,
    width: u32,
    height: u32,
    viewport_shape: bmc_widget_protocol::wayland_client::WEnum<
        bmc_widget_protocol::client::deck_widget_surface_v1::ViewportShape,
    >,
) {
    use bmc_widget_protocol::wayland_client::WEnum;
    let Some(shape) = (match viewport_shape {
        WEnum::Value(v) => Some(v.into()),
        WEnum::Unknown(value) => {
            tracing::warn!(
                value,
                "configure event carries unknown viewport_shape; ignoring event"
            );
            None
        }
    }) else {
        return;
    };
    state.pending_size = Some((shape, width, height));
}

fn apply_display_info_event(
    state: &mut WidgetState,
    width: u32,
    height: u32,
    shape: bmc_widget_protocol::wayland_client::WEnum<
        bmc_widget_protocol::client::deck_widget_surface_v1::DisplayShape,
    >,
    dpi: u32,
) {
    use bmc_widget_protocol::wayland_client::WEnum;
    let Some(shape) = (match shape {
        WEnum::Value(v) => Some(v.into()),
        WEnum::Unknown(value) => {
            tracing::warn!(
                value,
                "display_info event carries unknown display_shape;  event"
            );
            None
        }
    }) else {
        return;
    };
    state.pending_display = Some(bmc_widget_protocol::DisplayInfo {
        width,
        height,
        shape,
        dpi,
    });
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

impl Dispatch<DeckWidgetManagerV2, ()> for WidgetState {
    fn event(
        _state: &mut Self,
        _proxy: &DeckWidgetManagerV2,
        _event: <DeckWidgetManagerV2 as bmc_widget_protocol::wayland_client::Proxy>::Event,
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
            Event::Configure {
                width,
                height,
                viewport_shape,
                ..
            } => apply_configure_event(state, width, height, viewport_shape),
            Event::Params { json } => handle_params_json(
                &mut state.pending_params,
                &mut state.pending_events,
                state.configure_done,
                &json,
            ),
            Event::ConfigureDone => {
                state.configure_done = true;
            }
            Event::Timezone { value } => {
                push_setting(state, SettingUpdate::Timezone(value));
            }
            Event::NightMode { value } => {
                if let Some(b) = from_protocol::night_mode(value) {
                    push_setting(state, SettingUpdate::NightMode(b));
                }
            }
            Event::DateFormat { value } => {
                if let Some(v) = from_protocol::date_format(value) {
                    push_setting(state, SettingUpdate::DateFormat(v));
                }
            }
            Event::TimeFormat { value } => {
                if let Some(v) = from_protocol::time_format(value) {
                    push_setting(state, SettingUpdate::TimeFormat(v));
                }
            }
            Event::NumberFormat { value } => {
                if let Some(v) = from_protocol::number_format(value) {
                    push_setting(state, SettingUpdate::NumberFormat(v));
                }
            }
            Event::TemperatureUnit { value } => {
                if let Some(v) = from_protocol::temperature_unit(value) {
                    push_setting(state, SettingUpdate::TemperatureUnit(v));
                }
            }
            Event::FirstDayOfWeek { value } => {
                if let Some(v) = from_protocol::weekday(value) {
                    push_setting(state, SettingUpdate::FirstDayOfWeek(v));
                }
            }
            Event::UnitSystem { value } => {
                if let Some(v) = from_protocol::unit_system(value) {
                    push_setting(state, SettingUpdate::UnitSystem(v));
                }
            }
            Event::NextAlarm {
                present,
                fire_at_utc_ms_hi,
                fire_at_utc_ms_lo,
                name,
            } => {
                if let Some(present) = from_protocol::presence(present) {
                    let next = if present {
                        // i64 reassembly from the wayland-protocol hi/lo split
                        // (presentation-time `tv_sec_hi`/`tv_sec_lo` pattern).
                        let fire_at_utc_ms =
                            (i64::from(fire_at_utc_ms_hi) << 32) | i64::from(fire_at_utc_ms_lo);
                        Some(NextAlarm {
                            fire_at_utc_ms,
                            name,
                        })
                    } else {
                        None
                    };
                    push_setting(state, SettingUpdate::NextAlarm(next));
                }
            }
            Event::Shutdown => {
                state.pending_events.push(WidgetEvent::Shutdown);
            }
            Event::Lifecycle { state: value } => {
                if let Some(s) = from_protocol::lifecycle_state(value) {
                    state.pending_events.push(WidgetEvent::Lifecycle(s));
                }
            }
            Event::TransitionIncoming => {
                state.pending_events.push(WidgetEvent::TransitionIncoming);
            }
            Event::DisplayInfo {
                width,
                height,
                shape,
                dpi,
            } => apply_display_info_event(state, width, height, shape, dpi),
            Event::LedRequestStatus { request_id, status } => {
                tracing::debug!("Received led_request_status: req={request_id} status={status:?}");
            }
            // This client exposes no credential API to its widget,
            // so it has nothing to hold a resolution or spend a secret on.
            Event::Credentials { .. } | Event::CredentialSecrets { .. } => {
                tracing::trace!("Ignoring credential event on a client without credential support");
            }
            _ => {}
        }
    }
}

/// Route a setting event into the initial configure batch if the batch
/// is still open, or into the runtime event queue otherwise.
fn push_setting(state: &mut WidgetState, update: SettingUpdate) {
    if state.configure_done {
        state.pending_events.push(WidgetEvent::Setting(update));
    } else {
        state.pending_initial_settings.push(update);
    }
}

fn push_params(
    pending_params: &mut serde_json::Map<String, serde_json::Value>,
    pending_events: &mut Vec<WidgetEvent>,
    configure_done: bool,
    params: serde_json::Map<String, serde_json::Value>,
) {
    if configure_done {
        pending_events.push(WidgetEvent::ParamUpdate(params));
    } else {
        *pending_params = params;
    }
}

fn handle_params_json(
    pending_params: &mut serde_json::Map<String, serde_json::Value>,
    pending_events: &mut Vec<WidgetEvent>,
    configure_done: bool,
    json: &str,
) {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(map)) => {
            push_params(pending_params, pending_events, configure_done, map);
        }
        Ok(other) => {
            tracing::warn!("Params JSON is not an object, ignoring: {other}");
        }
        Err(e) => {
            tracing::warn!("Failed to decode params JSON ({}): {}", json, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_widget_protocol::LifecycleState;

    #[derive(Default)]
    struct MockHandler {
        settings: Vec<SettingUpdate>,
        param_updates: usize,
        shutdowns: usize,
        lifecycle: Vec<LifecycleState>,
    }

    impl WidgetEventHandler for MockHandler {
        fn on_setting(&mut self, update: SettingUpdate) {
            self.settings.push(update);
        }
        fn on_param_update(&mut self, _: serde_json::Map<String, serde_json::Value>) {
            self.param_updates += 1;
        }
        fn on_shutdown(&mut self) {
            self.shutdowns += 1;
        }
        fn on_lifecycle(&mut self, state: bmc_widget_protocol::LifecycleState) {
            self.lifecycle.push(state);
        }
    }

    #[test]
    fn dispatch_event_routes_lifecycle_to_on_lifecycle() {
        let mut handler = MockHandler::default();
        dispatch_event(
            &mut handler,
            WidgetEvent::Lifecycle(LifecycleState::Visible),
        );

        assert_eq!(handler.lifecycle.len(), 1);
        assert!(matches!(handler.lifecycle[0], LifecycleState::Visible));
        assert_eq!(handler.shutdowns, 0);
        assert_eq!(handler.param_updates, 0);
        assert!(handler.settings.is_empty());
    }

    #[test]
    fn dispatch_event_routes_shutdown_to_on_shutdown() {
        let mut handler = MockHandler::default();
        dispatch_event(&mut handler, WidgetEvent::Shutdown);

        assert_eq!(handler.shutdowns, 1);
        assert!(handler.lifecycle.is_empty());
    }

    fn test_widget_state() -> WidgetState {
        WidgetState {
            compositor: None,
            manager: None,
            widget_key: "550e8400-e29b-41d4-a716-446655440000"
                .parse()
                .expect("BUG: test widget key must be canonical"),
            wl_surface: None,
            widget_surface: None,
            pending_events: Vec::new(),
            configure_done: false,
            pending_size: None,
            pending_display: None,
            pending_params: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
        }
    }

    #[test]
    fn resolve_display_defaults_to_bmc100() {
        assert_eq!(
            resolve_display(None),
            bmc_widget_protocol::DisplayInfo::BMC100
        );
    }

    #[test]
    fn resolve_display_keeps_compositor_value() {
        let info = bmc_widget_protocol::DisplayInfo {
            width: 320,
            height: 480,
            shape: bmc_widget_protocol::DisplayShape::Rectangular,
            dpi: 1,
        };
        assert_eq!(resolve_display(Some(info)), info);
    }

    #[test]
    fn display_info_event_updates_pending_display() {
        use bmc_widget_protocol::wayland_client::WEnum;
        let mut state = test_widget_state();
        apply_display_info_event(
            &mut state,
            480,
            480,
            WEnum::Value(bmc_widget_protocol::client::deck_widget_surface_v1::DisplayShape::Round),
            1,
        );
        assert_eq!(
            state.pending_display,
            Some(bmc_widget_protocol::DisplayInfo {
                width: 480,
                height: 480,
                shape: bmc_widget_protocol::DisplayShape::Round,
                dpi: 1,
            }),
        );
    }

    #[test]
    fn configure_event_populates_viewport_shape() {
        use bmc_widget_protocol::client::deck_widget_surface_v1::ViewportShape as P;
        use bmc_widget_protocol::wayland_client::WEnum;
        let mut state = test_widget_state();
        apply_configure_event(&mut state, 317, 238, WEnum::Value(P::Round));
        let (viewport_shape, width, height) = state
            .pending_size
            .expect("BUG: pending_size must be set after configure");
        assert_eq!(viewport_shape, ViewportShape::Round);
        assert_eq!(width, 317);
        assert_eq!(height, 238);
    }
}
