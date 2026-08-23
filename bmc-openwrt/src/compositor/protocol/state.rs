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

//! Protocol state management for deck_widget.

use bmc::compositor::{InstanceId, WidgetConnectionMode, WidgetInstanceKey, WidgetRegistration};
use bmc_widget_protocol::server::deck_widget_surface_v1::DeckWidgetSurfaceV1;
use bmc_widget_protocol::{
    ActionPayload, LedRequestId, LedRequestStatus, SettingUpdate, WidgetInitialConfig,
};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
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
    pub connection_mode: WidgetConnectionMode,
    pub config: WidgetInitialConfig,
    pub protocol_surface: Option<DeckWidgetSurfaceV1>,
    pub wl_surface: Option<WlSurface>,
    pub client_id: Option<ClientId>,
}

#[derive(Debug)]
pub struct DetachedWidget {
    pub client_id: Option<ClientId>,
}

pub enum SurfaceDetach {
    NoMatch,
    Detached,
}

#[derive(Debug)]
pub struct DeckWidgetProtocolState {
    widgets: HashMap<WidgetInstanceKey, WidgetData>,
    /// Latest observed value of each runtime setting. Emitted to newly
    /// connecting widgets as part of the initial batch so they start with
    /// a fully populated state instead of waiting for the next change.
    current_settings: Vec<SettingUpdate>,
    pending_actions: Vec<(InstanceId, ActionPayload)>,
    newly_connected: Vec<InstanceId>,
    newly_disconnected: Vec<InstanceId>,
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
        token: String,
    );
    fn display_info(
        &self,
        width: u32,
        height: u32,
        shape: bmc_widget_protocol::server::deck_widget_surface_v1::DisplayShape,
        dpi: u32,
    );
    fn params(&self, params_json: String);
    fn credentials(&self, credentials_json: String);
    fn credential_secrets(&self, secrets_json: String);
    fn emit_setting(&self, setting: &SettingUpdate);
    fn configure_done(&self);
    /// Negotiated interface version, which decides what may be sent at all.
    fn version(&self) -> u32;
}

/// Interface version that introduced the two credential events.
/// Mirrors `since="2"` in `deck-widget.xml`.
const CREDENTIAL_EVENTS_SINCE: u32 = 2;

impl WidgetSurface for DeckWidgetSurfaceV1 {
    fn configure(
        &self,
        width: u32,
        height: u32,
        viewport_shape: bmc_widget_protocol::server::deck_widget_surface_v1::ViewportShape,
        token: String,
    ) {
        self.configure(width, height, viewport_shape, token);
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

    fn credentials(&self, credentials_json: String) {
        self.credentials(credentials_json);
    }

    fn credential_secrets(&self, secrets_json: String) {
        self.credential_secrets(secrets_json);
    }

    fn emit_setting(&self, setting: &SettingUpdate) {
        emit_setting(self, setting);
    }

    fn configure_done(&self) {
        self.configure_done();
    }

    fn version(&self) -> u32 {
        Resource::version(self)
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
        }
    }

    pub fn register_retained_widget(&mut self, registration: WidgetRegistration) {
        let key = registration.key;
        let instance_id = registration.key.to_string();
        if let Some(widget) = self.widgets.get_mut(&key) {
            widget.config = registration.initial_config;
            return;
        }
        self.widgets.insert(
            key,
            WidgetData {
                instance_id,
                connection_mode: registration.connection_mode,
                config: registration.initial_config,
                protocol_surface: None,
                wl_surface: None,
                client_id: None,
            },
        );
    }

    pub fn activate_widget(&mut self, key: WidgetInstanceKey) -> bool {
        if let Some(widget) = self.widgets.get_mut(&key) {
            widget.connection_mode = WidgetConnectionMode::Accepting;
            true
        } else {
            false
        }
    }

    pub fn deactivate_widget(&mut self, key: WidgetInstanceKey) -> Option<DetachedWidget> {
        let (instance_id, client_id, had_attachment) = {
            let widget = self.widgets.get_mut(&key)?;
            widget.connection_mode = WidgetConnectionMode::Inactive;
            let client_id = widget.client_id.take();
            let protocol_surface = widget.protocol_surface.take();
            let wl_surface = widget.wl_surface.take();
            let had_attachment =
                protocol_surface.is_some() || wl_surface.is_some() || client_id.is_some();
            (widget.instance_id.clone(), client_id, had_attachment)
        };
        self.purge_attachment_events(&instance_id);
        if had_attachment {
            self.newly_disconnected.push(instance_id);
        }
        Some(DetachedWidget { client_id })
    }

    pub fn unregister_retained_widget(&mut self, key: WidgetInstanceKey) -> Option<DetachedWidget> {
        let widget = self.widgets.remove(&key)?;
        self.purge_attachment_events(&widget.instance_id);
        let had_attachment = widget.protocol_surface.is_some()
            || widget.wl_surface.is_some()
            || widget.client_id.is_some();
        if had_attachment {
            self.newly_disconnected.push(widget.instance_id.clone());
        }
        Some(DetachedWidget {
            client_id: widget.client_id,
        })
    }

    /// Attach the wl_surface and protocol surface produced by
    /// `get_widget_surface` to an existing (or freshly promoted) widget
    /// record.
    pub fn attach_surface(
        &mut self,
        instance_id: &InstanceId,
        wl_surface: WlSurface,
        protocol_surface: DeckWidgetSurfaceV1,
    ) -> Option<DetachedWidget> {
        let Some(entry) = self.widget_mut(instance_id) else {
            tracing::error!(
                "attach_surface for {instance_id}: no registered widget for accepted key"
            );
            debug_assert!(
                false,
                "attach_surface called without a registered widget for {instance_id}"
            );
            return None;
        };
        let replaced = (entry.protocol_surface.is_some()
            || entry.wl_surface.is_some()
            || entry.client_id.is_some())
        .then(|| DetachedWidget {
            client_id: entry.client_id.take(),
        });
        if replaced.is_some() {
            self.purge_attachment_events(instance_id);
            self.newly_disconnected.push(instance_id.clone());
        }
        let entry = self
            .widget_mut(instance_id)
            .expect("BUG: accepted widget disappeared while attaching its surface");
        entry.wl_surface = Some(wl_surface);
        entry.client_id = protocol_surface.client().map(|client| client.id());
        entry.protocol_surface = Some(protocol_surface);
        self.newly_connected.push(instance_id.clone());
        replaced
    }

    pub fn accepting_instance_id(&self, key: WidgetInstanceKey) -> Option<&InstanceId> {
        self.widgets
            .get(&key)
            .filter(|widget| widget.connection_mode == WidgetConnectionMode::Accepting)
            .map(|widget| &widget.instance_id)
    }

    pub fn instance_id_for_surface(&self, surface: &WlSurface) -> Option<&InstanceId> {
        self.widgets
            .values()
            .find(|w| w.wl_surface.as_ref().is_some_and(|s| s == surface))
            .map(|w| &w.instance_id)
    }

    pub fn detach_surface(
        &mut self,
        instance_id: &InstanceId,
        client_id: &ClientId,
        protocol_surface_id: &ObjectId,
    ) -> SurfaceDetach {
        let Some(widget) = self.widget_mut(instance_id) else {
            return SurfaceDetach::NoMatch;
        };
        let Some(stored_surface) = widget.protocol_surface.as_ref().map(Resource::id) else {
            return SurfaceDetach::NoMatch;
        };
        let Some(stored_client) = widget.client_id.as_ref() else {
            return SurfaceDetach::NoMatch;
        };
        if stored_client != client_id || stored_surface != *protocol_surface_id {
            return SurfaceDetach::NoMatch;
        }
        widget.client_id = None;
        widget.protocol_surface = None;
        widget.wl_surface = None;
        self.purge_attachment_events(instance_id);
        self.newly_disconnected.push(instance_id.clone());
        SurfaceDetach::Detached
    }

    pub fn is_current_attachment(
        &self,
        instance_id: &InstanceId,
        client_id: &ClientId,
        protocol_surface_id: &ObjectId,
    ) -> bool {
        self.widget(instance_id).is_some_and(|widget| {
            widget.client_id.as_ref() == Some(client_id)
                && widget.protocol_surface.as_ref().map(Resource::id)
                    == Some(protocol_surface_id.clone())
        })
    }

    pub fn has_attachment(&self, instance_id: &InstanceId) -> bool {
        self.widget(instance_id)
            .is_some_and(|widget| widget.protocol_surface.is_some())
    }

    fn purge_attachment_events(&mut self, instance_id: &InstanceId) {
        self.pending_actions
            .retain(|(queued_instance_id, _)| queued_instance_id != instance_id);
        self.newly_connected
            .retain(|queued_instance_id| queued_instance_id != instance_id);
        self.newly_disconnected
            .retain(|queued_instance_id| queued_instance_id != instance_id);
    }

    #[cfg(test)]
    pub(crate) fn queue_connected_for_test(&mut self, instance_id: InstanceId) {
        self.newly_connected.push(instance_id);
    }

    #[cfg(test)]
    pub(crate) fn attach_protocol_surface_for_test(
        &mut self,
        instance_id: &InstanceId,
        protocol_surface: DeckWidgetSurfaceV1,
    ) {
        let widget = self
            .widget_mut(instance_id)
            .expect("BUG: test attachment requires a registration");
        widget.client_id = protocol_surface.client().map(|client| client.id());
        widget.protocol_surface = Some(protocol_surface);
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
        key: WidgetInstanceKey,
        params: serde_json::Map<String, serde_json::Value>,
    ) {
        let Some(widget_data) = self.widgets.get_mut(&key) else {
            tracing::warn!("update_widget_params: no widget record for {key}");
            return;
        };
        widget_data.config.params = params;

        let Some(surface) = widget_data.protocol_surface.as_ref() else {
            tracing::warn!("update_widget_params: widget {key} has no surface yet");
            return;
        };

        let params_json = serde_json::Value::Object(widget_data.config.params.clone()).to_string();
        surface.params(params_json);
    }

    /// Also refreshes the stored config, so a reconnect replays the resolution.
    /// Callers push without checking liveness,
    /// so a missing record or surface is the designed skip, not an anomaly.
    ///
    /// Returns whether the stored resolution changed.
    /// A record with no surface still counts as changed:
    /// a crash-looping widget has none.
    pub fn update_widget_credentials(
        &mut self,
        key: WidgetInstanceKey,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: bmc_widget_protocol::CredentialSecrets,
    ) -> bool {
        let Some(widget_data) = self.widgets.get_mut(&key) else {
            tracing::debug!("update_widget_credentials: no widget record for {key}");
            return false;
        };
        if !credentials_changed(&widget_data.config, &credentials, &secrets) {
            return false;
        }
        widget_data.config.credentials = credentials;
        widget_data.config.credential_secrets = secrets;

        if let Some(surface) = widget_data.protocol_surface.as_ref() {
            emit_credentials(surface, &widget_data.config);
        } else {
            tracing::debug!("update_widget_credentials: widget {key} has no surface yet");
        }
        true
    }

    /// Emit the initial configure batch on the given surface for the
    /// given instance: `configure` → `display_info` → `params` → setting events →
    /// `configure_done`. Called by the dispatch handler right after the
    /// surface role is assigned.
    pub fn emit_initial_state(&self, instance_id: &InstanceId, surface: &DeckWidgetSurfaceV1) {
        self.emit_initial_state_into(instance_id, surface);
    }

    fn emit_initial_state_into<S: WidgetSurface>(&self, instance_id: &InstanceId, surface: &S) {
        let Some(widget) = self.widget(instance_id) else {
            tracing::error!(
                "emit_initial_state for {instance_id}: no registered widget matches the surface key"
            );
            surface.configure_done();
            return;
        };
        let config = &widget.config;

        surface.configure(
            config.width,
            config.height,
            config.viewport_shape.into(),
            config.token.clone(),
        );

        surface.display_info(
            config.display.width,
            config.display.height,
            config.display.shape.into(),
            config.display.dpi,
        );

        let params_json = serde_json::Value::Object(config.params.clone()).to_string();
        surface.params(params_json);

        emit_credentials(surface, config);

        for setting in &self.current_settings {
            surface.emit_setting(setting);
        }

        surface.configure_done();
    }

    #[cfg(test)]
    pub(super) fn widget_config(&self, instance_id: &str) -> Option<&WidgetInitialConfig> {
        self.widget(instance_id).map(|w| &w.config)
    }

    #[cfg(test)]
    pub(super) fn test_emit_initial_state_events(
        &self,
        instance_id: &str,
    ) -> Option<RecordedEvents> {
        self.test_emit_initial_state_events_into(instance_id, RecordingSurface::default())
    }

    #[cfg(test)]
    pub(super) fn test_emit_initial_state_events_into(
        &self,
        instance_id: &str,
        sink: RecordingSurface,
    ) -> Option<RecordedEvents> {
        self.widget(instance_id)?;
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
        let Some(widget) = self.widget(instance_id) else {
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
        let Some(widget) = self.widget(instance_id) else {
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
        let Some(widget) = self.widget(instance_id) else {
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

    fn widget(&self, instance_id: &str) -> Option<&WidgetData> {
        self.widgets.get(&instance_id.parse().ok()?)
    }

    fn widget_mut(&mut self, instance_id: &str) -> Option<&mut WidgetData> {
        self.widgets.get_mut(&instance_id.parse().ok()?)
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

fn credentials_changed(
    stored: &WidgetInitialConfig,
    credentials: &serde_json::Map<String, serde_json::Value>,
    secrets: &bmc_widget_protocol::CredentialSecrets,
) -> bool {
    &stored.credentials != credentials || &stored.credential_secrets != secrets
}

/// Emit the guest-visible view and then the secrets, always as a pair:
/// a widget that saw a slot appear must be able to spend it.
///
/// Both events are `since="2"`. A version-1 peer is not merely uninterested:
/// sending it an event it has no opcode for would desynchronise the stream.
fn emit_credentials<S: WidgetSurface>(surface: &S, config: &WidgetInitialConfig) {
    if surface.version() < CREDENTIAL_EVENTS_SINCE {
        return;
    }
    surface.credentials(serde_json::Value::Object(config.credentials.clone()).to_string());
    surface.credential_secrets(config.credential_secrets.to_json_string());
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
    Credentials(String),
    CredentialSecrets(String),
    Setting,
    ConfigureDone,
}

#[cfg(test)]
pub(super) struct RecordingSurface {
    events: std::cell::RefCell<Vec<RecordedEvent>>,
    version: u32,
}

#[cfg(test)]
impl Default for RecordingSurface {
    fn default() -> Self {
        Self {
            events: std::cell::RefCell::default(),
            version: CREDENTIAL_EVENTS_SINCE,
        }
    }
}

#[cfg(test)]
impl RecordingSurface {
    /// A peer that negotiated an older interface version.
    fn at_version(version: u32) -> Self {
        Self {
            version,
            ..Self::default()
        }
    }
}

#[cfg(test)]
impl WidgetSurface for RecordingSurface {
    fn configure(
        &self,
        width: u32,
        height: u32,
        viewport_shape: bmc_widget_protocol::server::deck_widget_surface_v1::ViewportShape,
        _token: String,
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

    fn credentials(&self, credentials_json: String) {
        self.events
            .borrow_mut()
            .push(RecordedEvent::Credentials(credentials_json));
    }

    fn credential_secrets(&self, secrets_json: String) {
        self.events
            .borrow_mut()
            .push(RecordedEvent::CredentialSecrets(secrets_json));
    }

    fn emit_setting(&self, _setting: &SettingUpdate) {
        self.events.borrow_mut().push(RecordedEvent::Setting);
    }

    fn configure_done(&self) {
        self.events.borrow_mut().push(RecordedEvent::ConfigureDone);
    }

    fn version(&self) -> u32 {
        self.version
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
    /// The `credentials` and `credential_secrets` payloads, in emit order.
    fn credential_payloads(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter_map(|e| match e {
                RecordedEvent::Credentials(json) | RecordedEvent::CredentialSecrets(json) => {
                    Some(json.as_str())
                }
                RecordedEvent::Configure(..)
                | RecordedEvent::DisplayInfo { .. }
                | RecordedEvent::Params
                | RecordedEvent::Setting
                | RecordedEvent::ConfigureDone => None,
            })
            .collect()
    }

    fn names(&self) -> Vec<&'static str> {
        self.0
            .iter()
            .map(|e| match e {
                RecordedEvent::Configure(..) => "configure",
                RecordedEvent::DisplayInfo { .. } => "display_info",
                RecordedEvent::Params => "params",
                RecordedEvent::Credentials(_) => "credentials",
                RecordedEvent::CredentialSecrets(_) => "credential_secrets",
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
    use crate::compositor::{
        protocol::WidgetSurfaceUserData,
        state::{ClientState, CompositorState},
    };
    use bmc::compositor::WidgetPlacement;
    use bmc_widget_protocol::CredentialSecrets;
    use smithay::reexports::wayland_server::{
        Display, Resource,
        backend::{Handle, ObjectData, protocol::Message},
        protocol::wl_surface::WlSurface,
    };
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    const TEST_INSTANCE_ID: &str = "00000000-0000-0000-0000-000000000001";

    fn test_instance_key() -> WidgetInstanceKey {
        TEST_INSTANCE_ID
            .parse()
            .expect("BUG: test widget instance ID must be a canonical key")
    }

    struct TestObjectData;

    impl ObjectData<CompositorState> for TestObjectData {
        fn request(
            self: Arc<Self>,
            _: &Handle,
            _: &mut CompositorState,
            _: ClientId,
            _: Message<ObjectId, OwnedFd>,
        ) -> Option<Arc<dyn ObjectData<CompositorState>>> {
            None
        }

        fn destroyed(
            self: Arc<Self>,
            _: &Handle,
            _: &mut CompositorState,
            _: ClientId,
            _: ObjectId,
        ) {
        }
    }

    fn retained_registration(mode: WidgetConnectionMode) -> WidgetRegistration {
        let key = WidgetInstanceKey::from(bmc::scene::WidgetId::generate());
        WidgetRegistration {
            key,
            connection_mode: mode,
            placement: WidgetPlacement {
                instance_id: key.to_string(),
                position: bmc::compositor::Position { x: 5, y: 7 },
                size: bmc::compositor::Size {
                    width: 100,
                    height: 100,
                },
                visible: true,
            },
            initial_config: make_config(),
        }
    }

    fn make_config() -> WidgetInitialConfig {
        WidgetInitialConfig {
            width: 100,
            height: 100,
            viewport_shape: bmc_widget_protocol::ViewportShape::Rectangular,
            display: bmc_widget_protocol::DisplayInfo::BMC100,
            params: serde_json::Map::new(),
            credentials: serde_json::Map::new(),
            credential_secrets: bmc_widget_protocol::CredentialSecrets::default(),
            token: "test-instance-2x1".to_owned(),
        }
    }

    fn register_test_widget(
        state: &mut DeckWidgetProtocolState,
        instance_id: InstanceId,
        config: WidgetInitialConfig,
    ) {
        let key = instance_id
            .parse()
            .expect("BUG: test widget instance ID must be a canonical key");
        state.widgets.insert(
            key,
            WidgetData {
                instance_id,
                connection_mode: WidgetConnectionMode::Accepting,
                config,
                protocol_surface: None,
                wl_surface: None,
                client_id: None,
            },
        );
    }

    #[test]
    fn retained_reregistration_updates_values_without_detaching_the_client() {
        let display =
            Display::<CompositorState>::new().expect("BUG: test Wayland display should initialize");
        let mut handle = display.handle();
        let (socket, _peer) =
            UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
        let client = handle
            .insert_client(socket, Arc::new(ClientState::default()))
            .expect("BUG: test Wayland client should register");
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;
        let instance_id = registration.key.to_string();
        state.register_retained_widget(registration.clone());
        let protocol_surface = client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(
                &handle,
                2,
                WidgetSurfaceUserData {
                    instance_id: instance_id.clone(),
                },
            )
            .expect("BUG: test protocol surface should initialize");
        let protocol_surface_id = protocol_surface.id();
        let wl_surface = client
            .create_resource_from_objdata::<WlSurface, CompositorState>(
                &handle,
                6,
                Arc::new(TestObjectData),
            )
            .expect("BUG: test wl_surface should initialize");
        let wl_surface_id = wl_surface.id();
        state.attach_surface(&instance_id, wl_surface, protocol_surface);
        let _ = state.drain_connected();

        let mut updated = registration;
        updated.initial_config.params.insert(
            "label".to_owned(),
            serde_json::Value::String("current".to_owned()),
        );
        state.register_retained_widget(updated);

        let stored = state
            .widget_config(&instance_id)
            .expect("BUG: retained registration must remain present");
        assert_eq!(
            stored.params.get("label"),
            Some(&serde_json::Value::String("current".to_owned()))
        );
        let widget = &state.widgets[&key];
        assert_eq!(
            widget.protocol_surface.as_ref().map(Resource::id),
            Some(protocol_surface_id)
        );
        assert_eq!(
            widget.wl_surface.as_ref().map(Resource::id),
            Some(wl_surface_id)
        );
        assert_eq!(widget.client_id.as_ref(), Some(&client.id()));
        assert!(state.drain_disconnected().is_empty());
        assert!(state.test_emit_initial_state_events(&instance_id).is_some());
    }

    #[test]
    fn activation_is_idempotent_and_cannot_create_a_registration() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Inactive);
        let key = registration.key;

        assert!(!state.activate_widget(key));
        assert!(!state.widgets.contains_key(&key));

        state.register_retained_widget(registration);
        assert_eq!(
            state.widgets[&key].connection_mode,
            WidgetConnectionMode::Inactive
        );
        assert!(state.activate_widget(key));
        assert!(state.activate_widget(key));
        assert_eq!(
            state.widgets[&key].connection_mode,
            WidgetConnectionMode::Accepting
        );
    }

    #[test]
    fn keyed_admission_accepts_only_a_retained_accepting_registration() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;

        assert!(state.accepting_instance_id(key).is_none());
        state.register_retained_widget(registration);
        let instance_id = key.to_string();
        assert_eq!(state.accepting_instance_id(key), Some(&instance_id));
        state.deactivate_widget(key);
        assert!(state.accepting_instance_id(key).is_none());
    }

    #[test]
    fn retained_reregistration_preserves_mode_while_updating_config() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;
        state.register_retained_widget(registration);

        let mut inactive_update = retained_registration(WidgetConnectionMode::Inactive);
        inactive_update.key = key;
        inactive_update
            .placement
            .instance_id
            .clone_from(&instance_id);
        inactive_update
            .initial_config
            .params
            .insert("revision".to_owned(), serde_json::json!(1));
        state.register_retained_widget(inactive_update);
        assert_eq!(
            state.widgets[&key].connection_mode,
            WidgetConnectionMode::Accepting
        );
        assert_eq!(
            state.widgets[&key].config.params["revision"],
            serde_json::json!(1)
        );

        state.deactivate_widget(key);
        let mut accepting_update = retained_registration(WidgetConnectionMode::Accepting);
        accepting_update.key = key;
        accepting_update
            .placement
            .instance_id
            .clone_from(&instance_id);
        accepting_update
            .initial_config
            .params
            .insert("revision".to_owned(), serde_json::json!(2));
        state.register_retained_widget(accepting_update);
        assert_eq!(
            state.widgets[&key].connection_mode,
            WidgetConnectionMode::Inactive
        );
        assert_eq!(
            state.widgets[&key].config.params["revision"],
            serde_json::json!(2)
        );
    }

    #[test]
    fn lifecycle_cutoff_purges_queued_attachment_events() {
        let mut state = DeckWidgetProtocolState::new();
        let first = retained_registration(WidgetConnectionMode::Accepting);
        let first_key = first.key;
        let first_id = first_key.to_string();
        state.register_retained_widget(first);
        state.queue_connected_for_test(first_id.clone());
        state.add_action(first_id.clone(), ActionPayload::StopSound {});

        state.deactivate_widget(first_key);
        assert!(state.drain_connected().is_empty());
        assert!(state.drain_actions().is_empty());

        let second = retained_registration(WidgetConnectionMode::Accepting);
        let second_key = second.key;
        let second_id = second_key.to_string();
        state.register_retained_widget(second);
        state.queue_connected_for_test(second_id.clone());
        state.add_action(second_id, ActionPayload::StopSound {});

        state.unregister_retained_widget(second_key);
        assert!(state.drain_connected().is_empty());
        assert!(state.drain_actions().is_empty());
    }

    #[test]
    fn deactivation_clears_every_attachment_field() {
        let display =
            Display::<CompositorState>::new().expect("BUG: test Wayland display should initialize");
        let mut handle = display.handle();
        let (socket, _peer) =
            UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
        let client = handle
            .insert_client(socket, Arc::new(ClientState::default()))
            .expect("BUG: test Wayland client should register");
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;
        let instance_id = key.to_string();
        let protocol_surface = client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(
                &handle,
                2,
                WidgetSurfaceUserData {
                    instance_id: instance_id.clone(),
                },
            )
            .expect("BUG: test protocol surface should initialize");
        let wl_surface = client
            .create_resource_from_objdata::<WlSurface, CompositorState>(
                &handle,
                6,
                Arc::new(TestObjectData),
            )
            .expect("BUG: test wl_surface should initialize");
        let mut state = DeckWidgetProtocolState::new();
        state.register_retained_widget(registration);
        state.attach_surface(&instance_id, wl_surface, protocol_surface);

        state.deactivate_widget(key);

        let widget = &state.widgets[&key];
        assert!(widget.protocol_surface.is_none());
        assert!(widget.wl_surface.is_none());
        assert!(widget.client_id.is_none());
    }

    #[test]
    fn repeated_deactivate_and_unregister_retain_then_remove_configuration() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;
        let instance_id = key.to_string();
        state.register_retained_widget(registration);

        assert!(state.deactivate_widget(key).is_some());
        assert!(state.deactivate_widget(key).is_some());
        assert_eq!(
            state.widgets[&key].connection_mode,
            WidgetConnectionMode::Inactive
        );
        assert!(state.widget_config(&instance_id).is_some());

        assert!(state.unregister_retained_widget(key).is_some());
        assert!(state.unregister_retained_widget(key).is_none());
        assert!(!state.widgets.contains_key(&key));
    }

    #[test]
    fn stale_attachment_identity_cannot_detach_a_replacement() {
        let display =
            Display::<CompositorState>::new().expect("BUG: test Wayland display should initialize");
        let mut handle = display.handle();
        let (first_socket, _first_peer) =
            UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
        let first_client = handle
            .insert_client(first_socket, Arc::new(ClientState::default()))
            .expect("BUG: first test client should register");
        let (second_socket, _second_peer) =
            UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
        let second_client = handle
            .insert_client(second_socket, Arc::new(ClientState::default()))
            .expect("BUG: second test client should register");

        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;
        let instance_id = registration.key.to_string();
        let user_data = || WidgetSurfaceUserData {
            instance_id: instance_id.clone(),
        };
        let first_surface = first_client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(&handle, 2, user_data())
            .expect("BUG: first test protocol surface should initialize");
        let replacement_surface = second_client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(&handle, 2, user_data())
            .expect("BUG: replacement test protocol surface should initialize");

        let mut state = DeckWidgetProtocolState::new();
        state.register_retained_widget(registration);
        state.attach_protocol_surface_for_test(&instance_id, first_surface.clone());
        state.attach_protocol_surface_for_test(&instance_id, replacement_surface.clone());

        assert!(matches!(
            state.detach_surface(&instance_id, &first_client.id(), &first_surface.id()),
            SurfaceDetach::NoMatch
        ));
        assert!(matches!(
            state.detach_surface(&instance_id, &second_client.id(), &first_surface.id()),
            SurfaceDetach::NoMatch
        ));
        assert_eq!(
            state.widgets[&key]
                .protocol_surface
                .as_ref()
                .map(Resource::id),
            Some(replacement_surface.id())
        );
        assert!(matches!(
            state.detach_surface(&instance_id, &second_client.id(), &replacement_surface.id()),
            SurfaceDetach::Detached
        ));
    }

    #[test]
    fn replacement_and_stale_destruction_preserve_exact_attachment_intervals() {
        let display =
            Display::<CompositorState>::new().expect("BUG: test Wayland display should initialize");
        let mut handle = display.handle();
        let (socket, _peer) =
            UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
        let client = handle
            .insert_client(socket, Arc::new(ClientState::default()))
            .expect("BUG: test Wayland client should register");
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let instance_id = registration.key.to_string();
        let user_data = || WidgetSurfaceUserData {
            instance_id: instance_id.clone(),
        };
        let first_protocol = client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(&handle, 2, user_data())
            .expect("BUG: first protocol surface should initialize");
        let second_protocol = client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(&handle, 2, user_data())
            .expect("BUG: second protocol surface should initialize");
        let first_wl = client
            .create_resource_from_objdata::<WlSurface, CompositorState>(
                &handle,
                6,
                Arc::new(TestObjectData),
            )
            .expect("BUG: first wl_surface should initialize");
        let second_wl = client
            .create_resource_from_objdata::<WlSurface, CompositorState>(
                &handle,
                6,
                Arc::new(TestObjectData),
            )
            .expect("BUG: second wl_surface should initialize");

        let mut state = DeckWidgetProtocolState::new();
        state.register_retained_widget(registration);
        assert!(
            state
                .attach_surface(&instance_id, first_wl, first_protocol.clone())
                .is_none()
        );
        assert_eq!(state.drain_connected(), std::slice::from_ref(&instance_id));
        state.add_action(instance_id.clone(), ActionPayload::StopSound {});

        let replaced = state
            .attach_surface(&instance_id, second_wl, second_protocol.clone())
            .expect("BUG: replacement must detach the first interval");
        assert_eq!(replaced.client_id, Some(client.id()));
        assert!(state.drain_actions().is_empty());
        assert_eq!(
            state.drain_disconnected(),
            std::slice::from_ref(&instance_id)
        );
        assert_eq!(state.drain_connected(), std::slice::from_ref(&instance_id));
        assert!(state.is_current_attachment(&instance_id, &client.id(), &second_protocol.id()));

        assert!(matches!(
            state.detach_surface(&instance_id, &client.id(), &first_protocol.id()),
            SurfaceDetach::NoMatch
        ));
        assert!(state.drain_disconnected().is_empty());
        assert!(matches!(
            state.detach_surface(&instance_id, &client.id(), &second_protocol.id()),
            SurfaceDetach::Detached
        ));
        assert_eq!(state.drain_disconnected(), [instance_id]);
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
    fn registration_stores_display_info() {
        let mut state = DeckWidgetProtocolState::new();
        register_test_widget(&mut state, TEST_INSTANCE_ID.to_owned(), make_config());
        let stored = state
            .widget_config(TEST_INSTANCE_ID)
            .expect("BUG: test widget should be registered");
        assert_eq!(stored.display, bmc_widget_protocol::DisplayInfo::BMC100);
    }

    #[test]
    fn emit_initial_state_sends_display_info_between_configure_and_params() {
        let mut state = DeckWidgetProtocolState::new();
        register_test_widget(&mut state, TEST_INSTANCE_ID.to_owned(), make_config());

        let events = state
            .test_emit_initial_state_events(TEST_INSTANCE_ID)
            .expect("BUG: test widget must be registered");

        assert_eq!(
            events.names(),
            [
                "configure",
                "display_info",
                "params",
                "credentials",
                "credential_secrets",
                "configure_done",
            ],
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
    fn a_version_1_peer_gets_the_batch_without_credential_events() {
        let mut state = DeckWidgetProtocolState::new();
        register_test_widget(&mut state, TEST_INSTANCE_ID.to_owned(), make_config());

        let events = state
            .test_emit_initial_state_events_into(TEST_INSTANCE_ID, RecordingSurface::at_version(1))
            .expect("BUG: test widget must be registered");

        assert_eq!(
            events.names(),
            ["configure", "display_info", "params", "configure_done"],
            "an event a v1 peer has no opcode for would desynchronise its stream"
        );
    }

    fn bound_pool() -> (
        serde_json::Map<String, serde_json::Value>,
        CredentialSecrets,
    ) {
        let as_object = |v: serde_json::Value| {
            v.as_object()
                .expect("BUG: json! literal is an object")
                .clone()
        };
        let view = as_object(
            serde_json::json!({ "pool": { "type": "braiins-pool", "account": "My pool" } }),
        );
        let secrets = as_object(serde_json::json!({ "pool": { "fields": { "token": "s3cr3t" } } }));

        (view, CredentialSecrets::new(secrets))
    }

    #[test]
    fn a_bound_widget_replays_both_credential_events_on_reconnect() {
        let mut state = DeckWidgetProtocolState::new();
        let (view, secrets) = bound_pool();
        register_test_widget(&mut state, TEST_INSTANCE_ID.to_owned(), make_config());
        state.update_widget_credentials(test_instance_key(), view, secrets);

        let events = state
            .test_emit_initial_state_events(TEST_INSTANCE_ID)
            .expect("BUG: test widget must be registered");

        let payloads = events.credential_payloads();
        assert!(
            payloads[0].contains("My pool") && !payloads[0].contains("s3cr3t"),
            "the guest-visible half must name the account and carry no secret: {}",
            payloads[0]
        );
        assert!(payloads[1].contains("s3cr3t"));
    }

    #[test]
    fn an_identical_resolution_is_not_a_change() {
        let (view, secrets) = bound_pool();
        let mut stored = make_config();
        stored.credentials = view.clone();
        stored.credential_secrets = secrets.clone();

        assert!(!credentials_changed(&stored, &view, &secrets));
    }

    #[test]
    fn a_rotated_secret_is_a_change_even_though_the_view_is_identical() {
        let (view, secrets) = bound_pool();
        let mut stored = make_config();
        stored.credentials = view.clone();
        stored.credential_secrets = secrets;

        let rotated = CredentialSecrets::new(
            serde_json::json!({ "pool": { "fields": { "token": "rotated" } } })
                .as_object()
                .expect("BUG: json! literal is an object")
                .clone(),
        );

        assert!(
            credentials_changed(&stored, &view, &rotated),
            "a token rotation never shows in the view, so only the secret half can catch it"
        );
    }

    #[test]
    fn unbinding_is_a_change() {
        let (view, secrets) = bound_pool();
        let mut stored = make_config();
        stored.credentials = view;
        stored.credential_secrets = secrets;

        assert!(credentials_changed(
            &stored,
            &serde_json::Map::new(),
            &CredentialSecrets::default()
        ));
    }

    #[test]
    fn unbinding_clears_the_stored_resolution() {
        let mut state = DeckWidgetProtocolState::new();
        register_test_widget(&mut state, TEST_INSTANCE_ID.to_owned(), make_config());
        let (view, secrets) = bound_pool();
        state.update_widget_credentials(test_instance_key(), view, secrets);

        state.update_widget_credentials(
            test_instance_key(),
            serde_json::Map::new(),
            CredentialSecrets::default(),
        );

        let stored = state
            .widget_config(TEST_INSTANCE_ID)
            .expect("BUG: test widget must be registered");
        assert!(stored.credentials.is_empty() && stored.credential_secrets.is_empty());
    }

    /// A crash-looping widget has no surface,
    /// and it is the one whose respawn a credential change re-arms.
    #[test]
    fn a_surfaceless_record_still_reports_a_credential_change() {
        let mut state = DeckWidgetProtocolState::new();
        register_test_widget(&mut state, TEST_INSTANCE_ID.to_owned(), make_config());
        let (view, secrets) = bound_pool();

        assert!(state.update_widget_credentials(test_instance_key(), view, secrets));
    }

    #[test]
    fn a_missing_record_reports_no_credential_change() {
        let mut state = DeckWidgetProtocolState::new();
        let (view, secrets) = bound_pool();

        assert!(!state.update_widget_credentials(test_instance_key(), view, secrets));
    }

    #[test]
    fn a_repeated_credential_push_reports_no_change() {
        let mut state = DeckWidgetProtocolState::new();
        register_test_widget(&mut state, TEST_INSTANCE_ID.to_owned(), make_config());
        let (view, secrets) = bound_pool();
        state.update_widget_credentials(test_instance_key(), view.clone(), secrets.clone());

        assert!(!state.update_widget_credentials(test_instance_key(), view, secrets));
    }
}
