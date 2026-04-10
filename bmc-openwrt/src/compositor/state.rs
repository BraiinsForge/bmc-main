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
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_image_capture_source,
    delegate_image_copy_capture, delegate_output, delegate_output_capture_source, delegate_seat,
    delegate_shm, delegate_xdg_shell,
    input::{SeatHandler, SeatState},
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::{
        Client, Display, DisplayHandle, Resource,
        backend::{ClientData, ClientId, DisconnectReason, ObjectId},
        protocol::{wl_buffer::WlBuffer, wl_callback::WlCallback, wl_shm, wl_surface::WlSurface},
    },
    utils::Size,
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler,
            CompositorState as SmithayCompositorState, SurfaceAttributes, with_states,
        },
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
            OutputCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, Frame, ImageCopyCaptureHandler, ImageCopyCaptureState, Session,
            SessionRef,
        },
        output::OutputHandler,
        selection::{
            SelectionHandler,
            data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler},
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
    pub output_capture_source_state: OutputCaptureSourceState,
    pub image_copy_capture_state: ImageCopyCaptureState,
    pub capture_sessions: Vec<Session>,
    /// Logical display size (after rotation, used for widget layout).
    pub width: u32,
    pub height: u32,
    /// Physical framebuffer size (before rotation, used for capture).
    pub physical_width: u32,
    pub physical_height: u32,
    pub widget_buffers: Vec<(WlBuffer, InstanceId)>,
    pub pending_frame_callbacks: Vec<WlCallback>,

    /// Widget registration and connection tracking.
    pub widgets: WidgetTracker,

    /// Buffer IDs that have been destroyed and need texture cache invalidation.
    pub invalidated_buffers: Vec<ObjectId>,

    /// Buffer IDs that were newly committed and need texture reimport.
    /// Populated in the commit handler, drained by the renderer.
    pub dirty_buffers: Vec<ObjectId>,

    /// Capture frames waiting to be fulfilled after the next render pass.
    pub pending_capture_frames: Vec<Frame>,

    /// Whether image-copy capture is currently usable on this renderer.
    /// Disabled permanently after a fatal readback failure so capture clients
    /// fail cleanly instead of poisoning normal rendering.
    pub capture_enabled: bool,

    /// Set when widget content or scene layout changes and a new frame must be rendered.
    pub needs_redraw: bool,
}

impl CompositorState {
    #[must_use]
    pub fn new(
        display: &Display<Self>,
        width: u32,
        height: u32,
        physical_width: u32,
        physical_height: u32,
        refresh_mhz: i32,
    ) -> Self {
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

        // Advertise the display as a wl_output so capture clients can reference it.
        let output = Output::new(
            "DRM-1".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Braiins".into(),
                model: "Deck".into(),
                serial_number: String::new(),
            },
        );
        #[expect(clippy::cast_possible_wrap, reason = "display dimensions fit in i32")]
        let output_mode = OutputMode {
            size: (physical_width as i32, physical_height as i32).into(),
            refresh: refresh_mhz,
        };
        output.set_preferred(output_mode);
        output.change_current_state(Some(output_mode), None, None, None);
        output.create_global::<Self>(&display_handle);

        let output_capture_source_state = OutputCaptureSourceState::new::<Self>(&display_handle);
        let image_copy_capture_state = ImageCopyCaptureState::new::<Self>(&display_handle);

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
            output_capture_source_state,
            image_copy_capture_state,
            capture_sessions: Vec::new(),
            width,
            height,
            physical_width,
            physical_height,
            widget_buffers: Vec::new(),
            pending_frame_callbacks: Vec::new(),
            widgets: WidgetTracker::new(),
            invalidated_buffers: Vec::new(),
            dirty_buffers: Vec::new(),
            pending_capture_frames: Vec::new(),
            capture_enabled: true,
            needs_redraw: true,
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
        tracing::trace!("Surface committed: {:?}", surface.id());

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
                    tracing::trace!(
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

            if let Some(assignment) = attributes.buffer.take() {
                match assignment {
                    BufferAssignment::NewBuffer(buffer) => {
                        self.needs_redraw = true;
                        if let Some(id) = instance_id.take() {
                            // Release previous buffer so the client can reuse or
                            // destroy it.  Without this the client allocates a new
                            // buffer every frame and old textures leak.
                            for (old_buf, _) in
                                self.widget_buffers.iter().filter(|(_, eid)| eid == &id)
                            {
                                old_buf.release();
                            }
                            self.widget_buffers
                                .retain(|(_, existing_id)| existing_id != &id);
                            tracing::trace!(
                                "Buffer attached for widget {} (total buffers: {})",
                                id,
                                self.widget_buffers.len() + 1
                            );
                            self.dirty_buffers.push(buffer.id());
                            self.widget_buffers.push((buffer.clone(), id));
                        } else {
                            tracing::debug!("Buffer attached to unknown surface (no instance_id)");
                        }
                    }
                    BufferAssignment::Removed => {
                        if let Some(ref id) = instance_id {
                            for (old_buf, _) in
                                self.widget_buffers.iter().filter(|(_, eid)| eid == id)
                            {
                                old_buf.release();
                            }
                            self.widget_buffers
                                .retain(|(_, existing_id)| existing_id != id);
                            tracing::debug!("Buffer removed for widget {}", id);
                        }
                    }
                }
            }

            // Any commit with frame callbacks indicates the client rendered
            // and expects display feedback — trigger a redraw. This covers
            // Slint widgets that render to the same buffer without re-attaching.
            if !attributes.frame_callbacks.is_empty() {
                self.needs_redraw = true;
            }

            // Drain frame_callbacks and damage to prevent unbounded accumulation.
            // Smithay's merge_into() uses extend() on these fields, so they grow
            // indefinitely if not cleared after processing.
            self.pending_frame_callbacks
                .append(&mut attributes.frame_callbacks);
            attributes.damage.clear();
        });
    }
}

impl ShmHandler for CompositorState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl BufferHandler for CompositorState {
    fn buffer_destroyed(&mut self, buffer: &WlBuffer) {
        // Track destroyed buffer for texture cache invalidation
        self.invalidated_buffers.push(buffer.id());
    }
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

// ---- Image capture protocol handlers ----

impl OutputHandler for CompositorState {
    fn output_bound(
        &mut self,
        _output: Output,
        _wl_output: smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
    ) {
    }
}

impl ImageCaptureSourceHandler for CompositorState {}

impl OutputCaptureSourceHandler for CompositorState {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for CompositorState {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    #[expect(clippy::cast_possible_wrap, reason = "display dimensions fit in i32")]
    fn capture_constraints(&mut self, _source: &ImageCaptureSource) -> Option<BufferConstraints> {
        if !self.capture_enabled {
            return None;
        }

        Some(BufferConstraints {
            size: Size::from((self.physical_width as i32, self.physical_height as i32)),
            shm: vec![wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888],
            dma: None,
        })
    }

    fn new_session(&mut self, session: Session) {
        if !self.capture_enabled {
            session.stop();
            return;
        }

        #[expect(clippy::cast_possible_wrap, reason = "display dimensions fit in i32")]
        let constraints = BufferConstraints {
            size: Size::from((self.physical_width as i32, self.physical_height as i32)),
            shm: vec![wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888],
            dma: None,
        };
        session.update_constraints(constraints);
        self.capture_sessions.push(session);
    }

    fn frame(&mut self, _session: &SessionRef, frame: Frame) {
        if !self.capture_enabled {
            frame.fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped);
            return;
        }

        // Frames are collected here and fulfilled from the last rendered content.
        // Do NOT set needs_redraw — capture is passive, it reads whatever was
        // last rendered without triggering additional render passes.
        self.pending_capture_frames.push(frame);
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.capture_sessions.retain(|s| s.as_ref() != session);
    }
}

impl SelectionHandler for CompositorState {
    type SelectionUserData = ();
}

impl CompositorState {
    pub fn disable_capture(&mut self) {
        if !self.capture_enabled {
            return;
        }

        self.capture_enabled = false;

        for frame in self.pending_capture_frames.drain(..) {
            frame.fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped);
        }

        self.capture_sessions.clear();
    }
}

impl DataDeviceHandler for CompositorState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl WaylandDndGrabHandler for CompositorState {}

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

delegate_compositor!(self::CompositorState);
delegate_shm!(self::CompositorState);
delegate_dmabuf!(self::CompositorState);
delegate_seat!(self::CompositorState);
delegate_xdg_shell!(self::CompositorState);
delegate_data_device!(self::CompositorState);
delegate_output!(self::CompositorState);
delegate_image_capture_source!(self::CompositorState);
delegate_output_capture_source!(self::CompositorState);
delegate_image_copy_capture!(self::CompositorState);

smithay::reexports::wayland_server::delegate_global_dispatch!(
    CompositorState: [DeckWidgetManagerV1: ()] => DeckWidgetProtocolState
);
smithay::reexports::wayland_server::delegate_dispatch!(
    CompositorState: [DeckWidgetManagerV1: WidgetManagerUserData] => DeckWidgetProtocolState
);
smithay::reexports::wayland_server::delegate_dispatch!(
    CompositorState: [DeckWidgetSurfaceV1: WidgetSurfaceUserData] => DeckWidgetProtocolState
);
