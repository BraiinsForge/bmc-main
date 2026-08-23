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

//! Wayland dispatch implementations for deck_widget.

use bmc::compositor::{InstanceId, WidgetInstanceKey};
use bmc_widget_protocol::server::{
    deck_widget_manager_v2::{self, DeckWidgetManagerV2},
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientId, ObjectId},
};

use super::conversions::{led_effect_from_protocol, led_scope_from_protocol};
use super::state::{DeckWidgetProtocolState, SurfaceDetach};

fn rgb_from_protocol(
    widget: &str,
    r: u32,
    g: u32,
    b: u32,
) -> Option<bmc_widget_protocol::RgbColor> {
    let (Ok(r), Ok(g), Ok(b)) = (u8::try_from(r), u8::try_from(g), u8::try_from(b)) else {
        tracing::warn!(%widget, r, g, b, "RGB component out of u8 range; dropping request");
        return None;
    };
    Some(bmc_widget_protocol::RgbColor { r, g, b })
}

#[derive(Debug)]
pub struct WidgetManagerUserData;

#[derive(Debug, Clone)]
pub struct WidgetSurfaceUserData {
    pub instance_id: InstanceId,
}

pub trait DeckWidgetHandler {
    fn deck_widget_state(&mut self) -> &mut DeckWidgetProtocolState;

    fn drop_widget_render_state(&mut self, _instance_id: &InstanceId) {}

    fn forget_widget_lifecycle(&mut self, _instance_id: &InstanceId) {}

    fn detach_widget_surface(
        &mut self,
        instance_id: &InstanceId,
        client_id: &ClientId,
        protocol_surface_id: &ObjectId,
    ) {
        if let SurfaceDetach::Detached =
            self.deck_widget_state()
                .detach_surface(instance_id, client_id, protocol_surface_id)
        {
            self.drop_widget_render_state(instance_id);
            self.forget_widget_lifecycle(instance_id);
        }
    }

    fn attach_widget_surface(
        &mut self,
        instance_id: &InstanceId,
        wl_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        protocol_surface: DeckWidgetSurfaceV1,
    ) -> Option<ClientId> {
        let new_client_id = protocol_surface.client().map(|client| client.id());
        let replaced =
            self.deck_widget_state()
                .attach_surface(instance_id, wl_surface, protocol_surface);
        if let Some(replaced) = replaced {
            self.drop_widget_render_state(instance_id);
            self.forget_widget_lifecycle(instance_id);
            return replaced
                .client_id
                .filter(|old_client_id| Some(old_client_id) != new_client_id.as_ref());
        }
        None
    }
}

impl<D> GlobalDispatch<DeckWidgetManagerV2, (), D> for DeckWidgetProtocolState
where
    D: GlobalDispatch<DeckWidgetManagerV2, (), D>
        + Dispatch<DeckWidgetManagerV2, WidgetManagerUserData, D>
        + DeckWidgetHandler
        + 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckWidgetManagerV2>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, WidgetManagerUserData);
    }
}

impl<D> Dispatch<DeckWidgetManagerV2, WidgetManagerUserData, D> for DeckWidgetProtocolState
where
    D: Dispatch<DeckWidgetManagerV2, WidgetManagerUserData, D>
        + Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D>
        + DeckWidgetHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &DeckWidgetManagerV2,
        request: deck_widget_manager_v2::Request,
        _data: &WidgetManagerUserData,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            deck_widget_manager_v2::Request::GetWidgetSurface {
                id,
                widget_instance_key,
                surface,
            } => {
                let Ok(key) = widget_instance_key.parse::<WidgetInstanceKey>() else {
                    resource.post_error(
                        deck_widget_manager_v2::Error::InvalidKey,
                        format!("noncanonical widget instance key {widget_instance_key:?}"),
                    );
                    return;
                };
                let protocol_state = state.deck_widget_state();
                let Some(instance_id) = protocol_state.accepting_instance_id(key).cloned() else {
                    resource.post_error(
                        deck_widget_manager_v2::Error::UnknownWidget,
                        format!("widget instance {key} is not accepting connections"),
                    );
                    return;
                };
                let widget_surface = data_init.init(
                    id,
                    WidgetSurfaceUserData {
                        instance_id: instance_id.clone(),
                    },
                );
                if let Some(replaced_client) =
                    state.attach_widget_surface(&instance_id, surface, widget_surface.clone())
                {
                    dhandle.backend_handle().kill_client(
                        replaced_client,
                        smithay::reexports::wayland_server::backend::DisconnectReason::ConnectionClosed,
                    );
                }
                state
                    .deck_widget_state()
                    .emit_initial_state(&instance_id, &widget_surface);
            }
            deck_widget_manager_v2::Request::Destroy => {}
            other => tracing::warn!("Ignoring unknown keyed widget manager request: {other:?}"),
        }
    }
}

impl<D> Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D> for DeckWidgetProtocolState
where
    D: Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D> + DeckWidgetHandler + 'static,
{
    #[expect(
        clippy::too_many_lines,
        reason = "single big match over the protocol surface; splitting per-arm helpers would add indirection without clarity"
    )]
    fn request(
        state: &mut D,
        client: &Client,
        resource: &DeckWidgetSurfaceV1,
        request: deck_widget_surface_v1::Request,
        data: &WidgetSurfaceUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let instance_id = data.instance_id.clone();
        let current_attachment = state.deck_widget_state().is_current_attachment(
            &instance_id,
            &client.id(),
            &resource.id(),
        );
        match request {
            deck_widget_surface_v1::Request::Destroy => {
                state.detach_widget_surface(&instance_id, &client.id(), &resource.id());
                tracing::info!("Widget surface destroyed for instance: {}", instance_id);
            }
            _ if !current_attachment => {
                tracing::debug!(
                    %instance_id,
                    client = ?client.id(),
                    surface = ?resource.id(),
                    "ignoring request from stale widget attachment"
                );
            }
            deck_widget_surface_v1::Request::PlaySound { sound } => {
                let protocol_state = state.deck_widget_state();
                tracing::debug!("Widget {} play_sound: {sound}", instance_id);
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::PlaySound { sound },
                );
            }
            deck_widget_surface_v1::Request::StopSound => {
                let protocol_state = state.deck_widget_state();
                tracing::debug!("Widget {} stop_sound", instance_id);
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::StopSound {},
                );
            }
            deck_widget_surface_v1::Request::LedTemporary {
                request_id,
                effect,
                r,
                g,
                b,
                period_ms,
                duration_ms,
                scope,
            } => {
                let Ok(effect) = effect.into_result() else {
                    tracing::warn!("Widget {} led_temporary: unknown effect", instance_id);
                    return;
                };
                let Ok(scope) = scope.into_result() else {
                    tracing::warn!("Widget {} led_temporary: unknown scope", instance_id);
                    return;
                };
                if request_id == bmc_widget_protocol::LED_REQUEST_ID_ALL {
                    tracing::warn!(
                        "Widget {} led_temporary: request_id 0 is reserved; ignoring",
                        instance_id
                    );
                    return;
                }
                let protocol_state = state.deck_widget_state();
                tracing::debug!(
                    "Widget {} led_temporary: req={request_id} effect={effect:?} rgb=({r},{g},{b}) period_ms={period_ms} duration_ms={duration_ms} scope={scope:?}",
                    instance_id
                );
                let Some(color) = rgb_from_protocol(&instance_id, r, g, b) else {
                    return;
                };
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::LedTemporary {
                        request_id,
                        effect: led_effect_from_protocol(effect),
                        color,
                        period_ms,
                        duration_ms,
                        scope: led_scope_from_protocol(scope),
                    },
                );
            }
            deck_widget_surface_v1::Request::LedEndless {
                request_id,
                effect,
                r,
                g,
                b,
                period_ms,
                scope,
            } => {
                let Ok(effect) = effect.into_result() else {
                    tracing::warn!("Widget {} led_endless: unknown effect", instance_id);
                    return;
                };
                let Ok(scope) = scope.into_result() else {
                    tracing::warn!("Widget {} led_endless: unknown scope", instance_id);
                    return;
                };
                if request_id == bmc_widget_protocol::LED_REQUEST_ID_ALL {
                    tracing::warn!(
                        "Widget {} led_endless: request_id 0 is reserved; ignoring",
                        instance_id
                    );
                    return;
                }
                let protocol_state = state.deck_widget_state();
                tracing::debug!(
                    "Widget {} led_endless: req={request_id} effect={effect:?} rgb=({r},{g},{b}) period_ms={period_ms} scope={scope:?}",
                    instance_id
                );
                let Some(color) = rgb_from_protocol(&instance_id, r, g, b) else {
                    return;
                };
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::LedEndless {
                        request_id,
                        effect: led_effect_from_protocol(effect),
                        color,
                        period_ms,
                        scope: led_scope_from_protocol(scope),
                    },
                );
            }
            deck_widget_surface_v1::Request::StopLed { request_id } => {
                let protocol_state = state.deck_widget_state();
                tracing::debug!("Widget {} stop_led: req={request_id}", instance_id);
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::StopLed { request_id },
                );
            }
            other => {
                // `deck_widget_surface_v1::Request` is `#[non_exhaustive]`,
                // so the compiler cannot guarantee exhaustiveness. Any
                // variant added to the protocol but not yet handled here
                // is a programming error — log it loudly instead of
                // silently dropping widget traffic.
                tracing::warn!(
                    "Rejecting unknown widget surface request from instance {instance_id}: {other:?}"
                );
            }
        }
    }

    fn destroyed(
        state: &mut D,
        client_id: ClientId,
        resource: &DeckWidgetSurfaceV1,
        data: &WidgetSurfaceUserData,
    ) {
        state.detach_widget_surface(&data.instance_id, &client_id, &resource.id());
    }
}

pub fn create_global<D>(display: &DisplayHandle)
where
    D: GlobalDispatch<DeckWidgetManagerV2, (), D>
        + Dispatch<DeckWidgetManagerV2, WidgetManagerUserData, D>
        + Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D>
        + DeckWidgetHandler
        + 'static,
{
    display.create_global::<D, DeckWidgetManagerV2, ()>(2, ());
}
