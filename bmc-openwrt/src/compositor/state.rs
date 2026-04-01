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
    pub pending_frame_callbacks: Vec<PendingFrameCallback>,

    /// Widget registration and connection tracking.
    pub widgets: WidgetTracker,

    /// Per-widget frame generations used to correlate frame callbacks with
    /// the content that was actually presented.
    widget_frame_clocks: std::collections::HashMap<InstanceId, WidgetFrameClockState>,

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

    output_damage: OutputDamageTracker,

    /// Set when widget content or scene layout changes and a new frame must be rendered.
    pub needs_redraw: bool,
}

#[derive(Debug)]
pub struct PendingFrameCallback {
    pub callback: WlCallback,
    pub instance_id: Option<InstanceId>,
    pub client_pid: Option<u32>,
    pub generation: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct WidgetFrameClockState {
    latest_generation: u64,
    last_presented_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputDamage {
    Full,
    Widgets(std::collections::HashSet<InstanceId>),
}

#[derive(Debug, Default)]
struct OutputDamageTracker {
    full_damage: bool,
    widgets: std::collections::HashSet<InstanceId>,
}

impl OutputDamageTracker {
    fn mark_full(&mut self) {
        self.full_damage = true;
        self.widgets.clear();
    }

    fn mark_widget(&mut self, instance_id: &InstanceId) {
        if !self.full_damage {
            self.widgets.insert(instance_id.clone());
        }
    }

    fn snapshot(&self) -> OutputDamage {
        if self.full_damage {
            OutputDamage::Full
        } else {
            OutputDamage::Widgets(self.widgets.clone())
        }
    }

    fn clear(&mut self) {
        self.full_damage = false;
        self.widgets.clear();
    }
}

fn should_complete_frame_callback(
    instance_id: Option<&InstanceId>,
    generation: u64,
    eligible_generations: &std::collections::HashMap<InstanceId, u64>,
) -> bool {
    instance_id.is_some_and(|instance_id| {
        eligible_generations
            .get(instance_id)
            .is_some_and(|eligible_generation| generation <= *eligible_generation)
    })
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
            widget_frame_clocks: std::collections::HashMap::new(),
            invalidated_buffers: Vec::new(),
            dirty_buffers: Vec::new(),
            pending_capture_frames: Vec::new(),
            capture_enabled: true,
            output_damage: OutputDamageTracker {
                full_damage: true,
                widgets: std::collections::HashSet::new(),
            },
            needs_redraw: true,
        }
    }

    fn advance_widget_frame_generation(&mut self, instance_id: &InstanceId) -> u64 {
        let state = self
            .widget_frame_clocks
            .entry(instance_id.clone())
            .or_default();
        state.latest_generation = state.latest_generation.wrapping_add(1).max(1);
        state.latest_generation
    }

    fn queue_frame_callbacks(
        &mut self,
        callbacks: &mut Vec<WlCallback>,
        instance_id: Option<&InstanceId>,
        client_pid: Option<u32>,
        generation: Option<u64>,
    ) {
        let pending_instance_id = instance_id.cloned();
        let pending_generation = generation.unwrap_or(0);

        self.pending_frame_callbacks
            .extend(callbacks.drain(..).map(|callback| PendingFrameCallback {
                callback,
                instance_id: pending_instance_id.clone(),
                client_pid,
                generation: pending_generation,
            }));
    }

    fn surface_client_pid(&self, surface: &WlSurface) -> Option<u32> {
        let client = surface.client()?;
        let credentials = client.get_credentials(&self.display_handle).ok()?;
        #[expect(clippy::cast_sign_loss, reason = "PID is always positive")]
        Some(credentials.pid as u32)
    }

    fn resolve_pending_callback_instance_id(
        &self,
        pending: &PendingFrameCallback,
    ) -> Option<InstanceId> {
        pending.instance_id.clone().or_else(|| {
            self.deck_widget_state
                .instance_id_for_surface_by_pid(pending.client_pid)
                .cloned()
        })
    }

    fn widget_has_buffer(&self, instance_id: &InstanceId) -> bool {
        self.widget_buffers
            .iter()
            .any(|(_buffer, existing_id)| existing_id == instance_id)
    }

    fn eligible_callback_generations(&mut self) -> std::collections::HashMap<InstanceId, u64> {
        let visible_widgets: std::collections::HashSet<_> = self
            .widgets
            .active_scene()
            .widgets
            .iter()
            .filter(|widget| widget.visible)
            .map(|widget| widget.instance_id.clone())
            .collect();
        let mut eligible = std::collections::HashMap::new();

        for (_buffer, instance_id) in &self.widget_buffers {
            if !visible_widgets.contains(instance_id) {
                continue;
            }

            let Some(state) = self.widget_frame_clocks.get_mut(instance_id) else {
                continue;
            };

            if state.latest_generation > state.last_presented_generation {
                state.last_presented_generation = state.latest_generation;
                eligible.insert(instance_id.clone(), state.latest_generation);
            }
        }

        for instance_id in visible_widgets {
            let Some(state) = self.widget_frame_clocks.get(&instance_id) else {
                continue;
            };

            // Bootstrap visible widgets that still wait for their first frame
            // callback and have not attached a buffer yet.
            if state.last_presented_generation == 0 && !self.widget_has_buffer(&instance_id) {
                eligible.insert(instance_id, state.latest_generation);
            }
        }

        eligible
    }

    pub fn send_frame_callbacks_for_presented_widgets(&mut self, time: u32) {
        let eligible_generations = self.eligible_callback_generations();
        let pending_callbacks = std::mem::take(&mut self.pending_frame_callbacks);
        let mut deferred = Vec::with_capacity(pending_callbacks.len());

        for pending in pending_callbacks {
            let resolved_instance_id = self.resolve_pending_callback_instance_id(&pending);
            if should_complete_frame_callback(
                resolved_instance_id.as_ref(),
                pending.generation,
                &eligible_generations,
            ) {
                pending.callback.done(time);
            } else {
                deferred.push(pending);
            }
        }

        self.pending_frame_callbacks = deferred;
    }

    pub fn drop_widget_callback_state(
        &mut self,
        instance_id: &InstanceId,
        client_pid: Option<u32>,
    ) {
        self.widget_frame_clocks.remove(instance_id);
        self.pending_frame_callbacks.retain(|pending| {
            if pending.instance_id.as_ref() == Some(instance_id) {
                return false;
            }

            if pending.instance_id.is_none()
                && client_pid.is_some()
                && pending.client_pid == client_pid
            {
                return false;
            }

            true
        });
    }

    pub fn mark_full_output_damage(&mut self) {
        self.output_damage.mark_full();
    }

    pub fn mark_widget_output_damage(&mut self, instance_id: &InstanceId) {
        self.output_damage.mark_widget(instance_id);
    }

    pub fn current_output_damage(&self) -> OutputDamage {
        self.output_damage.snapshot()
    }

    pub fn clear_output_damage(&mut self) {
        self.output_damage.clear();
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
        let surface_pid = self.surface_client_pid(surface);

        // First try to match by surface directly (for protocol surface)
        let mut instance_id = self
            .deck_widget_state
            .instance_id_for_surface(surface)
            .cloned();

        // If not found, try to match by PID (for Slint render surfaces)
        if instance_id.is_none()
            && let Some(pid) = surface_pid
        {
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

        with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attributes = guard.current();
            let had_buffer_assignment = attributes.buffer.is_some();
            let had_frame_callbacks = !attributes.frame_callbacks.is_empty();

            let callback_generation = instance_id.as_ref().and_then(|id| {
                if had_buffer_assignment || had_frame_callbacks {
                    Some(self.advance_widget_frame_generation(id))
                } else {
                    None
                }
            });

            self.queue_frame_callbacks(
                &mut attributes.frame_callbacks,
                instance_id.as_ref(),
                surface_pid,
                callback_generation,
            );

            if let Some(assignment) = attributes.buffer.take() {
                match assignment {
                    BufferAssignment::NewBuffer(buffer) => {
                        self.needs_redraw = true;
                        if let Some(id) = instance_id.as_ref() {
                            self.mark_widget_output_damage(id);
                            // Release previous buffer so the client can reuse or
                            // destroy it.  Without this the client allocates a new
                            // buffer every frame and old textures leak.
                            for (old_buf, _) in
                                self.widget_buffers.iter().filter(|(_, eid)| eid == id)
                            {
                                old_buf.release();
                            }
                            self.widget_buffers
                                .retain(|(_, existing_id)| existing_id != id);
                            tracing::trace!(
                                "Buffer attached for widget {} (total buffers: {})",
                                id,
                                self.widget_buffers.len() + 1
                            );
                            self.dirty_buffers.push(buffer.id());
                            self.widget_buffers.push((buffer.clone(), id.clone()));
                        } else {
                            self.mark_full_output_damage();
                            tracing::debug!("Buffer attached to unknown surface (no instance_id)");
                        }
                    }
                    BufferAssignment::Removed => {
                        if let Some(ref id) = instance_id {
                            self.mark_widget_output_damage(id);
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
            if had_frame_callbacks {
                self.needs_redraw = true;
                if let Some(id) = instance_id.as_ref() {
                    self.mark_widget_output_damage(id);
                } else {
                    self.mark_full_output_damage();
                }
            }

            // Drain frame_callbacks and damage to prevent unbounded accumulation.
            // Smithay's merge_into() uses extend() on these fields, so they grow
            // indefinitely if not cleared after processing.
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

#[cfg(test)]
mod tests {
    use super::{OutputDamage, OutputDamageTracker, should_complete_frame_callback};
    use std::collections::HashMap;

    #[test]
    fn unknown_surface_callback_stays_deferred_until_it_resolves() {
        assert!(!should_complete_frame_callback(None, 0, &HashMap::new()));
    }

    #[test]
    fn presented_generation_completes_equal_or_older_callback() {
        let mut presented = HashMap::new();
        presented.insert(String::from("clock-left"), 3);

        assert!(should_complete_frame_callback(
            Some(&String::from("clock-left")),
            3,
            &presented,
        ));
        assert!(should_complete_frame_callback(
            Some(&String::from("clock-left")),
            2,
            &presented,
        ));
    }

    #[test]
    fn newer_or_unpresented_widget_callback_is_deferred() {
        let mut eligible_generations = HashMap::new();
        eligible_generations.insert(String::from("clock-left"), 2);

        assert!(!should_complete_frame_callback(
            Some(&String::from("clock-left")),
            3,
            &eligible_generations,
        ));
        assert!(!should_complete_frame_callback(
            Some(&String::from("clock-right")),
            1,
            &eligible_generations,
        ));
    }

    #[test]
    fn bootstrap_generation_allows_initial_widget_callback() {
        let mut eligible_generations = HashMap::new();
        eligible_generations.insert(String::from("clock-left"), 1);

        assert!(should_complete_frame_callback(
            Some(&String::from("clock-left")),
            1,
            &eligible_generations,
        ));
    }

    #[test]
    fn full_damage_overrides_widget_damage() {
        let mut tracker = OutputDamageTracker::default();
        tracker.mark_widget(&String::from("clock-left"));
        tracker.mark_full();

        assert_eq!(tracker.snapshot(), OutputDamage::Full);
    }

    #[test]
    fn widget_damage_accumulates_instances() {
        let mut tracker = OutputDamageTracker::default();
        tracker.mark_widget(&String::from("clock-left"));
        tracker.mark_widget(&String::from("clock-right"));

        let OutputDamage::Widgets(widgets) = tracker.snapshot() else {
            panic!("BUG: tracker should keep partial widget damage");
        };
        assert_eq!(widgets.len(), 2);
        assert!(widgets.contains("clock-left"));
        assert!(widgets.contains("clock-right"));
    }

    #[test]
    fn clearing_damage_resets_tracker() {
        let mut tracker = OutputDamageTracker::default();
        tracker.mark_full();
        tracker.clear();

        assert_eq!(
            tracker.snapshot(),
            OutputDamage::Widgets(std::collections::HashSet::new()),
        );
    }
}
