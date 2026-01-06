// Copyright (C) 2025  Braiins Systems s.r.o.

//! Compositor state management combining Smithay handlers with deck_widget_v1 protocol.

use std::collections::HashMap;

use super::protocol::{
    DeckWidgetHandler, DeckWidgetProtocolState, WidgetManagerUserData, WidgetSurfaceUserData,
};
use bmc::compositor::{InstanceId, Position, SceneLayout, Size};
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

/// Registration data for a widget pending connection.
#[derive(Debug, Clone)]
pub struct WidgetRegistration {
    pub instance_id: InstanceId,
    pub position: Position,
    pub size: Size,
}

/// Tracks a connected widget surface.
#[derive(Debug)]
pub struct ConnectedWidget {
    pub instance_id: InstanceId,
    pub position: Position,
    pub size: Size,
    pub surface: WlSurface,
}

pub struct CompositorState {
    pub display_handle: DisplayHandle,
    pub compositor_state: SmithayCompositorState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: DmabufGlobal,
    pub xdg_shell_state: XdgShellState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub deck_widget_state: DeckWidgetProtocolState,
    pub width: u32,
    pub height: u32,
    pub surfaces: Vec<WlSurface>,
    pub current_buffer: Option<WlBuffer>,
    pub pending_frame_callbacks: Vec<WlCallback>,

    /// Widgets registered by coordinator, waiting for client connection.
    /// Key is instance_id for deck_widget clients.
    pub pending_widgets: HashMap<InstanceId, WidgetRegistration>,

    /// PID to instance_id mapping for xdg_toplevel clients.
    /// When a third-party client connects, we look up its PID here.
    pub pid_to_instance: HashMap<u32, InstanceId>,

    /// Connected widgets with their surfaces (both deck_widget and xdg_toplevel).
    pub connected_widgets: HashMap<InstanceId, ConnectedWidget>,

    /// Current active scene layout determining which widgets are visible and where.
    pub active_scene: SceneLayout,
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
            dmabuf_global,
            xdg_shell_state,
            seat_state,
            data_device_state,
            deck_widget_state,
            width,
            height,
            surfaces: Vec::new(),
            current_buffer: None,
            pending_frame_callbacks: Vec::new(),
            pending_widgets: HashMap::new(),
            pid_to_instance: HashMap::new(),
            connected_widgets: HashMap::new(),
            active_scene: SceneLayout::default(),
        }
    }

    pub fn send_frame_callbacks(&mut self, time: u32) {
        for callback in self.pending_frame_callbacks.drain(..) {
            callback.done(time);
        }
    }

    /// Register a widget before its process connects.
    /// For deck_widget clients, pid is None (they identify via protocol).
    /// For xdg_toplevel clients, pid is required for matching.
    pub fn register_widget(
        &mut self,
        instance_id: InstanceId,
        position: Position,
        size: Size,
        pid: Option<u32>,
    ) {
        let registration = WidgetRegistration {
            instance_id: instance_id.clone(),
            position,
            size,
        };

        self.pending_widgets.insert(instance_id.clone(), registration);

        if let Some(pid) = pid {
            self.pid_to_instance.insert(pid, instance_id);
        }
    }

    /// Unregister a widget when its process stops.
    pub fn unregister_widget(&mut self, instance_id: &InstanceId) {
        self.pending_widgets.remove(instance_id);
        self.connected_widgets.remove(instance_id);

        // Remove from PID mapping if present
        self.pid_to_instance.retain(|_, id| id != instance_id);
    }

    /// Look up instance_id by PID (for xdg_toplevel clients).
    #[must_use]
    pub fn instance_id_for_pid(&self, pid: u32) -> Option<&InstanceId> {
        self.pid_to_instance.get(&pid)
    }

    /// Get registration data for a pending widget.
    #[must_use]
    pub fn get_pending_widget(&self, instance_id: &InstanceId) -> Option<&WidgetRegistration> {
        self.pending_widgets.get(instance_id)
    }

    /// Move widget from pending to connected state.
    pub fn connect_widget(&mut self, instance_id: &InstanceId, surface: WlSurface) -> bool {
        if let Some(registration) = self.pending_widgets.remove(instance_id) {
            self.connected_widgets.insert(
                instance_id.clone(),
                ConnectedWidget {
                    instance_id: registration.instance_id,
                    position: registration.position,
                    size: registration.size,
                    surface,
                },
            );
            true
        } else {
            false
        }
    }

    /// Check if a widget is visible in the current scene.
    #[must_use]
    pub fn is_widget_visible(&self, instance_id: &InstanceId) -> bool {
        self.active_scene
            .widgets
            .iter()
            .any(|w| &w.instance_id == instance_id && w.visible)
    }

    /// Get visible widgets for rendering.
    #[must_use]
    pub fn visible_widgets(&self) -> Vec<&ConnectedWidget> {
        self.connected_widgets
            .values()
            .filter(|w| self.is_widget_visible(&w.instance_id))
            .collect()
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

        with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attributes = guard.current();

            if let Some(assignment) = &attributes.buffer {
                match assignment {
                    BufferAssignment::NewBuffer(buffer) => {
                        tracing::debug!("New buffer attached to surface");
                        self.current_buffer = Some(buffer.clone());
                    }
                    BufferAssignment::Removed => {
                        tracing::debug!("Buffer removed from surface");
                        self.current_buffer = None;
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
