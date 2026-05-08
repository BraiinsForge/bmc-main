// Copyright (C) 2025  Braiins Systems s.r.o.

//! Protocol state management for deck_widget_v1.

use std::sync::{Arc, Mutex};

use bmc::compositor::InstanceId;
use bmc_widget_protocol::server::deck_widget_surface_v1::DeckWidgetSurfaceV1;
use bmc_widget_protocol::{ActionPayload, SettingUpdate, WidgetInitialConfig};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::collections::HashMap;

use super::conversions::{
    date_format_to_protocol, night_mode_to_protocol, number_format_to_protocol,
    size_type_to_protocol, temperature_unit_to_protocol, time_format_to_protocol,
    weekday_to_protocol,
};

#[derive(Debug, Clone)]
pub struct WidgetData {
    pub instance_id: InstanceId,
    pub protocol_surface: Option<DeckWidgetSurfaceV1>,
    pub wl_surface: Option<WlSurface>,
    /// PID of the widget process. Used to (1) match a Wayland connection
    /// back to the registered instance via `SO_PEERCRED` at
    /// `get_widget_surface` time, and (2) match Slint render surfaces from
    /// the rendering connection sharing the same process.
    pub pid: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DisconnectedWidget {
    pub instance_id: InstanceId,
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
    /// Initial configuration pushed by the coordinator before spawning a
    /// widget. Consumed on `get_widget_surface` to emit the initial batch
    /// (configure + param_* + settings + configure_done).
    initial_configs: HashMap<InstanceId, WidgetInitialConfig>,
    /// Latest observed value of each runtime setting. Emitted to newly
    /// connecting widgets as part of the initial batch so they start with
    /// a fully populated state instead of waiting for the next change.
    current_settings: Vec<SettingUpdate>,
    pending_actions: Vec<(InstanceId, ActionPayload)>,
    newly_connected: Vec<InstanceId>,
    newly_disconnected: Vec<DisconnectedWidget>,
    /// Connections that arrived before the coordinator called
    /// `set_widget_pid`. Resolved in `set_widget_pid`.
    pending_connections: Vec<PendingConnection>,
}

impl DeckWidgetProtocolState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
            initial_configs: HashMap::new(),
            current_settings: Vec::new(),
            pending_actions: Vec::new(),
            newly_connected: Vec::new(),
            newly_disconnected: Vec::new(),
            pending_connections: Vec::new(),
        }
    }

    /// Store initial config for a widget before it connects.
    pub fn register_initial_config(
        &mut self,
        instance_id: InstanceId,
        config: WidgetInitialConfig,
    ) {
        tracing::info!(
            "Registering initial config for {}: size={:?} {}x{}",
            instance_id,
            config.size,
            config.width,
            config.height
        );
        self.initial_configs.insert(instance_id, config);
    }

    /// Associate a spawned process pid with an instance so that
    /// `get_widget_surface` can resolve the connection's identity via
    /// peer credentials.
    ///
    /// Also resolves any connection that arrived before this call (the
    /// race between process spawn and pid registration).
    pub fn set_widget_pid(&mut self, instance_id: &InstanceId, pid: u32) {
        self.widgets
            .entry(instance_id.clone())
            .or_insert_with(|| WidgetData {
                instance_id: instance_id.clone(),
                protocol_surface: None,
                wl_surface: None,
                pid: None,
            })
            .pid = Some(pid);

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
                .expect("BUG: instance_id lock poisoned")
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
        let entry = self
            .widgets
            .entry(instance_id.clone())
            .or_insert_with(|| WidgetData {
                instance_id: instance_id.clone(),
                protocol_surface: None,
                wl_surface: None,
                pid: None,
            });
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

    pub fn unregister_widget(&mut self, instance_id: &InstanceId) {
        // Purge any buffered connection that would otherwise resolve
        // to this instance's pid after the widget record is gone.
        // Closes the common race windows (scene change, spawn
        // failure) synchronously; the pidfd-on-natural-exit window
        // is handled by `clear_pid`.
        let pid_to_purge = self.widgets.get(instance_id).and_then(|w| w.pid);
        if let Some(pid) = pid_to_purge {
            let before = self.pending_connections.len();
            self.pending_connections.retain(|pc| pc.pid != pid);
            let purged = before - self.pending_connections.len();
            if purged > 0 {
                tracing::info!(
                    "unregister_widget({instance_id}): purged {purged} pending connection(s) with pid={pid}"
                );
            }
        }

        let removed_widget = self.widgets.remove(instance_id);
        let had_config = self.initial_configs.remove(instance_id).is_some();
        if let Some(widget) = removed_widget {
            self.newly_disconnected.push(DisconnectedWidget {
                instance_id: widget.instance_id,
                pid: widget.pid,
            });
        } else if had_config {
            self.newly_disconnected.push(DisconnectedWidget {
                instance_id: instance_id.clone(),
                pid: None,
            });
        }
    }

    /// Remove any pending connection or widget entry associated with the
    /// given pid. Called when a widget process exits so that a recycled
    /// pid cannot be mistaken for the dead widget.
    pub fn clear_pid(&mut self, pid: u32) {
        for widget in self.widgets.values_mut() {
            if widget.pid == Some(pid) {
                widget.pid = None;
            }
        }
        self.pending_connections.retain(|pc| pc.pid != pid);
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

    pub fn drain_disconnected(&mut self) -> Vec<DisconnectedWidget> {
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
        params: &serde_json::Map<String, serde_json::Value>,
    ) {
        if let Some(config) = self.initial_configs.get_mut(instance_id) {
            config.params.clone_from(params);
        }

        let Some(widget_data) = self.widgets.get(instance_id) else {
            tracing::warn!("update_widget_params: no widget record for {instance_id}");
            return;
        };
        let Some(surface) = widget_data.protocol_surface.as_ref() else {
            tracing::warn!("update_widget_params: widget {instance_id} has no surface yet");
            return;
        };

        let params_json = serde_json::Value::Object(params.clone()).to_string();
        surface.params(params_json);
    }

    /// Emit the initial configure batch on the given surface for the
    /// given instance: `configure` → `params` → setting events →
    /// `configure_done`. Called by the dispatch handler right after the
    /// surface role is assigned.
    pub fn emit_initial_state(&self, instance_id: &InstanceId, surface: &DeckWidgetSurfaceV1) {
        let Some(config) = self.initial_configs.get(instance_id) else {
            // This is a programming error — dispatch resolves the
            // instance via pid, and the coordinator guarantees the
            // `register_initial_config` call precedes `set_widget_pid`.
            // Emitting bare `configure_done` makes the widget fail at
            // `wait_for_configure` with a confusing
            // "configure_done without configure" message; log it
            // loudly so the real cause is not buried.
            tracing::error!(
                "emit_initial_state for {instance_id} without prior register_initial_config; widget will fail configure"
            );
            surface.configure_done();
            return;
        };

        surface.configure(
            size_type_to_protocol(config.size),
            config.width,
            config.height,
        );

        // The widget owns its manifest and is authoritative on param
        // validation; the compositor passes through the JSON as-is.
        let params_json = serde_json::Value::Object(config.params.clone()).to_string();
        surface.params(params_json);

        for setting in &self.current_settings {
            emit_setting(surface, setting);
        }

        surface.configure_done();
    }

    pub fn broadcast_shutdown(&self) {
        for widget_data in self.widgets.values() {
            if let Some(ref surface) = widget_data.protocol_surface {
                surface.shutdown();
            }
        }
    }
}

impl Default for DeckWidgetProtocolState {
    fn default() -> Self {
        Self::new()
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
    }
}
