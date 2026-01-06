// Copyright (C) 2025  Braiins Systems s.r.o.

//! deck_widget_v1 Wayland protocol handlers.

use bmc::compositor::InstanceId;
use bmc_widget_protocol::{ActionPayload, SettingUpdate};
use bmc_widget_protocol::server::{
    deck_widget_manager_v1::{self, DeckWidgetManagerV1},
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct WidgetData {
    pub instance_id: InstanceId,
    pub surface: Option<DeckWidgetSurfaceV1>,
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

    pub fn register_widget(&mut self, instance_id: InstanceId) {
        self.widgets.insert(
            instance_id.clone(),
            WidgetData {
                instance_id: instance_id.clone(),
                surface: None,
            },
        );
        self.newly_connected.push(instance_id);
    }

    pub fn unregister_widget(&mut self, instance_id: &InstanceId) {
        if self.widgets.remove(instance_id).is_some() {
            self.newly_disconnected.push(instance_id.clone());
        }
    }

    #[must_use]
    pub fn get_widget(&self, instance_id: &InstanceId) -> Option<&WidgetData> {
        self.widgets.get(instance_id)
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
            if let Some(ref surface) = widget_data.surface {
                surface.setting(setting_type, value.clone());
            }
        }
    }

    pub fn broadcast_shutdown(&self) {
        for widget_data in self.widgets.values() {
            if let Some(ref surface) = widget_data.surface {
                surface.shutdown();
            }
        }
    }

    #[must_use]
    pub fn widget_ids(&self) -> Vec<InstanceId> {
        self.widgets.keys().cloned().collect()
    }
}

impl Default for DeckWidgetProtocolState {
    fn default() -> Self {
        Self::new()
    }
}

fn setting_to_protocol(setting: &SettingUpdate) -> (deck_widget_surface_v1::SettingType, String) {
    use deck_widget_surface_v1::SettingType;

    match setting {
        SettingUpdate::Timezone(tz) => (SettingType::Timezone, tz.clone()),
        SettingUpdate::NightMode(enabled) => (SettingType::NightMode, enabled.to_string()),
        SettingUpdate::Localization(localization) => {
            let json = serde_json::to_string(localization).unwrap_or_default();
            (SettingType::Localization, json)
        }
    }
}

fn action_from_protocol(action_type: u32, payload: &str) -> Option<ActionPayload> {
    use deck_widget_surface_v1::ActionType;

    match action_type {
        x if x == ActionType::PlaySound as u32 => Some(ActionPayload::PlaySound {
            sound: payload.to_string(),
        }),
        x if x == ActionType::StopSound as u32 => Some(ActionPayload::StopSound {}),
        x if x == ActionType::Led as u32 => serde_json::from_str(payload).ok(),
        x if x == ActionType::StopLed as u32 => Some(ActionPayload::StopLed {}),
        _ => None,
    }
}

#[derive(Debug)]
pub struct WidgetManagerUserData {
    pub instance_id: Option<InstanceId>,
}

#[derive(Debug)]
pub struct WidgetSurfaceUserData {
    pub instance_id: InstanceId,
}

/// Trait to provide access to protocol state for Dispatch implementations.
pub trait DeckWidgetHandler {
    fn deck_widget_state(&mut self) -> &mut DeckWidgetProtocolState;
}

impl<D> GlobalDispatch<DeckWidgetManagerV1, (), D> for DeckWidgetProtocolState
where
    D: GlobalDispatch<DeckWidgetManagerV1, (), D>
        + Dispatch<DeckWidgetManagerV1, WidgetManagerUserData, D>
        + DeckWidgetHandler
        + 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckWidgetManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, WidgetManagerUserData { instance_id: None });
    }
}

impl<D> Dispatch<DeckWidgetManagerV1, WidgetManagerUserData, D> for DeckWidgetProtocolState
where
    D: Dispatch<DeckWidgetManagerV1, WidgetManagerUserData, D>
        + Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D>
        + DeckWidgetHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &DeckWidgetManagerV1,
        request: deck_widget_manager_v1::Request,
        _data: &WidgetManagerUserData,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            deck_widget_manager_v1::Request::Destroy => {}
            deck_widget_manager_v1::Request::GetWidgetSurface {
                id,
                surface: _,
                instance_id,
            } => {
                let protocol_state = state.deck_widget_state();
                protocol_state.register_widget(instance_id.clone());

                let widget_surface = data_init.init(
                    id,
                    WidgetSurfaceUserData {
                        instance_id: instance_id.clone(),
                    },
                );

                if let Some(widget_data) = protocol_state.get_widget_mut(&instance_id) {
                    widget_data.surface = Some(widget_surface);
                }

                tracing::info!("Widget surface created for instance: {}", instance_id);
            }
            _ => {}
        }
    }
}

impl<D> Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D> for DeckWidgetProtocolState
where
    D: Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D> + DeckWidgetHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &DeckWidgetSurfaceV1,
        request: deck_widget_surface_v1::Request,
        data: &WidgetSurfaceUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            deck_widget_surface_v1::Request::Destroy => {
                let protocol_state = state.deck_widget_state();
                protocol_state.unregister_widget(&data.instance_id);
                tracing::info!("Widget surface destroyed for instance: {}", data.instance_id);
            }
            deck_widget_surface_v1::Request::RequestAction { action_type, payload } => {
                let action_type_u32: u32 = action_type.into();
                if let Some(action_payload) = action_from_protocol(action_type_u32, &payload) {
                    let protocol_state = state.deck_widget_state();
                    protocol_state.add_action(data.instance_id.clone(), action_payload);
                    tracing::debug!(
                        "Widget {} requested action: type={}, payload={}",
                        data.instance_id,
                        action_type_u32,
                        payload
                    );
                }
            }
            _ => {}
        }
    }
}

pub fn create_global<D>(display: &DisplayHandle)
where
    D: GlobalDispatch<DeckWidgetManagerV1, (), D>
        + Dispatch<DeckWidgetManagerV1, WidgetManagerUserData, D>
        + Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D>
        + DeckWidgetHandler
        + 'static,
{
    display.create_global::<D, DeckWidgetManagerV1, ()>(1, ());
}
