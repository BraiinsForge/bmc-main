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

//! Protocol state management for deck_widget_v1.

use std::sync::{Arc, Mutex};

use bmc::compositor::{
    InstanceId, WidgetConnectionMode, WidgetGeneration, WidgetInstanceKey, WidgetRegistration,
};
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
    generation: Option<WidgetGeneration>,
    pub connection_mode: WidgetConnectionMode,
    pub config: WidgetInitialConfig,
    pub protocol_surface: Option<DeckWidgetSurfaceV1>,
    pub wl_surface: Option<WlSurface>,
    pub client_id: Option<ClientId>,
    /// PID of the widget process. Used to (1) match a Wayland connection
    /// back to the registered instance via `SO_PEERCRED` at
    /// `get_widget_surface` time, and (2) match Slint render surfaces from
    /// the rendering connection sharing the same process.
    pub pid: Option<u32>,
    /// A pid whose exit was reported before anything bound it,
    /// so that a later bind cannot take it.
    exited_before_bind: Option<u32>,
}

#[derive(Debug)]
pub struct DetachedWidget {
    pub client_id: Option<ClientId>,
    pub pid: Option<u32>,
}

pub enum SurfaceDetach {
    NoMatch,
    Detached { pid: Option<u32> },
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
/// Mirrors `since="2"` in `deck-widget-v1.xml`.
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
            pending_connections: Vec::new(),
        }
    }

    pub fn register_widget(
        &mut self,
        instance_id: InstanceId,
        generation: WidgetGeneration,
        config: WidgetInitialConfig,
    ) {
        tracing::info!(
            "Registering widget {} (generation {}): {}x{} viewport_shape={:?}",
            instance_id,
            generation,
            config.width,
            config.height,
            config.viewport_shape
        );
        self.widgets.insert(
            instance_id.clone(),
            WidgetData {
                instance_id,
                generation: Some(generation),
                connection_mode: WidgetConnectionMode::Accepting,
                config,
                protocol_surface: None,
                wl_surface: None,
                client_id: None,
                pid: None,
                exited_before_bind: None,
            },
        );
    }

    pub fn register_retained_widget(&mut self, registration: WidgetRegistration) {
        let instance_id = registration.key.to_string();
        if let Some(widget) = self.widgets.get_mut(&instance_id) {
            widget.config = registration.initial_config;
            return;
        }
        self.widgets.insert(
            instance_id.clone(),
            WidgetData {
                instance_id,
                generation: None,
                connection_mode: registration.connection_mode,
                config: registration.initial_config,
                protocol_surface: None,
                wl_surface: None,
                client_id: None,
                pid: None,
                exited_before_bind: None,
            },
        );
    }

    pub fn activate_widget(&mut self, key: WidgetInstanceKey) {
        if let Some(widget) = self.widgets.get_mut(&key.to_string()) {
            widget.connection_mode = WidgetConnectionMode::Accepting;
        }
    }

    pub fn deactivate_widget(&mut self, key: WidgetInstanceKey) -> Option<DetachedWidget> {
        let instance_id = key.to_string();
        let (client_id, pid, had_attachment) = {
            let widget = self.widgets.get_mut(&instance_id)?;
            widget.connection_mode = WidgetConnectionMode::Inactive;
            let client_id = widget.client_id.take();
            let protocol_surface = widget.protocol_surface.take();
            let wl_surface = widget.wl_surface.take();
            let had_attachment =
                protocol_surface.is_some() || wl_surface.is_some() || client_id.is_some();
            let pid = widget.pid.take();
            (client_id, pid, had_attachment)
        };
        if let Some(pid) = pid {
            self.purge_pending_connections(&instance_id, pid);
        }
        self.purge_attachment_events(&instance_id);
        if had_attachment {
            self.newly_disconnected.push(instance_id);
        }
        Some(DetachedWidget { client_id, pid })
    }

    pub fn unregister_retained_widget(&mut self, key: WidgetInstanceKey) -> Option<DetachedWidget> {
        let instance_id = key.to_string();
        let widget = self.widgets.remove(&instance_id)?;
        if let Some(pid) = widget.pid {
            self.purge_pending_connections(&instance_id, pid);
        }
        self.purge_attachment_events(&instance_id);
        let had_attachment = widget.protocol_surface.is_some()
            || widget.wl_surface.is_some()
            || widget.client_id.is_some();
        if had_attachment {
            self.newly_disconnected.push(widget.instance_id.clone());
        }
        Some(DetachedWidget {
            client_id: widget.client_id,
            pid: widget.pid,
        })
    }

    /// Associate a spawned process pid with an instance so that
    /// `get_widget_surface` can resolve the connection's identity via
    /// peer credentials.
    ///
    /// Also resolves any connection that arrived before this call (the
    /// race between process spawn and pid registration).
    pub fn set_widget_pid(
        &mut self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
        pid: u32,
    ) {
        if self
            .widgets
            .get(instance_id)
            .is_some_and(|widget| widget.connection_mode == WidgetConnectionMode::Inactive)
        {
            self.purge_pending_connections(instance_id, pid);
            return;
        }
        if self
            .widgets
            .get(instance_id)
            .is_some_and(|widget| widget.generation != Some(generation))
        {
            tracing::debug!(
                "set_widget_pid for {instance_id}: generation {generation} has been re-registered; dropping the bind of pid={pid}"
            );
            self.purge_pending_connections(instance_id, pid);
            return;
        }
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
        if widget.exited_before_bind == Some(pid) {
            widget.exited_before_bind = None;
            tracing::warn!(
                "set_widget_pid for {instance_id}: pid={pid} exited before this bind; leaving the instance unbound for its respawn"
            );
            return;
        }
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

    /// Bind a crash-respawned process, returning whether the bind happened.
    ///
    /// Takes effect only on the registration the respawn belongs to,
    /// and only while that registration is still unbound; see
    /// `Compositor::bind_respawned_pid` for why a stale bind must be dropped.
    pub fn bind_respawned_pid(
        &mut self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
        pid: u32,
    ) -> bool {
        // Warn, unlike the superseded case below: no later call binds this pid,
        // so the respawned process can never reach a surface.
        let current = self
            .widgets
            .get(instance_id)
            .and_then(|widget| widget.generation);
        if current != Some(generation) {
            tracing::warn!(
                "bind_respawned_pid: generation {generation} of {instance_id} is gone (now {current:?}); pid={pid} is left with nothing to resolve it"
            );
            self.purge_pending_connections(instance_id, pid);
            return false;
        }
        let widget = self
            .widgets
            .get_mut(instance_id)
            .expect("BUG: the generation check proved the record is there");
        if let Some(bound) = widget.pid {
            tracing::debug!(
                "bind_respawned_pid: instance {instance_id} is already bound to pid={bound}; dropping the superseded bind of pid={pid}"
            );
            self.purge_pending_connections(instance_id, pid);
            return false;
        }
        // A recycled pid belongs to the live process this respawn announces,
        // so a tombstone naming that pid is stale and may go. One naming a
        // different pid is not: its own set_widget_pid may still be queued.
        if widget.exited_before_bind == Some(pid) {
            widget.exited_before_bind = None;
        }
        self.set_widget_pid(instance_id, generation, pid);
        true
    }

    /// Drop an instance supervision gave up on, returning whether it went.
    ///
    /// Guarded like [`Self::bind_respawned_pid`]:
    /// only the registration this names can be the one it tears down.
    pub fn unregister_abandoned(
        &mut self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
    ) -> bool {
        let current = self
            .widgets
            .get(instance_id)
            .and_then(|widget| widget.generation);
        if current != Some(generation) {
            tracing::debug!(
                "unregister_abandoned: generation {generation} of {instance_id} is already gone (now {current:?})"
            );
            return false;
        }
        let widget = self
            .widgets
            .get(instance_id)
            .expect("BUG: the generation check proved the record is there");
        if let Some(pid) = widget.pid {
            tracing::debug!(
                "unregister_abandoned: instance {instance_id} is bound to pid={pid}; dropping the superseded abandon"
            );
            return false;
        }
        self.unregister_widget(instance_id);
        true
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
        entry.client_id = protocol_surface.client().map(|client| client.id());
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

        if let Some(pid) = widget.pid {
            self.purge_pending_connections(instance_id, pid);
        }

        let pid = widget.pid;
        self.newly_disconnected.push(widget.instance_id);
        pid
    }

    pub fn detach_surface(
        &mut self,
        instance_id: &InstanceId,
        client_id: &ClientId,
        protocol_surface_id: &ObjectId,
    ) -> SurfaceDetach {
        let Some(widget) = self.widgets.get_mut(instance_id) else {
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
        let pid = widget.pid;
        self.purge_attachment_events(instance_id);
        self.newly_disconnected.push(instance_id.clone());
        SurfaceDetach::Detached { pid }
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
            .widgets
            .get_mut(instance_id)
            .expect("BUG: test attachment requires a registration");
        widget.client_id = protocol_surface.client().map(|client| client.id());
        widget.protocol_surface = Some(protocol_surface);
    }

    /// Synthesize a disconnect for an exited widget process,
    /// keeping the instance registered.
    ///
    /// A crashed or SIGTERM'd widget can exit without sending protocol
    /// `Destroy`, so the coordinator emits this call from its child-exit
    /// watcher. To avoid PID-reuse races, disconnection is guarded by both
    /// instance id and expected pid: stale exit notifications for a prior
    /// spawn of the same instance are ignored.
    ///
    /// The pid and both surfaces go, so nothing resolves to the dead process.
    /// `config` stays, so the respawn replays the same configure batch
    /// as the first attach and only has to re-run `set_widget_pid`.
    /// Dropping the whole record here would leave it nothing to bind to.
    pub fn clear_pid_for_instance(
        &mut self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
        expected_pid: u32,
    ) -> Option<u32> {
        let current = self
            .widgets
            .get(instance_id)
            .and_then(|widget| widget.generation);
        if current != Some(generation) {
            tracing::debug!(
                "clear_pid_for_instance: ignoring stale clear for generation {generation} of {instance_id} (now {current:?}, expected_pid={expected_pid})"
            );
            return None;
        }
        let widget = self
            .widgets
            .get_mut(instance_id)
            .expect("BUG: the generation check proved the record is there");
        if widget.pid.is_none() {
            widget.exited_before_bind = Some(expected_pid);
            tracing::debug!(
                "clear_pid_for_instance: {instance_id} is unbound; recording pid={expected_pid} as dead"
            );
            self.purge_pending_connections(instance_id, expected_pid);
            return None;
        }
        if widget.pid != Some(expected_pid) {
            tracing::debug!(
                "clear_pid_for_instance: ignoring stale clear for instance {instance_id}: expected pid {expected_pid}, current pid {:?}",
                widget.pid
            );
            return None;
        }

        widget.pid = None;
        widget.wl_surface = None;
        widget.protocol_surface = None;
        widget.client_id = None;

        self.purge_pending_connections(instance_id, expected_pid);
        self.newly_disconnected.push(instance_id.clone());
        Some(expected_pid)
    }

    /// Drop buffered connections of a pid nothing will resolve: the instance
    /// ends, only its process does, or the bind that would have claimed the
    /// pid is refused.
    fn purge_pending_connections(&mut self, instance_id: &InstanceId, pid: u32) {
        let before = self.pending_connections.len();
        self.pending_connections.retain(|pc| pc.pid != pid);
        let purged = before - self.pending_connections.len();
        if purged > 0 {
            tracing::info!("{instance_id}: purged {purged} pending connection(s) with pid={pid}");
        }
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

    /// Also refreshes the stored config, so a reconnect replays the resolution.
    /// Callers push without checking liveness,
    /// so a missing record or surface is the designed skip, not an anomaly.
    ///
    /// Returns whether the stored resolution changed.
    /// A record with no surface still counts as changed:
    /// a crash-looping widget has none.
    pub fn update_widget_credentials(
        &mut self,
        instance_id: &InstanceId,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: bmc_widget_protocol::CredentialSecrets,
    ) -> bool {
        let Some(widget_data) = self.widgets.get_mut(instance_id) else {
            tracing::debug!("update_widget_credentials: no widget record for {instance_id}");
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
            tracing::debug!("update_widget_credentials: widget {instance_id} has no surface yet");
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
        let Some(widget) = self.widgets.get(instance_id) else {
            tracing::error!(
                "emit_initial_state for {instance_id}: no widget record; dispatch resolved a pid that has no registered widget"
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
        self.widgets.get(instance_id).map(|w| &w.config)
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
        if !self.widgets.contains_key(instance_id) {
            return None;
        }
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

    const GEN: WidgetGeneration = WidgetGeneration(1);
    const NEXT_GEN: WidgetGeneration = WidgetGeneration(2);

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

    #[test]
    fn retained_registration_updates_detached_initial_batch() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let instance_id = registration.key.to_string();
        state.register_retained_widget(registration.clone());

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
        assert!(state.test_emit_initial_state_events(&instance_id).is_some());
    }

    #[test]
    fn retained_registration_update_does_not_detach_the_current_process() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let instance_id = registration.key.to_string();
        state.register_widget(instance_id.clone(), GEN, make_config());
        state.set_widget_pid(&instance_id, GEN, 123);

        state.register_retained_widget(registration);

        assert_eq!(state.widgets[&instance_id].pid, Some(123));
        assert!(state.drain_disconnected().is_empty());
    }

    #[test]
    fn activation_is_idempotent_and_cannot_create_a_registration() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Inactive);
        let key = registration.key;

        state.activate_widget(key);
        assert!(!state.widgets.contains_key(&key.to_string()));

        state.register_retained_widget(registration);
        assert_eq!(
            state.widgets[&key.to_string()].connection_mode,
            WidgetConnectionMode::Inactive
        );
        state.set_widget_pid(&key.to_string(), GEN, 123);
        assert_eq!(state.widgets[&key.to_string()].pid, None);
        state.activate_widget(key);
        state.activate_widget(key);
        assert_eq!(
            state.widgets[&key.to_string()].connection_mode,
            WidgetConnectionMode::Accepting
        );
    }

    #[test]
    fn retained_reregistration_preserves_mode_while_updating_config() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;
        let instance_id = key.to_string();
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
            state.widgets[&instance_id].connection_mode,
            WidgetConnectionMode::Accepting
        );
        assert_eq!(
            state.widgets[&instance_id].config.params["revision"],
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
            state.widgets[&instance_id].connection_mode,
            WidgetConnectionMode::Inactive
        );
        assert_eq!(
            state.widgets[&instance_id].config.params["revision"],
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
                    instance_id: Arc::new(Mutex::new(instance_id.clone())),
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

        let widget = &state.widgets[&instance_id];
        assert!(widget.protocol_surface.is_none());
        assert!(widget.wl_surface.is_none());
        assert!(widget.client_id.is_none());
    }

    #[test]
    fn deactivation_clears_legacy_pid_and_refuses_late_rebind() {
        let mut state = DeckWidgetProtocolState::new();
        let registration = retained_registration(WidgetConnectionMode::Accepting);
        let key = registration.key;
        let instance_id = key.to_string();
        state.register_widget(instance_id.clone(), GEN, make_config());
        state.set_widget_pid(&instance_id, GEN, 123);
        state.register_retained_widget(registration);

        state.deactivate_widget(key);
        state.set_widget_pid(&instance_id, GEN, 123);

        assert_eq!(state.widgets[&instance_id].pid, None);
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
            state.widgets[&instance_id].connection_mode,
            WidgetConnectionMode::Inactive
        );
        assert!(state.widget_config(&instance_id).is_some());

        assert!(state.unregister_retained_widget(key).is_some());
        assert!(state.unregister_retained_widget(key).is_none());
        assert!(!state.widgets.contains_key(&instance_id));
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
        let instance_id = registration.key.to_string();
        let user_data = || WidgetSurfaceUserData {
            instance_id: Arc::new(Mutex::new(instance_id.clone())),
        };
        let first_surface = first_client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(&handle, 2, user_data())
            .expect("BUG: first test protocol surface should initialize");
        let replacement_surface = second_client
            .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(&handle, 2, user_data())
            .expect("BUG: replacement test protocol surface should initialize");

        let mut state = DeckWidgetProtocolState::new();
        state.register_retained_widget(registration);
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
            state.widgets[&instance_id]
                .protocol_surface
                .as_ref()
                .map(Resource::id),
            Some(replacement_surface.id())
        );
        assert!(matches!(
            state.detach_surface(&instance_id, &second_client.id(), &replacement_surface.id()),
            SurfaceDetach::Detached { .. }
        ));
    }

    fn register_with_pid(state: &mut DeckWidgetProtocolState, instance_id: &str, pid: u32) {
        state.register_widget(instance_id.to_owned(), GEN, make_config());
        state.set_widget_pid(&instance_id.to_owned(), GEN, pid);
    }

    #[test]
    fn clear_pid_for_instance_detaches_only_matching_instance_and_pid() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        register_with_pid(&mut state, "beta", 200);
        let _ = state.drain_connected();

        let pid = state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);

        assert_eq!(pid, Some(100));
        let disconnected = state.drain_disconnected();
        assert_eq!(disconnected, vec!["alpha".to_owned()]);

        assert_eq!(
            state.widgets["beta"].pid,
            Some(200),
            "clearing alpha must leave beta attached"
        );
        assert_eq!(
            state.widgets["alpha"].pid, None,
            "the exited process must be detached from its instance"
        );
    }

    /// The crash-respawn path: the instance survives its process,
    /// so the respawn has a record to bind its new pid to.
    #[test]
    fn cleared_instance_stays_registered_and_rebinds_on_respawn() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        let _ = state.drain_connected();

        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);
        let _ = state.drain_disconnected();

        assert!(
            state.widget_config("alpha").is_some(),
            "the stored config must outlive the crashed process"
        );

        assert!(state.bind_respawned_pid(&"alpha".to_owned(), GEN, 200));

        assert_eq!(
            state.widgets["alpha"].pid,
            Some(200),
            "the respawn must bind through bind_respawned_pid alone"
        );
        assert_eq!(
            state.instance_id_for_surface_by_pid(Some(200)),
            Some(&"alpha".to_owned()),
            "the respawned process must resolve to its instance"
        );
    }

    /// The respawn announcement is drained separately from the scene edits and
    /// reload signals that also re-spawn an instance, so it can arrive after one
    /// of them has already bound a newer process. Binding then would point the
    /// record at a dead pid and leave the live process's buffered connection
    /// with nothing left to resolve it.
    #[test]
    fn a_superseded_respawn_bind_is_ignored() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);

        register_with_pid(&mut state, "alpha", 300);

        assert!(
            !state.bind_respawned_pid(&"alpha".to_owned(), GEN, 200),
            "a respawn must not bind over an instance that is already bound"
        );
        assert_eq!(
            state.widgets["alpha"].pid,
            Some(300),
            "the live pid must survive the stale respawn"
        );
    }

    /// The coordinator binds the pid only once `spawn_widget` has returned it,
    /// so a process that exits inside that window is reported against a record
    /// nothing has bound yet. Binding the dead pid afterwards would leave every
    /// respawn dropped as superseded, and the cell blank for good.
    #[test]
    fn an_exit_racing_the_initial_bind_leaves_the_instance_respawnable() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());

        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);
        state.set_widget_pid(&"alpha".to_owned(), GEN, 100);

        assert_eq!(
            state.widgets["alpha"].pid, None,
            "a pid already reported dead must not bind"
        );

        assert!(state.bind_respawned_pid(&"alpha".to_owned(), GEN, 200));
        assert_eq!(
            state.widgets["alpha"].pid,
            Some(200),
            "supervision's respawn must still find the instance bindable"
        );
    }

    /// The kernel is free to hand the respawn the pid
    /// the exited process just released.
    #[test]
    fn a_respawn_supersedes_a_recorded_premature_exit() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());
        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);

        assert!(state.bind_respawned_pid(&"alpha".to_owned(), GEN, 100));
        assert_eq!(
            state.widgets["alpha"].pid,
            Some(100),
            "a recycled pid belongs to the live process the respawn announced"
        );
    }

    /// A stop between the respawn and its announcement ends the instance. The
    /// bind must then be a no-op rather than the `set_widget_pid` assert firing.
    #[test]
    fn a_respawn_bind_for_a_stopped_instance_is_ignored() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);
        state.unregister_widget(&"alpha".to_owned());

        assert!(
            !state.bind_respawned_pid(&"alpha".to_owned(), GEN, 200),
            "there is no instance left to bind to"
        );
        assert!(!state.widgets.contains_key("alpha"));
    }

    /// Supervision gives up only on an instance it left unbound,
    /// so the abandon must find that instance and end it.
    #[test]
    fn an_abandoned_instance_is_unregistered() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);
        let _ = state.drain_disconnected();

        assert!(state.unregister_abandoned(&"alpha".to_owned(), GEN));
        assert!(
            !state.widgets.contains_key("alpha"),
            "the instance a widget type outlived must not stay registered"
        );
    }

    /// The abandon is drained separately from the scene edits that re-spawn an
    /// instance, so it can arrive after one has bound a live process. Ending
    /// the instance then would blank a cell nothing is going to bring back.
    #[test]
    fn a_superseded_abandon_leaves_the_live_instance_alone() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);

        register_with_pid(&mut state, "alpha", 300);

        assert!(!state.unregister_abandoned(&"alpha".to_owned(), GEN));
        assert_eq!(
            state.widgets["alpha"].pid,
            Some(300),
            "the live process must survive the stale abandon"
        );
    }

    /// The coordinator's own `set_widget_pid` for the first process can still
    /// be queued when the respawn arrives. Disarming the tombstone that call
    /// was recorded for lets it bind a pid the kernel has already reaped.
    #[test]
    fn a_respawn_leaves_a_tombstone_for_another_pid_armed() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());
        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);

        assert!(state.bind_respawned_pid(&"alpha".to_owned(), GEN, 200));
        state.set_widget_pid(&"alpha".to_owned(), GEN, 100);

        assert_eq!(
            state.widgets["alpha"].pid,
            Some(200),
            "the respawned process must survive the late bind of the dead pid"
        );
    }

    /// A fresh registration is unbound until its `set_widget_pid` lands,
    /// exactly like the state a crash leaves behind.
    #[test]
    fn a_respawn_bind_from_a_previous_registration_is_refused() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 200);
        state.unregister_widget(&"alpha".to_owned());
        state.register_widget("alpha".to_owned(), NEXT_GEN, make_config());

        assert!(
            !state.bind_respawned_pid(&"alpha".to_owned(), GEN, 200),
            "a bind stamped for a registration that is gone must not land"
        );
        assert_eq!(
            state.widgets["alpha"].pid, None,
            "the fresh registration must stay unbound for its own set_widget_pid"
        );
    }

    /// A stale clear taking the unbound arm records a tombstone
    /// the new registration never asked for.
    /// Pid recycling is what makes that tombstone refuse the live bind.
    #[test]
    fn a_clear_from_a_previous_registration_leaves_no_tombstone() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        state.unregister_widget(&"alpha".to_owned());
        state.register_widget("alpha".to_owned(), NEXT_GEN, make_config());

        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);
        state.set_widget_pid(&"alpha".to_owned(), NEXT_GEN, 100);

        assert_eq!(
            state.widgets["alpha"].pid,
            Some(100),
            "the new registration must still be able to bind its own process"
        );
    }

    /// The abandon is drained separately from the scene edit that re-spawns
    /// the instance, so it can arrive while the new registration is unbound.
    #[test]
    fn an_abandon_from_a_previous_registration_is_refused() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());
        state.unregister_widget(&"alpha".to_owned());
        state.register_widget("alpha".to_owned(), NEXT_GEN, make_config());

        assert!(!state.unregister_abandoned(&"alpha".to_owned(), GEN));
        assert!(
            state.widgets.contains_key("alpha"),
            "a registration still mid-spawn must survive a stale abandon"
        );
    }

    #[test]
    fn a_pid_bind_from_a_previous_registration_is_refused() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());
        state.unregister_widget(&"alpha".to_owned());
        state.register_widget("alpha".to_owned(), NEXT_GEN, make_config());

        state.set_widget_pid(&"alpha".to_owned(), GEN, 100);

        assert_eq!(
            state.widgets["alpha"].pid, None,
            "the record must stay unbound for the pid of its own registration"
        );
    }

    #[test]
    fn unregister_widget_after_a_clear_removes_the_instance() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        let _ = state.drain_connected();

        state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);
        state.unregister_widget(&"alpha".to_owned());

        assert!(
            !state.widgets.contains_key("alpha"),
            "stopping a crashed instance must still end it"
        );
    }

    #[test]
    fn clear_pid_for_instance_with_no_matching_widget_is_noop() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        let _ = state.drain_connected();

        let pid = state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 999);

        assert_eq!(pid, None);
        assert!(state.drain_disconnected().is_empty());
        assert!(state.widgets.contains_key("alpha"));
    }

    #[test]
    fn clear_pid_for_instance_ignores_unknown_instance() {
        let mut state = DeckWidgetProtocolState::new();
        register_with_pid(&mut state, "alpha", 100);
        let _ = state.drain_connected();

        let pid = state.clear_pid_for_instance(&"missing".to_owned(), GEN, 100);

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

        state.set_widget_pid(&"alpha".to_owned(), GEN, 200);

        let pid = state.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);

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
        state.register_widget("alpha".to_owned(), GEN, make_config());
        let stored = state
            .widget_config("alpha")
            .expect("BUG: widget alpha should be registered");
        assert_eq!(stored.display, bmc_widget_protocol::DisplayInfo::BMC100);
    }

    #[test]
    fn emit_initial_state_sends_display_info_between_configure_and_params() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());

        let events = state
            .test_emit_initial_state_events("alpha")
            .expect("BUG: alpha must be registered");

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
        state.register_widget("alpha".to_owned(), GEN, make_config());

        let events = state
            .test_emit_initial_state_events_into("alpha", RecordingSurface::at_version(1))
            .expect("BUG: alpha must be registered");

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
        state.register_widget("alpha".to_owned(), GEN, make_config());
        state.update_widget_credentials(&"alpha".to_owned(), view, secrets);

        let events = state
            .test_emit_initial_state_events("alpha")
            .expect("BUG: alpha must be registered");

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
        state.register_widget("alpha".to_owned(), GEN, make_config());
        let (view, secrets) = bound_pool();
        state.update_widget_credentials(&"alpha".to_owned(), view, secrets);

        state.update_widget_credentials(
            &"alpha".to_owned(),
            serde_json::Map::new(),
            CredentialSecrets::default(),
        );

        let stored = state.widget_config("alpha").expect("BUG: registered");
        assert!(stored.credentials.is_empty() && stored.credential_secrets.is_empty());
    }

    /// A crash-looping widget has no surface,
    /// and it is the one whose respawn a credential change re-arms.
    #[test]
    fn a_surfaceless_record_still_reports_a_credential_change() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());
        let (view, secrets) = bound_pool();

        assert!(state.update_widget_credentials(&"alpha".to_owned(), view, secrets));
    }

    #[test]
    fn a_repeated_credential_push_reports_no_change() {
        let mut state = DeckWidgetProtocolState::new();
        state.register_widget("alpha".to_owned(), GEN, make_config());
        let (view, secrets) = bound_pool();
        state.update_widget_credentials(&"alpha".to_owned(), view.clone(), secrets.clone());

        assert!(!state.update_widget_credentials(&"alpha".to_owned(), view, secrets));
    }
}
