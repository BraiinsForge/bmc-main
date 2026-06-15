// Copyright (C) 2026  Braiins Systems s.r.o.

//! Server state and dispatch for the vendored `deck_screen_edge_v1` protocol.

use deck_screen_edge_v1::server::deck_auto_hide_screen_edge_v1::{self, DeckAutoHideScreenEdgeV1};
use deck_screen_edge_v1::server::deck_screen_edge_manager_v1::{
    self, Border, DeckScreenEdgeManagerV1, Error,
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    protocol::wl_surface::WlSurface,
};

use super::state::CompositorState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeFlags {
    pub border: Border,
    pub armed: bool,
    pub revealed: bool,
}

impl EdgeFlags {
    pub fn try_trigger(&mut self, border: Border) -> bool {
        if self.border != border || !self.armed {
            return false;
        }

        self.armed = false;
        self.revealed = true;
        true
    }

    /// Return to the armed-and-hidden resting state. Called when the surface
    /// unmaps (commits a NULL buffer): a crashed or misbehaving overlay that
    /// unmaps without re-arming would otherwise leave `revealed` set, and the
    /// scene-drag suppression it drives (`any_screen_edge_revealed`) would pin
    /// scene navigation off indefinitely.
    pub fn rearm(&mut self) {
        self.armed = true;
        self.revealed = false;
    }
}

#[derive(Debug)]
pub struct ScreenEdgeSession {
    pub resource: DeckAutoHideScreenEdgeV1,
    pub surface: WlSurface,
    pub flags: EdgeFlags,
}

#[derive(Debug, Clone)]
pub struct ScreenEdgeUserData {
    pub surface: WlSurface,
}

impl GlobalDispatch<DeckScreenEdgeManagerV1, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckScreenEdgeManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<DeckScreenEdgeManagerV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckScreenEdgeManagerV1,
        request: deck_screen_edge_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_screen_edge_manager_v1::Request::GetAutoHideScreenEdge {
                id,
                border,
                surface,
            } => {
                let border = match border {
                    WEnum::Value(border) => border,
                    WEnum::Unknown(value) => {
                        resource
                            .post_error(Error::InvalidBorder, format!("invalid border {value}"));
                        return;
                    }
                };

                if !state.surface_has_layer_role(&surface) {
                    resource.post_error(
                        Error::InvalidRole,
                        "surface must have a layer-shell role".to_owned(),
                    );
                    return;
                }

                if state
                    .screen_edge_sessions
                    .iter()
                    .any(|session| session.surface == surface)
                {
                    resource.post_error(
                        Error::AlreadyConstructed,
                        "surface already has a screen edge".to_owned(),
                    );
                    return;
                }

                let edge = data_init.init(
                    id,
                    ScreenEdgeUserData {
                        surface: surface.clone(),
                    },
                );
                state.screen_edge_sessions.push(ScreenEdgeSession {
                    resource: edge,
                    surface,
                    flags: EdgeFlags {
                        border,
                        armed: false,
                        revealed: false,
                    },
                });
            }
            deck_screen_edge_manager_v1::Request::Destroy => {}
            other => {
                tracing::warn!("Rejecting unknown screen edge manager request: {other:?}");
            }
        }
    }
}

impl Dispatch<DeckAutoHideScreenEdgeV1, ScreenEdgeUserData> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckAutoHideScreenEdgeV1,
        request: deck_auto_hide_screen_edge_v1::Request,
        data: &ScreenEdgeUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let Some(session) = state
            .screen_edge_sessions
            .iter_mut()
            .find(|session| session.resource == *resource)
        else {
            tracing::warn!(
                "Ignoring screen edge request for untracked surface {:?}",
                data.surface.id()
            );
            return;
        };

        match request {
            deck_auto_hide_screen_edge_v1::Request::Activate => {
                session.flags.armed = true;
                session.flags.revealed = false;
                resource.hidden();
            }
            deck_auto_hide_screen_edge_v1::Request::Deactivate => {
                session.flags.armed = false;
                session.flags.revealed = true;
                resource.revealed();
                state.mark_full_output_damage();
            }
            deck_auto_hide_screen_edge_v1::Request::Destroy => {
                state
                    .screen_edge_sessions
                    .retain(|session| session.resource != *resource);
            }
            other => {
                tracing::warn!("Rejecting unknown screen edge request: {other:?}");
            }
        }
    }
}

pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckScreenEdgeManagerV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use super::{Border, EdgeFlags};

    #[test]
    fn try_trigger_spends_armed_edge_on_matching_border() {
        let mut flags = EdgeFlags {
            border: Border::Top,
            armed: true,
            revealed: false,
        };

        assert!(flags.try_trigger(Border::Top));
        assert_eq!(
            flags,
            EdgeFlags {
                border: Border::Top,
                armed: false,
                revealed: true,
            },
            "triggering the armed edge must reveal once and spend the arm"
        );
    }

    #[test]
    fn rearm_clears_a_stale_reveal_on_unmap() {
        let mut flags = EdgeFlags {
            border: Border::Top,
            armed: false,
            revealed: true,
        };

        flags.rearm();
        assert_eq!(
            flags,
            EdgeFlags {
                border: Border::Top,
                armed: true,
                revealed: false,
            },
            "an unmap must clear the reveal so it stops suppressing scene navigation"
        );
    }

    #[test]
    fn try_trigger_ignores_unarmed_edge() {
        let mut flags = EdgeFlags {
            border: Border::Top,
            armed: false,
            revealed: false,
        };

        assert!(!flags.try_trigger(Border::Top));
        assert_eq!(
            flags,
            EdgeFlags {
                border: Border::Top,
                armed: false,
                revealed: false,
            },
            "unarmed edges must remain hidden until explicitly armed"
        );
    }
}
