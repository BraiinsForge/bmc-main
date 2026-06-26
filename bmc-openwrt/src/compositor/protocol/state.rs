// Copyright (C) 2025  Braiins Systems s.r.o.

//! Protocol state management for deck_widget_v1.

use std::sync::{Arc, Mutex};

use bmc::compositor::InstanceId;
use bmc_widget_protocol::server::deck_widget_surface_v1::DeckWidgetSurfaceV1;
use bmc_widget_protocol::{
    ActionPayload, LedRequestId, LedRequestStatus, SettingUpdate, WidgetInitialConfig,
};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::collections::HashMap;

use super::conversions::{
    date_format_to_protocol, night_mode_to_protocol, number_format_to_protocol,
    presence_to_protocol, temperature_unit_to_protocol, time_format_to_protocol,
    unit_system_to_protocol, weekday_to_protocol,
};
use crate::compositor::widget_tracker::LifecycleState;

#[derive(Debug, Clone)]
pub struct WidgetData {
    pub instance_id: InstanceId,
    pub config: WidgetInitialConfig,
    pub protocol_surface: Option<DeckWidgetSurfaceV1>,
    pub wl_surface: Option<WlSurface>,
    /// PID of the widget process. Used to (1) match a Wayland connection
    /// back to the registered instance via `SO_PEERCRED` at
    /// `get_widget_surface` time, and (2) match Slint render surfaces from
    /// the rendering connection sharing the same process.
    pub pid: Option<u32>,
}

/// A widget connection that arrived before `set_widget_pid` registered
/// the process. Buffered until the pid is known.
///
/// `pid` is always populated with a real kernel pid — we refuse to
/// buffer connections whose peer credentials the compositor could
/// not read, so the unknown-pid fan-in that would otherwise uncap
/// this buffer is closed at the dispatch layer.
#[derive(Debug)]
struct PendingConnection {
    pid: u32,
    wl_surface: WlSurface,
    protocol_surface: DeckWidgetSurfaceV1,
    instance_id_lock: Arc<Mutex<InstanceId>>,
}

/// Cap on `pending_connections` to stop a rogue or crash-looping
/// same-UID client (the only attackers in the Wayland threat model)
/// from exhausting compositor memory by spamming
/// `get_widget_surface` with pids the coordinator has not
/// registered. Oldest buffered entries are dropped first; the
/// eviction is cheap because the queue is small.
const MAX_PENDING_CONNECTIONS: usize = 512;

#[derive(Debug)]
pub struct DeckWidgetProtocolState {
    widgets: HashMap<InstanceId, WidgetData>,
    /// Latest observed value of each runtime setting. Emitted to newly
    /// connecting widgets as part of the initial batch so they start with
    /// a fully populated state instead of waiting for the next change.
    current_settings: Vec<SettingUpdate>,
    pending_actions: Vec<(InstanceId, ActionPayload)>,
    newly_connected: Vec<InstanceId>,
    newly_disconnected: Vec<InstanceId>,
    /// Connections that arrived before the coordinator called
    /// `set_widget_pid`. Resolved in `set_widget_pid`.
    pending_connections: Vec<PendingConnection>,
}

/// Abstraction over the Wayland server surface used by `emit_initial_state_into`.
/// The real implementation delegates to `DeckWidgetSurfaceV1`; the `#[cfg(test)]`
/// implementation records events in a `Vec` for order/payload assertions.
trait WidgetSurface {
    fn configure(
        &self,
        width: u32,
        height: u32,
        viewport_shape: bmc_widget_protocol::server::deck_widget_surface_v1::ViewportShape,
    );
    fn display_info(
        &self,
        width: u32,
        height: u32,
        shape: bmc_widget_protocol::server::deck_widget_surface_v1::DisplayShape,
        dpi: u32,
    );
    fn params(&self, params_json: String);
    fn widget_identity(&self, json: String);
    fn emit_setting(&self, setting: &SettingUpdate);
    fn configure_done(&self);
}

impl WidgetSurface for DeckWidgetSurfaceV1 {
    fn configure(
        &self,
        width: u32,
        height: u32,
        viewport_shape: bmc_widget_protocol::server::deck_widget_surface_v1::ViewportShape,
    ) {
        self.configure(width, height, viewport_shape);
    }

    fn display_info(
        &self,
        width: u32,
        height: u32,
        shape: bmc_widget_protocol::server::deck_widget_surface_v1::DisplayShape,
        dpi: u32,
    ) {
        self.display_info(width, height, shape, dpi);
    }

    fn params(&self, params_json: String) {
        self.params(params_json);
    }

    fn widget_identity(&self, json: String) {
        self.widget_identity(json);
    }

    fn emit_setting(&self, setting: &SettingUpdate) {
        emit_setting(self, setting);
    }

    fn configure_done(&self) {
        self.configure_done();
    }
}

impl DeckWidgetProtocolState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
            current_settings: Vec::new(),
            pending_actions: Vec::new(),
            newly_connected: Vec::new(),
            newly_disconnected: Vec::new(),
            pending_connections: Vec::new(),
        }
    }

    pub fn register_widget(&mut self, instance_id: InstanceId, config: WidgetInitialConfig) {
        tracing::info!(
            "Registering widget {}: {}x{} viewport_shape={:?}",
            instance_id,
            config.width,
            config.height,
            config.viewport_shape
        );
        self.widgets.insert(
            instance_id.clone(),
            WidgetData {
                instance_id,
                config,
                protocol_surface: None,
                wl_surface: None,
                pid: None,
            },
        );
    }

    /// Associate a spawned process pid with an instance so that
    /// `get_widget_surface` can resolve the connection's identity via
    /// peer credentials.
    ///
    /// Also resolves any connection that arrived before this call (the
    /// race between process spawn and pid registration).
    pub fn set_widget_pid(&mut self, instance_id: &InstanceId, pid: u32) {
        let Some(widget) = self.widgets.get_mut(instance_id) else {
            tracing::error!(
                "set_widget_pid for {instance_id}: no widget record; register_widget was not called first"
            );
            debug_assert!(
                false,
                "set_widget_pid called before register_widget for {instance_id}"
            );
            return;
        };
        widget.pid = Some(pid);

        // Check if this widget already connected before its pid was registered.
        let pending = self
            .pending_connections
            .iter()
            .position(|pc| pc.pid == pid)
            .map(|idx| self.pending_connections.swap_remove(idx));

        if let Some(pending) = pending {
            tracing::info!(
                "Resolving buffered connection for pid={} → instance={}",
                pid,
                instance_id
            );
            pending
                .instance_id_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone_from(instance_id);
            self.attach_surface(instance_id, pending.wl_surface, pending.protocol_surface);

            let surface = self.widgets[instance_id]
                .protocol_surface
                .as_ref()
                .expect("BUG: attach_surface should have set protocol_surface")
                .clone();
            self.emit_initial_state(instance_id, &surface);
        }
    }

    /// Buffer a widget connection whose pid hasn't been registered yet.
    ///
    /// Callers must supply a valid (non-sentinel) kernel pid; dispatch
    /// refuses to buffer unknown-credential connections to keep the
    /// buffer bounded even under hostile input. If the queue is at
    /// [`MAX_PENDING_CONNECTIONS`] capacity, the oldest entry is
    /// dropped (its Wayland resources go out of scope, so the client
    /// sees its surface vanish).
    pub fn buffer_pending_connection(
        &mut self,
        pid: u32,
        wl_surface: WlSurface,
        protocol_surface: DeckWidgetSurfaceV1,
        instance_id_lock: Arc<Mutex<InstanceId>>,
    ) {
        if self.pending_connections.len() >= MAX_PENDING_CONNECTIONS {
            let dropped = self.pending_connections.remove(0);
            tracing::warn!(
                "pending_connections at capacity ({}); evicting oldest (dropped pid={})",
                MAX_PENDING_CONNECTIONS,
                dropped.pid,
            );
        }
        self.pending_connections.push(PendingConnection {
            pid,
            wl_surface,
            protocol_surface,
            instance_id_lock,
        });
    }

    /// Attach the wl_surface and protocol surface produced by
    /// `get_widget_surface` to an existing (or freshly promoted) widget
    /// record.
    pub fn attach_surface(
        &mut self,
        instance_id: &InstanceId,
        wl_surface: WlSurface,
        protocol_surface: DeckWidgetSurfaceV1,
    ) {
        let Some(entry) = self.widgets.get_mut(instance_id) else {
            tracing::error!(
                "attach_surface for {instance_id}: no widget record; dispatch resolved a pid that has no registered widget"
            );
            debug_assert!(
                false,
                "attach_surface called without a registered widget for {instance_id}"
            );
            return;
        };
        entry.wl_surface = Some(wl_surface);
        entry.protocol_surface = Some(protocol_surface);
        self.newly_connected.push(instance_id.clone());
    }

    /// Find instance_id for a surface, matching by PID. Accepts
    /// `Option<u32>` so callers that derive the pid from Wayland
    /// peer credentials don't have to unwrap first; `None` shortcuts
    /// to no match.
    pub fn instance_id_for_surface_by_pid(&self, pid: Option<u32>) -> Option<&InstanceId> {
        let pid = pid?;
        self.widgets
            .values()
            .find(|w| w.pid == Some(pid))
            .map(|w| &w.instance_id)
    }

    pub fn instance_id_for_surface(&self, surface: &WlSurface) -> Option<&InstanceId> {
        self.widgets
            .values()
            .find(|w| w.wl_surface.as_ref().is_some_and(|s| s == surface))
            .map(|w| &w.instance_id)
    }

    /// Remove the widget record and return its pid so the caller can
    /// run pid-tagged cleanup synchronously. Pushes the instance id
    /// onto `newly_disconnected` for `WidgetDisconnected` event
    /// emission.
    pub fn unregister_widget(&mut self, instance_id: &InstanceId) -> Option<u32> {
        let widget = self.widgets.remove(instance_id)?;

        // Purge any buffered connection for the disconnecting pid
        // before its widget record is gone; the natural-exit window
        // for already-registered widgets is handled by `clear_pid`.
        if let Some(pid) = widget.pid {
            let before = self.pending_connections.len();
            self.pending_connections.retain(|pc| pc.pid != pid);
            let purged = before - self.pending_connections.len();
            if purged > 0 {
                tracing::info!(
                    "unregister_widget({instance_id}): purged {purged} pending connection(s) with pid={pid}"
                );
            }
        }

        let pid = widget.pid;
        self.newly_disconnected.push(widget.instance_id);
        pid
    }

    /// Synthesize a disconnect for an exited widget process.
    ///
    /// A crashed or SIGTERM'd widget can exit without sending protocol
    /// `Destroy`, so the coordinator emits this call from its child-exit
    /// watcher. To avoid PID-reuse races, disconnection is guarded by both
    /// instance id and expected pid: stale exit notifications for a prior
    /// spawn of the same instance are ignored.
    pub fn clear_pid_for_instance(
        &mut self,
        instance_id: &InstanceId,
        expected_pid: u32,
    ) -> Option<u32> {
        let Some(current_pid) = self.widgets.get(instance_id).and_then(|w| w.pid) else {
            tracing::debug!(
                "clear_pid_for_instance: ignoring stale clear for unknown instance {instance_id} (expected_pid={expected_pid})"
            );
            return None;
        };

        if current_pid != expected_pid {
            tracing::debug!(
                "clear_pid_for_instance: ignoring stale clear for instance {instance_id}: expected pid {}, current pid {}",
                expected_pid,
                current_pid
            );
            return None;
        }

        self.unregister_widget(instance_id)
    }

    pub fn add_action(&mut self, instance_id: InstanceId, payload: ActionPayload) {
        self.pending_actions.push((instance_id, payload));
    }

    pub fn drain_actions(&mut self) -> Vec<(InstanceId, ActionPayload)> {
        std::mem::take(&mut self.pending_actions)
    }

    pub fn drain_connected(&mut self) -> Vec<InstanceId> {
        std::mem::take(&mut self.newly_connected)
    }

    pub fn drain_disconnected(&mut self) -> Vec<InstanceId> {
        std::mem::take(&mut self.newly_disconnected)
    }

    /// Store and broadcast a setting update.
    pub fn broadcast_setting(&mut self, setting: &SettingUpdate) {
        self.current_settings
            .retain(|s| std::mem::discriminant(s) != std::mem::discriminant(setting));
        self.current_settings.push(setting.clone());

        for widget_data in self.widgets.values() {
            if let Some(ref surface) = widget_data.protocol_surface {
                emit_setting(surface, setting);
            }
        }
    }

    /// Push fresh params to a running widget by re-emitting the
    /// `params` event on its surface. Also refreshes the stored
    /// initial config so a reconnect (e.g. crash + respawn) sees the
    /// current values.
    pub fn update_widget_params(
        &mut self,
        instance_id: &InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    ) {
        let Some(widget_data) = self.widgets.get_mut(instance_id) else {
            tracing::warn!("update_widget_params: no widget record for {instance_id}");
            return;
        };
        widget_data.config.params = params;

        let Some(surface) = widget_data.protocol_surface.as_ref() else {
            tracing::warn!("update_widget_params: widget {instance_id} has no surface yet");
            return;
        };

        let params_json = serde_json::Value::Object(widget_data.config.params.clone()).to_string();
        surface.params(params_json);
    }

    /// Emit the initial configure batch on the given surface for the
    /// given instance: `configure` → `display_info` → `params` → setting events →
    /// `configure_done`. Called by the dispatch handler right after the
    /// surface role is assigned.
    pub fn emit_initial_state(&self, instance_id: &InstanceId, surface: &DeckWidgetSurfaceV1) {
        self.emit_initial_state_into(instance_id, surface);
    }

    fn emit_initial_state_into<S: WidgetSurface>(&self, instance_id: &InstanceId, surface: &S) {
        let Some(widget) = self.widgets.get(instance_id) else {
            tracing::error!(
                "emit_initial_state for {instance_id}: no widget record; dispatch resolved a pid that has no registered widget"
            );
            surface.configure_done();
            return;
        };
        let config = &widget.config;

        surface.configure(config.width, config.height, config.viewport_shape.into());

        surface.display_info(
            config.display.width,
            config.display.height,
            config.display.shape.into(),
            config.display.dpi,
        );

        if let Some(identity) = &config.identity {
            surface.widget_identity(identity.to_wire());
        }

        let params_json = serde_json::Value::Object(config.params.clone()).to_string();
        surface.params(params_json);

        for setting in &self.current_settings {
            surface.emit_setting(setting);
        }

        surface.configure_done();
    }

    #[cfg(test)]
    pub(super) fn widget_config(&self, instance_id: &str) -> Option<&WidgetInitialConfig> {
        self.widgets.get(instance_id).map(|w| &w.config)
    }

    #[cfg(test)]
    pub(super) fn test_emit_initial_state_events(
        &self,
        instance_id: &str,
    ) -> Option<RecordedEvents> {
        if !self.widgets.contains_key(instance_id) {
            return None;
        }
        let sink = RecordingSurface::default();
        self.emit_initial_state_into(&instance_id.to_owned(), &sink);
        Some(sink.into_events())
    }

    /// Emit `led_request_status` on the widget's surface. Drops
    /// silently if the widget has no surface (still configuring or
    /// already gone).
    pub fn emit_led_request_status(
        &self,
        instance_id: &InstanceId,
        request_id: LedRequestId,
        status: LedRequestStatus,
    ) {
        let Some(widget) = self.widgets.get(instance_id) else {
            return;
        };
        let Some(ref surface) = widget.protocol_surface else {
            return;
        };
        surface.led_request_status(request_id, led_request_status_to_protocol(status));
    }

    pub fn broadcast_shutdown(&self) {
        for widget_data in self.widgets.values() {
            if let Some(ref surface) = widget_data.protocol_surface {
                surface.shutdown();
            }
        }
    }

    /// Emit a `lifecycle` event on the widget's surface.
    ///
    /// Returns the [`ClientId`] of the receiving widget so callers can
    /// scope the subsequent display flush to just the affected clients.
    /// `None` when the widget has not registered yet or its Wayland
    /// surface has not been attached — the compositor calls this
    /// eagerly from scene-change paths, so it must tolerate missing
    /// entries.
    pub fn send_lifecycle(
        &self,
        instance_id: &InstanceId,
        state: LifecycleState,
    ) -> Option<ClientId> {
        let Some(widget) = self.widgets.get(instance_id) else {
            tracing::trace!("send_lifecycle: no widget record for {instance_id} (state={state:?})");
            return None;
        };
        let Some(surface) = widget.protocol_surface.as_ref() else {
            tracing::debug!("send_lifecycle: {instance_id} has no surface yet (state={state:?})");
            return None;
        };
        surface.lifecycle(state);
        surface.client().map(|c| c.id())
    }

    /// Emit a `transition_incoming` event on the widget's surface.
    ///
    /// Returns the [`ClientId`] of the receiving widget so callers can
    /// flush only affected clients. `None` when the widget has not
    /// registered yet or its Wayland surface has not been attached.
    pub fn send_transition_incoming(&self, instance_id: &InstanceId) -> Option<ClientId> {
        let Some(widget) = self.widgets.get(instance_id) else {
            tracing::trace!("send_transition_incoming: no widget record for {instance_id}");
            return None;
        };
        let Some(surface) = widget.protocol_surface.as_ref() else {
            tracing::debug!("send_transition_incoming: {instance_id} has no surface yet");
            return None;
        };
        surface.transition_incoming();
        surface.client().map(|c| c.id())
    }
}

impl Default for DeckWidgetProtocolState {
    fn default() -> Self {
        Self::new()
    }
}

fn led_request_status_to_protocol(
    status: LedRequestStatus,
) -> bmc_widget_protocol::server::deck_widget_surface_v1::LedRequestStatus {
    use bmc_widget_protocol::server::deck_widget_surface_v1::LedRequestStatus as P;
    match status {
        LedRequestStatus::Accepted => P::Accepted,
        LedRequestStatus::Rejected => P::Rejected,
        LedRequestStatus::Superseded => P::Superseded,
        LedRequestStatus::Expired => P::Expired,
    }
}

fn emit_setting(surface: &DeckWidgetSurfaceV1, setting: &SettingUpdate) {
    match setting {
        SettingUpdate::Timezone(tz) => surface.timezone(tz.clone()),
        SettingUpdate::NightMode(enabled) => surface.night_mode(night_mode_to_protocol(*enabled)),
        SettingUpdate::DateFormat(d) => surface.date_format(date_format_to_protocol(*d)),
        SettingUpdate::TimeFormat(t) => surface.time_format(time_format_to_protocol(*t)),
        SettingUpdate::NumberFormat(n) => surface.number_format(number_format_to_protocol(*n)),
        SettingUpdate::TemperatureUnit(u) => {
            surface.temperature_unit(temperature_unit_to_protocol(*u));
        }
        SettingUpdate::FirstDayOfWeek(w) => surface.first_day_of_week(weekday_to_protocol(*w)),
        SettingUpdate::UnitSystem(u) => surface.unit_system(unit_system_to_protocol(*u)),
        SettingUpdate::NextAlarm(next) => {
            // wayland has no native 64-bit integer; split fire_at_utc_ms
            // into high/low halves the same way wp_presentation_feedback
            // splits tv_sec into tv_sec_hi/tv_sec_lo.
            let (present, fire_at_utc_ms_hi, fire_at_utc_ms_lo, name) = match next {
                Some(na) => {
                    // Decompose i64 into i32 hi + u32 lo via little-endian
                    // bytes — sign-loss-free, no `as` casts required.
                    let bytes = na.fire_at_utc_ms.to_le_bytes();
                    let lo = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    let hi = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    (true, hi, lo, cap_alarm_name(&na.name))
                }
                None => (false, 0, 0, String::new()),
            };
            surface.next_alarm(
                presence_to_protocol(present),
                fire_at_utc_ms_hi,
                fire_at_utc_ms_lo,
                name,
            );
        }
    }
}

/// Cap on `next_alarm.name` bytes. Belt-and-braces
/// — operator input is also capped at the gRPC boundary,
/// but on-disk config can bypass that path.
const NEXT_ALARM_NAME_MAX_BYTES: usize = 256;

const NEXT_ALARM_NAME_ELLIPSIS: &str = "…";

/// Truncate `name` to fit `NEXT_ALARM_NAME_MAX_BYTES` with a trailing
/// ellipsis on a UTF-8 char boundary, or return verbatim if it fits.
fn cap_alarm_name(name: &str) -> String {
    if name.len() <= NEXT_ALARM_NAME_MAX_BYTES {
        return name.to_owned();
    }
    let budget = NEXT_ALARM_NAME_MAX_BYTES - NEXT_ALARM_NAME_ELLIPSIS.len();
    let mut out = String::with_capacity(NEXT_ALARM_NAME_MAX_BYTES);
    let mut used = 0;
    // Char walk; `&str` indexing is forbidden by `string_slice` lint.
    for c in name.chars() {
        let c_len = c.len_utf8();
        if used + c_len > budget {
            break;
        }
        out.push(c);
        used += c_len;
    }
    out.push_str(NEXT_ALARM_NAME_ELLIPSIS);
    out
}

#[cfg(test)]
#[derive(Debug, Clone)]
enum RecordedEvent {
    Configure(
        u32,
        u32,
        bmc_widget_protocol::server::deck_widget_surface_v1::ViewportShape,
    ),
    DisplayInfo {
        width: u32,
        height: u32,
        shape: bmc_widget_protocol::server::deck_widget_surface_v1::DisplayShape,
        dpi: u32,
    },
    Params,
    WidgetIdentity,
    Setting,
    ConfigureDone,
}

#[cfg(test)]
#[derive(Default)]
struct RecordingSurface {
    events: std::cell::RefCell<Vec<RecordedEvent>>,
}

#[cfg(test)]
impl WidgetSurface for RecordingSurface {
    fn configure(
        &self,
        width: u32,
        height: u32,
        viewport_shape: bmc_widget_protocol::server::deck_widget_surface_v1::ViewportShape,
    ) {
        self.events
            .borrow_mut()
            .push(RecordedEvent::Configure(width, height, viewport_shape));
    }

    fn display_info(
        &self,
        width: u32,
        height: u32,
        shape: bmc_widget_protocol::server::deck_widget_surface_v1::DisplayShape,
        dpi: u32,
    ) {
        self.events.borrow_mut().push(RecordedEvent::DisplayInfo {
            width,
            height,
            shape,
            dpi,
        });
    }

    fn params(&self, _params_json: String) {
        self.events.borrow_mut().push(RecordedEvent::Params);
    }

    fn widget_identity(&self, _json: String) {
        self.events.borrow_mut().push(RecordedEvent::WidgetIdentity);
    }

    fn emit_setting(&self, _setting: &SettingUpdate) {
        self.events.borrow_mut().push(RecordedEvent::Setting);
    }

    fn configure_done(&self) {
        self.events.borrow_mut().push(RecordedEvent::ConfigureDone);
    }
}

#[cfg(test)]
pub(super) struct RecordedEvents(Vec<RecordedEvent>);

#[cfg(test)]
impl RecordingSurface {
    fn into_events(self) -> RecordedEvents {
        RecordedEvents(self.events.into_inner())
    }
}

#[cfg(test)]
impl RecordedEvents {
    fn names(&self) -> Vec<&'static str> {
        self.0
            .iter()
            .map(|e| match e {
                RecordedEvent::Configure(..) => "configure",
                RecordedEvent::DisplayInfo { .. } => "display_info",
                RecordedEvent::WidgetIdentity => "widget_identity",
                RecordedEvent::Params => "params",
                RecordedEvent::Setting => "setting",
                RecordedEvent::ConfigureDone => "configure_done",
            })
            .collect()
    }

    fn configure(&self) -> Option<(u32, u32, bmc_widget_protocol::ViewportShape)> {
        self.0.iter().find_map(|e| {
            if let RecordedEvent::Configure(w, h, s) = e {
                use bmc_widget_protocol::server::deck_widget_surface_v1::ViewportShape as P;
                let domain_shape = match s {
                    P::Rectangular => bmc_widget_protocol::ViewportShape::Rectangular,
                    P::Round => bmc_widget_protocol::ViewportShape::Round,
                    _ => return None,
                };
                Some((*w, *h, domain_shape))
            } else {
                None
            }
        })
    }

    fn display_info(&self) -> Option<(u32, u32, bmc_widget_protocol::DisplayShape, u32)> {
        self.0.iter().find_map(|e| {
            if let RecordedEvent::DisplayInfo {
                width,
                height,
                shape,
                dpi,
            } = e
            {
                use bmc_widget_protocol::server::deck_widget_surface_v1::DisplayShape as P;
                let domain_shape = match shape {
                    P::Rectangular => bmc_widget_protocol::DisplayShape::Rectangular,
                    P::Round => bmc_widget_protocol::DisplayShape::Round,
                    _ => return None,
                };
                Some((*width, *height, domain_shape, *dpi))
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> WidgetInitialConfig {
        WidgetInitialConfig {
            width: 100,
            height: 100,
            viewport_shape: bmc_widget_protocol::ViewportShape::Rectangular,
            display: bmc_widget_protocol::DisplayInfo::BMC100,
            params: serde_json::Map::new(),
            identity: None,
        }
    }

    fn register_with_pid(state: &mut DeckWidgetProtocolState, instance_id: &str, pid: u32) {
        state.register_widget(instance_id.to_owned(), make_config());
        state.set_widget_pid(&instance_id.to_owned(), pid);
    }

    #[test]
    fn clear_pid_for_instance_unregisters_only_matching_instance_and_pid() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        register_with_pid(&mut state, "beta", 200);
        let _ = state.drain_connected();

        let pid = state.clear_pid_for_instance(&"alpha".to_owned(), 100);

        assert_eq!(pid, Some(100));
        let disconnected = state.drain_disconnected();
        assert_eq!(disconnected, vec!["alpha".to_owned()]);

        assert!(state.widgets.contains_key("beta"));
        assert!(!state.widgets.contains_key("alpha"));
    }

    #[test]
    fn clear_pid_for_instance_with_no_matching_widget_is_noop() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        let _ = state.drain_connected();

        let pid = state.clear_pid_for_instance(&"alpha".to_owned(), 999);

        assert_eq!(pid, None);
        assert!(state.drain_disconnected().is_empty());
        assert!(state.widgets.contains_key("alpha"));
    }

    #[test]
    fn clear_pid_for_instance_ignores_unknown_instance() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        let _ = state.drain_connected();

        let pid = state.clear_pid_for_instance(&"missing".to_owned(), 100);

        assert_eq!(pid, None);
        assert!(state.drain_disconnected().is_empty());
        assert!(state.widgets.contains_key("alpha"));
    }

    #[test]
    fn send_lifecycle_for_unregistered_widget_is_noop() {
        let mut state = DeckWidgetProtocolState::new();
        let client = state.send_lifecycle(
            &String::from("never-registered"),
            crate::compositor::widget_tracker::LifecycleState::Visible,
        );
        assert!(client.is_none());
        assert!(state.drain_disconnected().is_empty());
        assert!(state.drain_connected().is_empty());
    }

    #[test]
    fn clear_pid_for_instance_ignores_stale_exit_after_respawn() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        let _ = state.drain_connected();

        state.set_widget_pid(&"alpha".to_owned(), 200);

        let pid = state.clear_pid_for_instance(&"alpha".to_owned(), 100);

        assert_eq!(pid, None);
        assert!(state.drain_disconnected().is_empty());
        assert!(state.widgets.contains_key("alpha"));
    }

    #[test]
    fn cap_alarm_name_passes_through_when_within_budget() {
        let name = "Wake up";
        assert_eq!(cap_alarm_name(name), name);
    }

    #[test]
    fn cap_alarm_name_passes_through_exactly_at_budget() {
        let name = "a".repeat(NEXT_ALARM_NAME_MAX_BYTES);
        assert_eq!(cap_alarm_name(&name), name);
    }

    #[test]
    fn cap_alarm_name_truncates_with_ellipsis_when_over_budget() {
        let name = "a".repeat(NEXT_ALARM_NAME_MAX_BYTES + 50);
        let capped = cap_alarm_name(&name);
        assert!(
            capped.len() <= NEXT_ALARM_NAME_MAX_BYTES,
            "capped output must fit in {NEXT_ALARM_NAME_MAX_BYTES} bytes; got {}",
            capped.len()
        );
        assert!(
            capped.ends_with(NEXT_ALARM_NAME_ELLIPSIS),
            "truncated output must end with ellipsis"
        );
    }

    #[test]
    fn cap_alarm_name_truncates_on_utf8_char_boundary() {
        // 254 ASCII bytes + 'é' (2 bytes) + 'é' (2 bytes) = 258 bytes.
        // Raw byte cut at 253 (budget = 256 - 3 ellipsis bytes) would
        // land in the middle of the first multibyte 'é'; the helper
        // must walk back to the boundary at 254.
        let mut name = "a".repeat(254);
        name.push('é');
        name.push('é');
        let capped = cap_alarm_name(&name);
        let head = capped
            .strip_suffix(NEXT_ALARM_NAME_ELLIPSIS)
            .expect("BUG: truncated output must end with ellipsis");
        assert!(head.chars().all(|c| c == 'a'));
        assert!(capped.len() <= NEXT_ALARM_NAME_MAX_BYTES);
    }

    #[test]
    fn register_widget_stores_display_info_from_config() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), make_config());
        let stored = state
            .widget_config("alpha")
            .expect("BUG: widget alpha should be registered");
        assert_eq!(stored.display, bmc_widget_protocol::DisplayInfo::BMC100);
    }

    #[test]
    fn emit_initial_state_sends_display_info_between_configure_and_params() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), make_config());

        let events = state
            .test_emit_initial_state_events("alpha")
            .expect("BUG: alpha must be registered");

        assert_eq!(
            events.names(),
            ["configure", "display_info", "params", "configure_done",],
        );
        assert_eq!(
            events.configure(),
            Some((100, 100, bmc_widget_protocol::ViewportShape::Rectangular)),
        );
        assert_eq!(
            events.display_info(),
            Some((
                1_280,
                480,
                bmc_widget_protocol::DisplayShape::Rectangular,
                217
            )),
        );
    }

    #[test]
    fn emit_initial_state_sends_widget_identity_after_display_info() {
        let mut state = DeckWidgetProtocolState::new();
        let mut config = make_config();
        config.identity = Some(
            bmc_widget_protocol::WidgetIdentity::from_wire(r#"{"token":"abcd-2x1"}"#)
                .expect("BUG: valid widget identity json"),
        );
        state.register_widget("alpha".to_owned(), config);

        let events = state
            .test_emit_initial_state_events("alpha")
            .expect("BUG: alpha must be registered");

        assert_eq!(
            events.names(),
            [
                "configure",
                "display_info",
                "widget_identity",
                "params",
                "configure_done"
            ],
        );
    }
}
