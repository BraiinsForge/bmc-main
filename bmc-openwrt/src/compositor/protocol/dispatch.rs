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

//! Wayland dispatch implementations for deck_widget_v1.

use std::sync::{Arc, Mutex};

use bmc::compositor::{InstanceId, WidgetGeneration};
use bmc_widget_protocol::server::{
    deck_widget_manager_v1::{self, DeckWidgetManagerV1},
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
    pub instance_id: Arc<Mutex<InstanceId>>,
}

pub trait DeckWidgetHandler {
    fn deck_widget_state(&mut self) -> &mut DeckWidgetProtocolState;

    /// Synchronously drop per-widget render state owned outside the
    /// protocol state. Called from the trait's `unregister_widget` /
    /// `clear_pid_for_instance` defaults while the disconnecting pid
    /// is still the current pid, so a same-id re-register cannot find
    /// this cleanup running against its fresh state.
    fn drop_widget_render_state(&mut self, _instance_id: &InstanceId, _pid: Option<u32>) {}

    fn forget_widget_lifecycle(&mut self, _instance_id: &InstanceId) {}

    fn detach_widget_surface(
        &mut self,
        instance_id: &InstanceId,
        client_id: &ClientId,
        protocol_surface_id: &ObjectId,
    ) {
        if let SurfaceDetach::Detached { pid } =
            self.deck_widget_state()
                .detach_surface(instance_id, client_id, protocol_surface_id)
        {
            self.drop_widget_render_state(instance_id, pid);
            self.forget_widget_lifecycle(instance_id);
        }
    }

    /// Unregister legacy PID-based state before another command-loop
    /// registration can reuse the instance id.
    fn unregister_widget(&mut self, instance_id: &InstanceId) {
        let pid = self.deck_widget_state().unregister_widget(instance_id);
        if pid.is_some() {
            self.drop_widget_render_state(instance_id, pid);
        }
    }

    /// Bind-guarded unregister for supervision's abandon path. Needs no
    /// render-state cleanup: the instance is unbound by construction, so
    /// `clear_pid_for_instance` dropped that when the process exited.
    fn unregister_abandoned(
        &mut self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
    ) -> bool {
        self.deck_widget_state()
            .unregister_abandoned(instance_id, generation)
    }

    /// Pid-guarded unregister for the coordinator's child-exit path.
    /// Bundled with render-state cleanup for the same reason
    /// `unregister_widget` is: the two protocol-state unregister
    /// entry points must not diverge on which side owns cleanup.
    fn clear_pid_for_instance(
        &mut self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
        expected_pid: u32,
    ) -> Option<u32> {
        let pid =
            self.deck_widget_state()
                .clear_pid_for_instance(instance_id, generation, expected_pid);
        if pid.is_some() {
            self.drop_widget_render_state(instance_id, pid);
        }
        pid
    }
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
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        let instance_id = data
            .instance_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if instance_id.is_empty() {
            tracing::warn!("Received request on unresolved widget surface; ignoring");
            return;
        }
        match request {
            deck_widget_surface_v1::Request::Destroy => {
                state.detach_widget_surface(&instance_id, &client.id(), &resource.id());
                tracing::info!("Widget surface destroyed for instance: {}", instance_id);
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
}

pub fn create_global<D>(display: &DisplayHandle)
where
    D: GlobalDispatch<DeckWidgetManagerV1, (), D>
        + Dispatch<DeckWidgetManagerV1, WidgetManagerUserData, D>
        + Dispatch<DeckWidgetSurfaceV1, WidgetSurfaceUserData, D>
        + DeckWidgetHandler
        + 'static,
{
    display.create_global::<D, DeckWidgetManagerV1, ()>(2, ());
}

#[cfg(test)]
mod tests {
    use bmc::compositor::{InstanceId, WidgetGeneration};
    use bmc_widget_protocol::{ViewportShape, WidgetInitialConfig};

    use super::super::state::DeckWidgetProtocolState;
    use super::DeckWidgetHandler;

    const GEN: WidgetGeneration = WidgetGeneration(1);

    struct MockHandler {
        state: DeckWidgetProtocolState,
        cleanup_log: Vec<(InstanceId, Option<u32>)>,
    }

    impl DeckWidgetHandler for MockHandler {
        fn deck_widget_state(&mut self) -> &mut DeckWidgetProtocolState {
            &mut self.state
        }

        fn drop_widget_render_state(&mut self, instance_id: &InstanceId, pid: Option<u32>) {
            self.cleanup_log.push((instance_id.clone(), pid));
        }
    }

    fn make_config() -> WidgetInitialConfig {
        WidgetInitialConfig {
            width: 100,
            height: 100,
            viewport_shape: ViewportShape::Rectangular,
            display: bmc_widget_protocol::DisplayInfo::BMC100,
            params: serde_json::Map::new(),
            credentials: serde_json::Map::new(),
            credential_secrets: bmc_widget_protocol::CredentialSecrets::default(),
            token: "test-instance-2x1".to_owned(),
        }
    }

    #[test]
    fn unregister_drops_render_state_before_same_id_register_can_race() {
        let mut handler = MockHandler {
            state: DeckWidgetProtocolState::new(),
            cleanup_log: Vec::new(),
        };
        handler
            .state
            .register_widget("alpha".to_owned(), GEN, make_config());
        handler.state.set_widget_pid(&"alpha".to_owned(), GEN, 100);
        let _ = handler.state.drain_connected();

        handler.unregister_widget(&"alpha".to_owned());

        assert_eq!(
            handler.cleanup_log,
            vec![("alpha".to_owned(), Some(100))],
            "cleanup must run synchronously while the old pid is still current"
        );

        handler
            .state
            .register_widget("alpha".to_owned(), GEN, make_config());
        handler.state.set_widget_pid(&"alpha".to_owned(), GEN, 200);

        assert_eq!(
            handler.cleanup_log.len(),
            1,
            "no deferred queue should fire a second cleanup against the fresh pid=200 state"
        );

        let drained = handler.state.drain_disconnected();
        assert_eq!(drained, vec!["alpha".to_owned()]);
    }

    // A stale `clear_pid_for_instance` (pid does not match current)
    // must not invoke render-state cleanup — the widget still belongs
    // to a different (newer) process and its render state must
    // survive.
    #[test]
    fn stale_clear_pid_does_not_invoke_render_state_cleanup() {
        let mut handler = MockHandler {
            state: DeckWidgetProtocolState::new(),
            cleanup_log: Vec::new(),
        };
        handler
            .state
            .register_widget("alpha".to_owned(), GEN, make_config());
        handler.state.set_widget_pid(&"alpha".to_owned(), GEN, 200);
        let _ = handler.state.drain_connected();

        handler.clear_pid_for_instance(&"alpha".to_owned(), GEN, 100);

        assert!(
            handler.cleanup_log.is_empty(),
            "stale clear (pid mismatch) must not trigger any cleanup"
        );
        assert!(handler.state.drain_disconnected().is_empty());
    }
}
