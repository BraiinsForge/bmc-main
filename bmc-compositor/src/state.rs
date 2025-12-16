// Copyright (C) 2025  Braiins Systems s.r.o.

//! Compositor state management

use smithay::{
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
    input::{SeatHandler, SeatState},
    reexports::wayland_server::{
        Client, Display, DisplayHandle, Resource,
        backend::{ClientData, ClientId, DisconnectReason},
        protocol::{wl_buffer::WlBuffer, wl_callback::WlCallback, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes, with_states,
        },
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

/// Main compositor state
#[expect(
    missing_debug_implementations,
    reason = "contains non-Debug smithay types"
)]
pub struct Compositor {
    /// Wayland display handle
    pub display_handle: DisplayHandle,

    /// Compositor protocol state
    pub compositor_state: CompositorState,

    /// Shared memory state for client buffers
    pub shm_state: ShmState,

    /// XDG shell state for window management
    pub xdg_shell_state: XdgShellState,

    /// Seat state for input handling
    pub seat_state: SeatState<Self>,

    /// Data device state for copy/paste
    pub data_device_state: DataDeviceState,

    /// Display dimensions
    pub width: u32,
    pub height: u32,

    /// Track client surfaces for compositing
    pub surfaces: Vec<WlSurface>,

    /// Current active buffer (from the most recent commit)
    pub current_buffer: Option<WlBuffer>,

    /// Pending frame callbacks to fire after rendering
    pub pending_frame_callbacks: Vec<WlCallback>,
}

impl Compositor {
    /// Create a new compositor state
    #[must_use]
    pub fn new(display: &Display<Self>, width: u32, height: u32) -> Self {
        let display_handle = display.handle();

        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);

        // Create a seat for input
        seat_state.new_wl_seat(&display_handle, "seat0");

        Self {
            display_handle,
            compositor_state,
            shm_state,
            xdg_shell_state,
            seat_state,
            data_device_state,
            width,
            height,
            surfaces: Vec::new(),
            current_buffer: None,
            pending_frame_callbacks: Vec::new(),
        }
    }

    /// Send frame callbacks to clients (call after rendering a frame)
    pub fn send_frame_callbacks(&mut self, time: u32) {
        for callback in self.pending_frame_callbacks.drain(..) {
            callback.done(time);
        }
    }
}

// Compositor handler - called when surfaces are created/committed
impl CompositorHandler for Compositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
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
        // Track the surface for rendering
        if !self.surfaces.iter().any(|s| s.id() == surface.id()) {
            self.surfaces.push(surface.clone());
        }

        // Capture the buffer and frame callbacks from this commit
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

            // Collect frame callbacks - client waits for these before rendering next frame
            self.pending_frame_callbacks
                .extend(attributes.frame_callbacks.iter().cloned());
        });
    }
}

// SHM handler - for shared memory buffers
impl ShmHandler for Compositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

// Buffer handler - required by ShmState
impl BufferHandler for Compositor {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {
        // Nothing to do - we don't track buffers explicitly
    }
}

// Seat handler - for input devices
impl SeatHandler for Compositor {
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
        // Handle focus changes
    }

    fn cursor_image(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
        // Handle cursor image changes
    }
}

// XDG Shell handler - for window management
impl XdgShellHandler for Compositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        tracing::info!("New toplevel surface created");
        // Configure the surface with our display size
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
        // We don't support popups for widgets
    }

    fn grab(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        // We don't support popup grabs
    }

    fn reposition_request(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _positioner: smithay::wayland::shell::xdg::PositionerState,
        _token: u32,
    ) {
        // We don't support popup repositioning
    }
}

// Selection handler - required by DataDeviceHandler
impl SelectionHandler for Compositor {
    type SelectionUserData = ();
}

// Data device handlers for copy/paste
impl DataDeviceHandler for Compositor {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Compositor {}
impl ServerDndGrabHandler for Compositor {}

/// Per-client state for tracking compositor-related data
#[derive(Debug, Default)]
pub struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientState {
    /// Get a reference to the compositor state
    pub fn compositor_state(&self) -> &CompositorClientState {
        &self.compositor_state
    }
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

// Delegate macros to wire up the protocol handlers
delegate_compositor!(Compositor);
delegate_shm!(Compositor);
delegate_seat!(Compositor);
delegate_xdg_shell!(Compositor);
delegate_data_device!(Compositor);
