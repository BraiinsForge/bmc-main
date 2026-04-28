// Copyright (C) 2025  Braiins Systems s.r.o.

//! Wayland dispatch implementations for deck_widget_v1.

use std::sync::{Arc, Mutex};

use bmc::compositor::InstanceId;
use bmc_widget_protocol::server::{
    deck_widget_manager_v1::{self, DeckWidgetManagerV1},
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::conversions::led_effect_from_protocol;
use super::state::DeckWidgetProtocolState;

/// Wayland's `uint` is a `u32`, but our RGB values are byte-sized.
/// Clamp to fit so a malicious or buggy widget can't supply 0xFFFFFFFF
/// and overflow downstream LED drivers.
fn clamp_u8(v: u32) -> u8 {
    u8::try_from(v).unwrap_or(u8::MAX)
}

#[derive(Debug)]
pub struct WidgetManagerUserData;

#[derive(Debug, Clone)]
pub struct WidgetSurfaceUserData {
    pub instance_id: Arc<Mutex<InstanceId>>,
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
        match request {
            deck_widget_manager_v1::Request::GetWidgetSurface { id, surface } => {
                // `SO_PEERCRED` returns the client's pid as `i32`. Reject
                // failures outright (no credentials → can't safely buffer)
                // and reject non-positive pids (the kernel's `-1` sentinel
                // and any arithmetic edge case): accepting them would let
                // callers with unknown credentials grow
                // `pending_connections` without bound.
                let pid = match client.get_credentials(dhandle).ok().map(|c| c.pid) {
                    Some(pid) if pid > 0 => {
                        #[expect(clippy::cast_sign_loss, reason = "guarded by pid > 0 check above")]
                        let pid = pid as u32;
                        pid
                    }
                    _ => {
                        tracing::warn!(
                            "GetWidgetSurface: rejecting connection without readable peer credentials"
                        );
                        return;
                    }
                };

                let protocol_state = state.deck_widget_state();

                let instance_id_lock = Arc::new(Mutex::new(String::new()));
                let user_data = WidgetSurfaceUserData {
                    instance_id: Arc::clone(&instance_id_lock),
                };
                let widget_surface = data_init.init(id, user_data);

                if let Some(instance_id) = protocol_state
                    .instance_id_for_surface_by_pid(Some(pid))
                    .cloned()
                {
                    tracing::info!(
                        "GetWidgetSurface: instance={} surface={:?} pid={}",
                        instance_id,
                        surface.id(),
                        pid
                    );
                    instance_id_lock
                        .lock()
                        .expect("BUG: instance_id lock poisoned")
                        .clone_from(&instance_id);
                    protocol_state.attach_surface(&instance_id, surface, widget_surface.clone());
                    protocol_state.emit_initial_state(&instance_id, &widget_surface);
                } else {
                    // The widget connected before set_widget_pid arrived.
                    // Buffer it — set_widget_pid will resolve and complete.
                    tracing::info!(
                        "GetWidgetSurface: pid={} not yet registered, buffering",
                        pid
                    );
                    protocol_state.buffer_pending_connection(
                        pid,
                        surface,
                        widget_surface,
                        instance_id_lock,
                    );
                }
            }
            deck_widget_manager_v1::Request::Destroy => {
                // Destructor request; wayland-server handles teardown.
            }
            other => {
                tracing::warn!("Rejecting unknown widget manager request: {other:?}");
            }
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
        let instance_id = data
            .instance_id
            .lock()
            .expect("BUG: instance_id lock poisoned")
            .clone();
        if instance_id.is_empty() {
            tracing::warn!("Received request on unresolved widget surface; ignoring");
            return;
        }
        let protocol_state = state.deck_widget_state();
        match request {
            deck_widget_surface_v1::Request::Destroy => {
                protocol_state.unregister_widget(&instance_id);
                tracing::info!("Widget surface destroyed for instance: {}", instance_id);
            }
            deck_widget_surface_v1::Request::PlaySound { sound } => {
                tracing::debug!("Widget {} play_sound: {sound}", instance_id);
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::PlaySound { sound },
                );
            }
            deck_widget_surface_v1::Request::StopSound => {
                tracing::debug!("Widget {} stop_sound", instance_id);
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::StopSound {},
                );
            }
            deck_widget_surface_v1::Request::LedTemporary {
                effect,
                r,
                g,
                b,
                duration_ms,
            } => {
                let Ok(effect) = effect.into_result() else {
                    tracing::warn!("Widget {} led_temporary: unknown effect", instance_id);
                    return;
                };
                tracing::debug!(
                    "Widget {} led_temporary: effect={effect:?} rgb=({r},{g},{b}) duration_ms={duration_ms}",
                    instance_id
                );
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::LedTemporary {
                        effect: led_effect_from_protocol(effect),
                        color: bmc_widget_protocol::RgbColor {
                            r: clamp_u8(r),
                            g: clamp_u8(g),
                            b: clamp_u8(b),
                        },
                        duration_ms,
                    },
                );
            }
            deck_widget_surface_v1::Request::LedEndless { effect, r, g, b } => {
                let Ok(effect) = effect.into_result() else {
                    tracing::warn!("Widget {} led_endless: unknown effect", instance_id);
                    return;
                };
                tracing::debug!(
                    "Widget {} led_endless: effect={effect:?} rgb=({r},{g},{b})",
                    instance_id
                );
                protocol_state.add_action(
                    instance_id,
                    bmc_widget_protocol::ActionPayload::LedEndless {
                        effect: led_effect_from_protocol(effect),
                        color: bmc_widget_protocol::RgbColor {
                            r: clamp_u8(r),
                            g: clamp_u8(g),
                            b: clamp_u8(b),
                        },
                    },
                );
            }
            deck_widget_surface_v1::Request::StopLed => {
                tracing::debug!("Widget {} stop_led", instance_id);
                protocol_state
                    .add_action(instance_id, bmc_widget_protocol::ActionPayload::StopLed {});
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
