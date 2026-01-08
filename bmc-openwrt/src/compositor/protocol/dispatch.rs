// Copyright (C) 2025  Braiins Systems s.r.o.

//! Wayland dispatch implementations for deck_widget_v1.

use bmc::compositor::InstanceId;
use bmc_widget_protocol::server::{
    deck_widget_manager_v1::{self, DeckWidgetManagerV1},
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};
use bmc_widget_protocol::wayland_server::Resource;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};

use super::conversions::action_from_protocol;
use super::state::DeckWidgetProtocolState;

#[derive(Debug)]
pub struct WidgetManagerUserData;

#[derive(Debug)]
pub struct WidgetSurfaceUserData {
    pub instance_id: InstanceId,
}

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
        data_init.init(resource, WidgetManagerUserData);
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
        client: &Client,
        _resource: &DeckWidgetManagerV1,
        request: deck_widget_manager_v1::Request,
        _data: &WidgetManagerUserData,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        if let deck_widget_manager_v1::Request::GetWidgetSurface {
            id,
            surface,
            instance_id,
        } = request
        {
            // Get client PID for matching render surfaces from Slint connection
            #[expect(clippy::cast_sign_loss, reason = "PID is always positive")]
            let pid = client
                .get_credentials(dhandle)
                .ok()
                .map(|creds| creds.pid as u32);

            tracing::info!(
                "GetWidgetSurface: instance={} surface={:?} pid={:?}",
                instance_id,
                surface.id(),
                pid
            );

            let protocol_state = state.deck_widget_state();
            protocol_state.register_widget(instance_id.clone(), Some(surface.clone()), pid);

            let widget_surface = data_init.init(
                id,
                WidgetSurfaceUserData {
                    instance_id: instance_id.clone(),
                },
            );

            if let Some(widget_data) = protocol_state.get_widget_mut(&instance_id) {
                widget_data.protocol_surface = Some(widget_surface);
            }

            tracing::info!("Widget surface created for instance: {}", instance_id);
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
                tracing::info!(
                    "Widget surface destroyed for instance: {}",
                    data.instance_id
                );
            }
            deck_widget_surface_v1::Request::RequestAction {
                action_type,
                payload,
            } => {
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
