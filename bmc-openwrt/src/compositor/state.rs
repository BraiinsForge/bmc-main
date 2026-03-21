// Copyright (C) 2025  Braiins Systems s.r.o.

//! Compositor state management combining Smithay handlers with deck_widget_v1 protocol.

use super::protocol::{
    DeckWidgetHandler, DeckWidgetProtocolState, WidgetManagerUserData, WidgetSurfaceUserData,
};
use super::widget_tracker::WidgetTracker;
use bmc::compositor::InstanceId;
use bmc_widget_protocol::server::{
    deck_widget_manager_v1::DeckWidgetManagerV1, deck_widget_surface_v1::DeckWidgetSurfaceV1,
};
use smithay::{
    backend::allocator::{Buffer, Format, Fourcc, Modifier, dmabuf::Dmabuf},
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_seat, delegate_shm,
    delegate_xdg_shell,
    input::{SeatHandler, SeatState},
    reexports::wayland_server::{
        Client, Display, DisplayHandle, Resource,
        backend::{ClientData, ClientId, DisconnectReason},
        protocol::{wl_buffer::WlBuffer, wl_callback::WlCallback, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler,
            CompositorState as SmithayCompositorState, SurfaceAttributes, with_states,
        },
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        selection::{
            SelectionHandler,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
        },
        shell::xdg::{XdgShellHandler, XdgShellState},
        shm::{ShmHandler, ShmState},
    },
};

#[expect(clippy::struct_field_names)]
pub struct CompositorState {
    display_handle: DisplayHandle,
    pub compositor_state: SmithayCompositorState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    pub xdg_shell_state: XdgShellState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub deck_widget_state: DeckWidgetProtocolState,
    pub width: u32,
    pub height: u32,
    pub surfaces: Vec<WlSurface>,
    pub widget_buffers: Vec<(WlBuffer, InstanceId)>,
    pub pending_frame_callbacks: Vec<WlCallback>,

    /// Widget registration and connection tracking.
    pub widgets: WidgetTracker,
}

impl CompositorState {
    #[must_use]
    pub fn new(display: &Display<Self>, width: u32, height: u32) -> Self {
        let display_handle = display.handle();

        let compositor_state = SmithayCompositorState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);

        let dmabuf_formats = [
            Format {
                code: Fourcc::Xrgb8888,
                modifier: Modifier::Linear,
            },
            Format {
                code: Fourcc::Argb8888,
                modifier: Modifier::Linear,
            },
        ];
        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global = dmabuf_state.create_global::<Self>(&display_handle, dmabuf_formats);

        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);

        seat_state.new_wl_seat(&display_handle, "seat0");

        let deck_widget_state = DeckWidgetProtocolState::new();
        super::protocol::create_global::<Self>(&display_handle);

        Self {
            display_handle,
            compositor_state,
            shm_state,
            dmabuf_state,
            _dmabuf_global: dmabuf_global,
            xdg_shell_state,
            seat_state,
            data_device_state,
            deck_widget_state,
            width,
            height,
            surfaces: Vec::new(),
            widget_buffers: Vec::new(),
            pending_frame_callbacks: Vec::new(),
            widgets: WidgetTracker::new(),
        }
    }

    pub fn send_frame_callbacks(&mut self, time: u32) {
        for callback in self.pending_frame_callbacks.drain(..) {
            callback.done(time);
        }
    }
}

impl DeckWidgetHandler for CompositorState {
    fn deck_widget_state(&mut self) -> &mut DeckWidgetProtocolState {
        &mut self.deck_widget_state
    }
}

impl CompositorHandler for CompositorState {
    fn compositor_state(&mut self) -> &mut SmithayCompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        client
            .get_data::<ClientState>()
            .expect("BUG: client missing ClientState")
            .compositor_state()
    }

    fn commit(&mut self, surface: &WlSurface) {
        tracing::debug!("Surface committed: {:?}", surface.id());
        if !self.surfaces.iter().any(|s| s.id() == surface.id()) {
            self.surfaces.push(surface.clone());
        }

        // First try to match by surface directly (for protocol surface)
        let mut instance_id = self
            .deck_widget_state
            .instance_id_for_surface(surface)
            .cloned();

        // If not found, try to match by PID (for Slint render surfaces)
        if instance_id.is_none() {
            // Get PID from surface's client
            if let Some(client) = surface.client()
                && let Ok(creds) = client.get_credentials(&self.display_handle)
            {
                #[expect(clippy::cast_sign_loss, reason = "PID is always positive")]
                let pid = creds.pid as u32;
                instance_id = self
                    .deck_widget_state
                    .instance_id_for_surface_by_pid(Some(pid))
                    .cloned();
                if instance_id.is_some() {
                    tracing::debug!(
                        "Matched surface {:?} to widget by PID {}",
                        surface.id(),
                        pid
                    );
                }
            }
        }

        with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attributes = guard.current();

            if let Some(assignment) = &attributes.buffer {
                match assignment {
                    BufferAssignment::NewBuffer(buffer) => {
                        if let Some(ref id) = instance_id {
                            self.widget_buffers
                                .retain(|(_, existing_id)| existing_id != id);
                            self.widget_buffers.push((buffer.clone(), id.clone()));
                            tracing::debug!(
                                "Buffer attached for widget {} (total buffers: {})",
                                id,
                                self.widget_buffers.len()
                            );
                        } else {
                            tracing::debug!("Buffer attached to unknown surface (no instance_id)");
                        }
                    }
                    BufferAssignment::Removed => {
                        if let Some(ref id) = instance_id {
                            self.widget_buffers
                                .retain(|(_, existing_id)| existing_id != id);
                            tracing::debug!("Buffer removed for widget {}", id);
                        }
                    }
                }
            }

            self.pending_frame_callbacks
                .extend(attributes.frame_callbacks.iter().cloned());
        });
    }
}

impl ShmHandler for CompositorState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl BufferHandler for CompositorState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl DmabufHandler for CompositorState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        tracing::debug!(
            "DMA-BUF imported: {}x{}, format={:?}",
            dmabuf.width(),
            dmabuf.height(),
            dmabuf.format()
        );
        let _ = notifier.successful::<CompositorState>();
    }
}

impl SeatHandler for CompositorState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _focused: Option<&Self::KeyboardFocus>,
    ) {
    }

    fn cursor_image(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }
}

impl XdgShellHandler for CompositorState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        tracing::info!("New toplevel surface created");
        #[expect(
            clippy::cast_possible_wrap,
            reason = "display dimensions are always small enough"
        )]
        surface.with_pending_state(|state| {
            state.size = Some((self.width as i32, self.height as i32).into());
        });
        surface.send_configure();
    }

    fn new_popup(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _positioner: smithay::wayland::shell::xdg::PositionerState,
    ) {
    }

    fn grab(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
    }

    fn reposition_request(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _positioner: smithay::wayland::shell::xdg::PositionerState,
        _token: u32,
    ) {
    }
}

impl SelectionHandler for CompositorState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for CompositorState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for CompositorState {}
impl ServerDndGrabHandler for CompositorState {}

#[derive(Debug, Default)]
pub struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientState {
    pub fn compositor_state(&self) -> &CompositorClientState {
        &self.compositor_state
    }
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

delegate_compositor!(CompositorState);
delegate_shm!(CompositorState);
delegate_dmabuf!(CompositorState);
delegate_seat!(CompositorState);
delegate_xdg_shell!(CompositorState);
delegate_data_device!(CompositorState);

smithay::reexports::wayland_server::delegate_global_dispatch!(
    CompositorState: [DeckWidgetManagerV1: ()] => DeckWidgetProtocolState
);
smithay::reexports::wayland_server::delegate_dispatch!(
    CompositorState: [DeckWidgetManagerV1: WidgetManagerUserData] => DeckWidgetProtocolState
);
smithay::reexports::wayland_server::delegate_dispatch!(
    CompositorState: [DeckWidgetSurfaceV1: WidgetSurfaceUserData] => DeckWidgetProtocolState
);
