// Copyright (C) 2025  Braiins Systems s.r.o.

//! Protocol state management for deck_widget_v1.

use bmc::compositor::InstanceId;
use bmc_widget_protocol::server::deck_widget_surface_v1::DeckWidgetSurfaceV1;
use bmc_widget_protocol::{ActionPayload, SettingUpdate};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::collections::HashMap;

use super::conversions::setting_to_protocol;

#[derive(Debug, Clone)]
pub struct WidgetData {
    pub instance_id: InstanceId,
    pub protocol_surface: Option<DeckWidgetSurfaceV1>,
    pub wl_surface: Option<WlSurface>,
    /// PID of the widget process (used to match render surfaces from Slint connection)
    pub pid: Option<u32>,
}

#[derive(Debug)]
pub struct DeckWidgetProtocolState {
    widgets: HashMap<InstanceId, WidgetData>,
    pending_actions: Vec<(InstanceId, ActionPayload)>,
    newly_connected: Vec<InstanceId>,
    newly_disconnected: Vec<InstanceId>,
}

impl DeckWidgetProtocolState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
            pending_actions: Vec::new(),
            newly_connected: Vec::new(),
            newly_disconnected: Vec::new(),
        }
    }

    pub fn register_widget(
        &mut self,
        instance_id: InstanceId,
        wl_surface: Option<WlSurface>,
        pid: Option<u32>,
    ) {
        tracing::info!("Registering widget {} with pid={:?}", instance_id, pid);
        self.widgets.insert(
            instance_id.clone(),
            WidgetData {
                instance_id: instance_id.clone(),
                protocol_surface: None,
                wl_surface,
                pid,
            },
        );
        self.newly_connected.push(instance_id);
    }

    /// Find instance_id for a surface, matching by PID if available.
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
        if self.widgets.remove(instance_id).is_some() {
            self.newly_disconnected.push(instance_id.clone());
        }
    }

    pub fn get_widget_mut(&mut self, instance_id: &InstanceId) -> Option<&mut WidgetData> {
        self.widgets.get_mut(instance_id)
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

    pub fn broadcast_setting(&self, setting: &SettingUpdate) {
        let (setting_type, value) = setting_to_protocol(setting);

        for widget_data in self.widgets.values() {
            if let Some(ref surface) = widget_data.protocol_surface {
                surface.setting(setting_type, value.clone());
            }
        }
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
