// Copyright (C) 2025  Braiins Systems s.r.o.
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

//! Compositor state management combining Smithay handlers with deck_widget protocol.

use std::collections::HashMap;

use super::lifecycle_emitter::LifecycleEmitter;
use super::protocol::{
    DeckWidgetHandler, DeckWidgetProtocolState, WidgetManagerUserData, WidgetSurfaceUserData,
};
use super::widget_tracker::{LifecycleState, WidgetTracker};
use crate::compositor::layer_surface::{LayerEntry, replace_buffer};
use bmc::compositor::InstanceId;
use bmc_widget_protocol::server::{
    deck_widget_manager_v1::DeckWidgetManagerV1, deck_widget_manager_v2::DeckWidgetManagerV2,
    deck_widget_surface_v1::DeckWidgetSurfaceV1,
};
use deck_screen_edge_v1::server::deck_screen_edge_manager_v1::Border;
use deck_settings_v1::server::deck_settings_v1::Capability;
use smithay::{
    backend::allocator::{Buffer, Format, Fourcc, Modifier, dmabuf::Dmabuf},
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_image_capture_source,
    delegate_image_copy_capture, delegate_layer_shell, delegate_output,
    delegate_output_capture_source, delegate_seat, delegate_shm, delegate_xdg_shell,
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus, touch::TouchHandle},
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::{
        self as wl, Client, Display, DisplayHandle, Resource,
        backend::{ClientData, ClientId, DisconnectReason, ObjectId},
        protocol::{
            wl_buffer::WlBuffer, wl_callback::WlCallback, wl_output::WlOutput, wl_seat::WlSeat,
            wl_shm, wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Rectangle, Serial, Size},
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
        shell::{
            wlr_layer::{
                Layer, LayerSurface, LayerSurfaceCachedState, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
        },
        shm::{ShmHandler, ShmState},
    },
};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

/// Minimum interval between consecutive frame callbacks for a single widget.
/// Compositor-side pacing: caps each widget's submission rate regardless of
/// how fast the display page-flips or how the widget behaves client-side.
///
/// 32 ms ≈ every second vblank on a 63 Hz display → ~31 Hz effective cap.
///
/// Rationale for ~30 Hz as the ceiling (not the display refresh rate):
/// - It's a hard ceiling, not a target. Widgets render in response to their
///   own content rate (e.g. flip-clock peaks at ~12 Hz during a digit flip),
///   and the cap only kicks in when something would otherwise submit every
///   vblank — which for embedded UI content is pure waste.
/// - 30 Hz is the accepted "smooth enough" threshold for UI animation across
///   Android/iOS/browser conventions; no widget we expect to run here needs
///   to animate faster.
/// - Halving the display rate (rather than thirding/quartering) leaves
///   enough headroom for a widget that genuinely wants 30 Hz motion; any
///   more aggressive cap would visibly stutter such content.
/// - Picking 32 ms rather than exactly 1/30 s makes the cap phase-lock to
///   every other vblank on the 63 Hz panel, so submissions land on a
///   page-flip boundary instead of drifting across frames.
const FRAME_CALLBACK_MIN_INTERVAL: Duration = Duration::from_millis(32);

/// Send `wl_buffer.release` for every buffer the compositor still holds for
/// `instance_id` and queue its textures for eviction. Returns the clients
/// that received a release so the caller can flush them. A free function
/// over the buffer fields rather than a `CompositorState` method because
/// lifecycle emission calls it while `deck_widget_state` is mutably
/// borrowed from the same struct.
pub fn release_widget_buffers(
    widget_buffers: &mut Vec<(WlBuffer, InstanceId)>,
    invalidated_buffers: &mut Vec<ObjectId>,
    instance_id: &InstanceId,
) -> Vec<ClientId> {
    let removed: Vec<_> = widget_buffers
        .extract_if(.., |(_, id)| id == instance_id)
        .collect();

    let mut clients = Vec::new();
    for (buffer, _) in removed {
        tracing::debug!("Releasing off-screen buffer for dormant widget {instance_id}");
        invalidated_buffers.push(buffer.id());
        buffer.release();
        if let Some(client_id) = buffer.client().map(|client| client.id())
            && !clients.contains(&client_id)
        {
            clients.push(client_id);
        }
    }
    clients
}

fn remove_destroyed_widget_buffers<T>(
    widget_buffers: &mut Vec<(T, InstanceId)>,
    destroyed_id: &ObjectId,
    buffer_id: impl Fn(&T) -> ObjectId,
) -> Vec<InstanceId> {
    let mut removed = Vec::new();
    widget_buffers.retain(|(buffer, instance_id)| {
        if buffer_id(buffer) == *destroyed_id {
            removed.push(instance_id.clone());
            false
        } else {
            true
        }
    });
    removed
}

#[expect(clippy::struct_field_names)]
pub struct CompositorState {
    display_handle: DisplayHandle,
    pub compositor_state: SmithayCompositorState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    /// Tracked wlr-layer-shell surfaces, drawn above the scene.
    pub layer_surfaces: Vec<crate::compositor::layer_surface::LayerEntry>,
    pub screen_edge_sessions: Vec<crate::compositor::screen_edge::ScreenEdgeSession>,
    pub settings: crate::compositor::settings::SettingsState,
    pub alarm: crate::compositor::alarm::AlarmState,
    pub upgrade: crate::compositor::upgrade::UpgradeState,
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
    pub pending_layer_frame_callbacks: Vec<WlCallback>,

    /// Widget registration and connection tracking.
    pub widgets: WidgetTracker,

    /// Tracks the last-emitted lifecycle state per widget to compute
    /// release/acquire batches on scene changes.
    pub lifecycle: LifecycleEmitter,

    /// Per-widget frame generations used to correlate frame callbacks with
    /// the content that was actually presented.
    widget_frame_clocks: std::collections::HashMap<InstanceId, WidgetFrameClockState>,

    /// Touch handle for sending wl_touch events to widget surfaces.
    pub touch_handle: TouchHandle<Self>,

    /// Render surfaces indexed by widget instance_id (populated during surface commit).
    pub render_surfaces: HashMap<InstanceId, WlSurface>,

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
}

#[derive(Debug)]
pub struct PendingFrameCallback {
    pub callback: WlCallback,
    pub instance_id: Option<InstanceId>,
    pub client_pid: Option<u32>,
    /// `None` is a placeholder used when the callback was queued before
    /// the widget's `instance_id` resolved. Such callbacks bypass the
    /// generation comparison once resolved.
    pub generation: Option<NonZeroU64>,
}

/// Per-widget bookkeeping for the frame-callback delivery path.
///
/// - `latest_generation` advances on every widget commit (new content
///   published). `None` means the widget has never committed.
/// - `last_presented_generation` advances only at the fire site in
///   [`CompositorState::send_frame_callbacks_for_presented_widgets`],
///   so a callback deferred by the rate cap stays eligible on the
///   next tick rather than being dropped from eligibility. `None` means
///   no callback has ever fired for this widget.
/// - `last_callback_fired_at` drives the 32 ms per-widget rate cap.
///   `None` means "never fired yet" — the first callback fires without
///   waiting.
///
/// `NonZeroU64` rather than `u64` makes the "never happened" state a
/// type-level property (`None`) rather than an opaque `0` sentinel, and
/// niche optimisation keeps the fields the same size as bare `u64`.
#[derive(Debug, Default, Clone, Copy)]
struct WidgetFrameClockState {
    latest_generation: Option<NonZeroU64>,
    last_presented_generation: Option<NonZeroU64>,
    last_callback_fired_at: Option<Instant>,
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

    fn is_empty(&self) -> bool {
        !self.full_damage && self.widgets.is_empty()
    }

    fn clear(&mut self) {
        self.full_damage = false;
        self.widgets.clear();
    }
}

fn should_complete_frame_callback(
    instance_id: Option<&InstanceId>,
    generation: Option<NonZeroU64>,
    eligible_generations: &std::collections::HashMap<InstanceId, NonZeroU64>,
) -> bool {
    instance_id.is_some_and(|instance_id| {
        eligible_generations
            .get(instance_id)
            .is_some_and(|eligible_generation| generation.is_none_or(|g| g <= *eligible_generation))
    })
}

impl CompositorState {
    pub fn deactivate_retained_widget(&mut self, key: bmc::compositor::WidgetInstanceKey) -> bool {
        let instance_id = key.to_string();
        let Some(detached) = self.deck_widget_state.deactivate_widget(key) else {
            return false;
        };
        self.lifecycle.forget(&instance_id);
        self.drop_widget_render_state(&instance_id, detached.pid);
        if let Some(client_id) = detached.client_id {
            self.display_handle
                .backend_handle()
                .kill_client(client_id, DisconnectReason::ConnectionClosed);
        }
        true
    }

    pub fn unregister_retained_widget(&mut self, key: bmc::compositor::WidgetInstanceKey) -> bool {
        let instance_id = key.to_string();
        let Some(detached) = self.deck_widget_state.unregister_retained_widget(key) else {
            return false;
        };
        self.drop_widget_render_state(&instance_id, detached.pid);
        self.lifecycle.forget(&instance_id);
        if let Some(client_id) = detached.client_id {
            self.display_handle
                .backend_handle()
                .kill_client(client_id, DisconnectReason::ConnectionClosed);
        }
        true
    }

    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "compositor construction threads display geometry, seat, and the settings capability set"
    )]
    pub fn new(
        display: &Display<Self>,
        width: u32,
        height: u32,
        physical_width: u32,
        physical_height: u32,
        refresh_mhz: i32,
        seat_name: &str,
        settings_caps: Capability,
    ) -> Self {
        let display_handle = display.handle();

        let compositor_state = SmithayCompositorState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);

        // Vivante-tiled lets the GC400 sample client buffers directly instead
        // of allocating a full-size tiled shadow copy per linear import.
        let dmabuf_formats = [
            Format {
                code: Fourcc::Xrgb8888,
                modifier: Modifier::Vivante_tiled,
            },
            Format {
                code: Fourcc::Argb8888,
                modifier: Modifier::Vivante_tiled,
            },
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
        let layer_shell_state = WlrLayerShellState::new::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);

        let mut seat = seat_state.new_wl_seat(&display_handle, seat_name);
        let touch_handle = seat.add_touch();

        let deck_widget_state = DeckWidgetProtocolState::new();
        super::protocol::create_global::<Self>(&display_handle);
        super::screen_edge::create_global(&display_handle);
        super::alarm::create_global(&display_handle);
        super::upgrade::create_global(&display_handle);
        super::settings::create_global(&display_handle);

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
            layer_shell_state,
            layer_surfaces: Vec::new(),
            screen_edge_sessions: Vec::new(),
            settings: crate::compositor::settings::SettingsState::new(settings_caps),
            alarm: crate::compositor::alarm::AlarmState::default(),
            upgrade: crate::compositor::upgrade::UpgradeState::default(),
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
            pending_layer_frame_callbacks: Vec::new(),
            widgets: WidgetTracker::with_screen_width(width),
            lifecycle: LifecycleEmitter::new(),
            widget_frame_clocks: std::collections::HashMap::new(),
            touch_handle,
            render_surfaces: HashMap::new(),
            invalidated_buffers: Vec::new(),
            dirty_buffers: Vec::new(),
            pending_capture_frames: Vec::new(),
            capture_enabled: true,
            output_damage: OutputDamageTracker {
                full_damage: true,
                widgets: std::collections::HashSet::new(),
            },
        }
    }

    fn advance_widget_frame_generation(&mut self, instance_id: &InstanceId) -> NonZeroU64 {
        let state = self
            .widget_frame_clocks
            .entry(instance_id.clone())
            .or_default();
        // `saturating_add` never wraps to zero; u64 would take ~18.9B years
        // to overflow at 31 Hz per widget, so the pin-at-MAX degenerate is
        // purely academic.
        let next = state
            .latest_generation
            .map_or(NonZeroU64::MIN, |g| g.saturating_add(1));
        state.latest_generation = Some(next);
        next
    }

    pub fn latest_widget_generation(&self, instance_id: &InstanceId) -> Option<NonZeroU64> {
        self.widget_frame_clocks
            .get(instance_id)
            .and_then(|state| state.latest_generation)
    }

    fn queue_frame_callbacks(
        &mut self,
        callbacks: &mut Vec<WlCallback>,
        instance_id: Option<&InstanceId>,
        client_pid: Option<u32>,
        generation: Option<NonZeroU64>,
    ) {
        let pending_instance_id = instance_id.cloned();

        // Protocol-compliant clients wait for `done` before requesting another
        // frame, so the queue stays at most one deep per widget. Misbehaving or
        // hidden-widget clients can keep requesting frames indefinitely; bound
        // the queue to one pending callback per widget to prevent unbounded
        // growth. Earlier pending callbacks are dropped silently — firing
        // `done` on them would drive an even-faster submission loop for hidden
        // widgets, which is worse than the client's missing event.
        if let Some(id) = pending_instance_id.as_ref() {
            self.pending_frame_callbacks
                .retain(|pending| pending.instance_id.as_ref() != Some(id));
        }

        self.pending_frame_callbacks
            .extend(callbacks.drain(..).map(|callback| PendingFrameCallback {
                callback,
                instance_id: pending_instance_id.clone(),
                client_pid,
                generation,
            }));
    }

    /// Hit-test a point against mapped layer surfaces (topmost painted first,
    /// honoring the input region) and then visible widgets in the active scene.
    /// Returns the surface and its origin in logical coords if one is hit.
    #[must_use]
    pub fn touch_focus_at(&self, x: f64, y: f64) -> Option<(WlSurface, Point<f64, Logical>)> {
        use crate::compositor::layer_surface::{layer_rank, paint_order};

        // Layer pass: topmost painted surface that is mapped, contains the
        // point, and accepts input at that point.
        let mapped: Vec<&crate::compositor::layer_surface::LayerEntry> = self
            .layer_surfaces
            .iter()
            .filter(|e| e.is_mapped())
            .collect();
        let ranks: Vec<u8> = mapped.iter().map(|e| layer_rank(e.layer)).collect();
        for &i in paint_order(&ranks).iter().rev() {
            let entry = mapped[i];
            let Some(g) = entry.last_geometry else {
                continue;
            };
            let gx = f64::from(g.loc.x);
            let gy = f64::from(g.loc.y);
            let gw = f64::from(g.size.w);
            let gh = f64::from(g.size.h);
            if !(x >= gx && x < gx + gw && y >= gy && y < gy + gh) {
                continue;
            }
            let surface = entry.surface.wl_surface();
            if !surface.is_alive() {
                continue;
            }
            // Honor the surface input region: None means whole surface accepts
            // input; an explicit region must contain the surface-local point.
            let local = Point::<f64, Logical>::from((x - gx, y - gy));
            let accepts = with_states(surface, |states| {
                let mut guard = states.cached_state.get::<SurfaceAttributes>();
                match &guard.current().input_region {
                    None => true,
                    Some(region) => region.contains(local.to_i32_round()),
                }
            });
            if accepts {
                return Some((surface.clone(), Point::from((gx, gy))));
            }
            // Region rejects: continue to the next layer surface, then widgets if none match.
        }

        let scene = self.widgets.active_scene();

        for widget in &scene.widgets {
            if !widget.visible {
                continue;
            }

            let wx = f64::from(widget.position.x);
            let wy = f64::from(widget.position.y);
            let ww = f64::from(widget.size.width);
            let wh = f64::from(widget.size.height);

            if x >= wx
                && x < wx + ww
                && y >= wy
                && y < wy + wh
                && let Some(surface) = self.render_surfaces.get(&widget.instance_id)
                && surface.is_alive()
            {
                return Some((surface.clone(), Point::from((wx, wy))));
            }
        }

        None
    }

    pub fn drop_widget_render_surface(&mut self, instance_id: &InstanceId) {
        self.render_surfaces.remove(instance_id);
    }

    /// Eagerly invalidates textures on disconnect rather than waiting
    /// for `buffer_destroyed` to fire when the dead client is reaped.
    /// A late `buffer_destroyed` for the same buffer will push a
    /// duplicate id; `invalidate_textures` tolerates duplicates
    /// (a re-invalidate of an already-evicted texture is a no-op).
    pub fn drop_widget_buffers(&mut self, instance_id: &InstanceId) {
        let removed: Vec<_> = self
            .widget_buffers
            .extract_if(.., |(_, id)| id == instance_id)
            .collect();

        for (buffer, _) in removed {
            self.invalidated_buffers.push(buffer.id());
        }
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

    fn eligible_callback_generations(&self) -> std::collections::HashMap<InstanceId, NonZeroU64> {
        let visible_widgets = self.widgets.presented_widget_ids();
        let mut eligible = std::collections::HashMap::new();

        for (_buffer, instance_id) in &self.widget_buffers {
            if !visible_widgets.contains(instance_id) {
                continue;
            }

            let Some(state) = self.widget_frame_clocks.get(instance_id) else {
                continue;
            };

            if let Some(latest) = state.latest_generation
                && state.last_presented_generation < Some(latest)
            {
                eligible.insert(instance_id.clone(), latest);
            }
        }

        for instance_id in visible_widgets {
            let Some(state) = self.widget_frame_clocks.get(&instance_id) else {
                continue;
            };

            // Bootstrap visible widgets that still wait for their first frame
            // callback and have not attached a buffer yet.
            if state.last_presented_generation.is_none()
                && !self.widget_has_buffer(&instance_id)
                && let Some(latest) = state.latest_generation
            {
                eligible.insert(instance_id, latest);
            }
        }

        eligible
    }

    pub fn send_frame_callbacks_for_presented_widgets(&mut self, time: u32) {
        let eligible_generations = self.eligible_callback_generations();
        let pending_callbacks = std::mem::take(&mut self.pending_frame_callbacks);
        let mut deferred = Vec::with_capacity(pending_callbacks.len());
        let now = Instant::now();

        for pending in pending_callbacks {
            let resolved_instance_id = self.resolve_pending_callback_instance_id(&pending);
            if !should_complete_frame_callback(
                resolved_instance_id.as_ref(),
                pending.generation,
                &eligible_generations,
            ) {
                deferred.push(pending);
                continue;
            }

            // Per-widget minimum-interval pacing. Callbacks for widgets whose
            // previous callback fired within the interval are deferred and
            // re-evaluated on the next render pass.
            if let Some(id) = resolved_instance_id.as_ref()
                && let Some(state) = self.widget_frame_clocks.get(id)
                && let Some(last) = state.last_callback_fired_at
                && now.duration_since(last) < FRAME_CALLBACK_MIN_INTERVAL
            {
                deferred.push(pending);
                continue;
            }

            if let Some(id) = resolved_instance_id.as_ref()
                && let Some(state) = self.widget_frame_clocks.get_mut(id)
            {
                state.last_callback_fired_at = Some(now);
                if let Some(pending_gen) = pending.generation
                    && state
                        .last_presented_generation
                        .is_none_or(|p| pending_gen > p)
                {
                    state.last_presented_generation = Some(pending_gen);
                }
            }

            pending.callback.done(time);
        }

        self.pending_frame_callbacks = deferred;
    }

    pub fn send_layer_frame_callbacks(&mut self, time: u32) {
        for callback in self.pending_layer_frame_callbacks.drain(..) {
            callback.done(time);
        }
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

    /// Derived from output damage — any pending damage means the next
    /// iteration must render. Collapsing the two flags into one source of
    /// truth removes a class of "marked damage but forgot to flag redraw"
    /// bugs.
    pub fn needs_redraw(&self) -> bool {
        !self.output_damage.is_empty()
    }

    pub fn clear_output_damage(&mut self) {
        self.output_damage.clear();
    }

    /// Send the initial `lifecycle` event for a widget that has just
    /// connected and atomically sync the [`LifecycleEmitter`] cache so
    /// the next scene-step does not re-emit the same state.
    ///
    /// Returns the receiving widget's [`ClientId`] so the caller can
    /// scope the subsequent flush, or `None` when the widget has no
    /// attached surface yet (the next scene-step will deliver the
    /// lifecycle as a regular acquire).
    pub fn send_initial_lifecycle(
        &mut self,
        instance_id: &InstanceId,
        state: LifecycleState,
    ) -> Option<ClientId> {
        let client_id = self.deck_widget_state.send_lifecycle(instance_id, state)?;
        self.lifecycle.record_initial(instance_id, state);
        Some(client_id)
    }

    /// Mapped layer surfaces as (buffer, logical destination rect) in paint
    /// order (bottom first). The renderer scales each buffer into the rect,
    /// so visuals always agree with the geometry-sized touch hit-box; a
    /// buffer that actually mismatches the rect is warned about at the
    /// commit boundary (see `LayerEntry::warn_on_buffer_mismatch`).
    #[must_use]
    pub fn layer_render_items(&self) -> Vec<(WlBuffer, Rectangle<i32, Logical>)> {
        use crate::compositor::layer_surface::{layer_rank, paint_order};
        let mapped: Vec<&crate::compositor::layer_surface::LayerEntry> = self
            .layer_surfaces
            .iter()
            .filter(|e| e.is_mapped())
            .collect();
        let ranks: Vec<u8> = mapped.iter().map(|e| layer_rank(e.layer)).collect();
        paint_order(&ranks)
            .into_iter()
            .filter_map(|i| {
                let e = mapped[i];
                Some((e.buffer.clone()?, e.last_geometry?))
            })
            .collect()
    }

    /// True when a mapped layer surface above the background covers the whole
    /// output. While such a blocker is up, scene-drag is suppressed and
    /// scene-swipe neighbors are demoted from `Prepared` to `Dormant`.
    #[must_use]
    pub fn fullscreen_blocker_active(&self) -> bool {
        use crate::compositor::layer_surface::is_fullscreen_blocker;
        let output = Size::from((
            i32::try_from(self.width).expect("BUG: logical display width fits i32"),
            i32::try_from(self.height).expect("BUG: logical display height fits i32"),
        ));
        self.layer_surfaces.iter().any(|e| {
            e.is_mapped()
                && e.last_geometry
                    .is_some_and(|g| is_fullscreen_blocker(e.layer, g, output))
        })
    }

    /// True when a mapped full-screen overlay *below* the top (`Overlay`) layer
    /// is covering the scene — the firing-alarm or startup overlay, not the
    /// settings-tray itself (which lives on `Overlay` and is excluded by the
    /// layer-rank filter). Unlike `fullscreen_blocker_active`, this never counts
    /// the tray, so it can drive the `deck_settings_v1.preempted` signal that
    /// retracts the tray. Purely geometric: any full-screen preempting overlay
    /// qualifies, so new modal overlays need no wiring here.
    #[must_use]
    pub fn modal_overlay_active(&self) -> bool {
        use crate::compositor::layer_surface::{is_fullscreen_blocker, layer_rank};
        let output = Size::from((
            i32::try_from(self.width).expect("BUG: logical display width fits i32"),
            i32::try_from(self.height).expect("BUG: logical display height fits i32"),
        ));
        self.layer_surfaces.iter().any(|e| {
            e.is_mapped()
                && layer_rank(e.layer) < layer_rank(Layer::Overlay)
                && e.last_geometry
                    .is_some_and(|g| is_fullscreen_blocker(e.layer, g, output))
        })
    }

    /// True when `surface` is tracked as a wlr-layer-shell surface.
    #[must_use]
    pub fn surface_has_layer_role(&self, surface: &WlSurface) -> bool {
        self.layer_surfaces
            .iter()
            .any(|entry| entry.surface.wl_surface() == surface)
    }

    #[must_use]
    pub fn any_screen_edge_revealed(&self) -> bool {
        self.screen_edge_sessions
            .iter()
            .any(|session| session.flags.revealed)
    }

    #[must_use]
    pub fn neighbors_suppressed(&self) -> bool {
        self.fullscreen_blocker_active() || self.any_screen_edge_revealed()
    }

    pub fn trigger_screen_edge(&mut self, border: Border) -> bool {
        let mut resource = None;
        for session in &mut self.screen_edge_sessions {
            if session.flags.try_trigger(border) {
                resource = Some(session.resource.clone());
                break;
            }
        }

        let Some(resource) = resource else {
            return false;
        };

        resource.revealed();
        self.mark_full_output_damage();
        true
    }

    /// Re-arm the screen edge bound to `surface` (if any) when `unmapped`. An
    /// unmap is the hidden resting state, so an overlay that unmaps without
    /// re-arming would otherwise leave `revealed` set and the scene-drag
    /// suppression it drives (`any_screen_edge_revealed`) pinned on.
    fn rearm_screen_edge_on_unmap(&mut self, surface: &WlSurface, unmapped: bool) {
        if !unmapped {
            return;
        }
        if let Some(session) = self
            .screen_edge_sessions
            .iter_mut()
            .find(|session| session.surface == *surface)
        {
            session.flags.rearm();
        }
    }

    /// Handle a commit for a tracked layer surface. Returns `true` if `surface`
    /// is a layer surface (and was handled), `false` otherwise.
    fn commit_layer_surface(&mut self, surface: &WlSurface) -> bool {
        use crate::compositor::layer_surface::{
            LayerPlacement, layer_commit_effects, layer_geometry, replace_buffer,
        };

        let Some(idx) = self
            .layer_surfaces
            .iter()
            .position(|e| e.surface.wl_surface() == surface)
        else {
            return false;
        };

        let layer_surface = self.layer_surfaces[idx].surface.clone();
        let needs_configure = layer_surface.has_pending_changes();

        // Read placement AND layer from the committed cached state, so a client
        // that changes its layer or anchor is reflected.
        let (placement, layer) = layer_surface.with_cached_state(|s: &LayerSurfaceCachedState| {
            (
                LayerPlacement {
                    size: s.size,
                    anchor: s.anchor,
                    margin: s.margin,
                },
                s.layer,
            )
        });
        let output_w = i32::try_from(self.width).expect("BUG: logical display width fits i32");
        let output_h = i32::try_from(self.height).expect("BUG: logical display height fits i32");
        let geometry = layer_geometry(&placement, Size::from((output_w, output_h)));
        let old_layer = self.layer_surfaces[idx].layer;
        let old_geometry = self.layer_surfaces[idx].last_geometry;
        let effects = layer_commit_effects(
            self.layer_surfaces[idx].is_mapped(),
            old_layer,
            old_geometry,
            layer,
            geometry,
        );
        self.layer_surfaces[idx].layer = layer;
        if effects.geometry_changed {
            self.layer_surfaces[idx].last_geometry = Some(geometry);
        }

        if needs_configure {
            layer_surface.with_pending_state(|state| state.size = Some(geometry.size));
            layer_surface.send_configure();
        }

        // Buffer handling. Collect bookkeeping into locals to avoid borrowing
        // self inside the with_states closure.
        let mut release: Option<WlBuffer> = None;
        let mut invalidate: Option<ObjectId> = None;
        let mut dirty: Option<ObjectId> = None;
        let mut had_buffer_change = false;
        let mut had_damage = false;
        let mut unmapped = false;
        let mut drained_callbacks: Vec<WlCallback> = Vec::new();

        with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attributes = guard.current();

            had_damage = !attributes.damage.is_empty();
            drained_callbacks.append(&mut attributes.frame_callbacks);
            let buffer_scale = attributes.buffer_scale;

            if let Some(assignment) = attributes.buffer.take() {
                had_buffer_change = true;
                let entry = &mut self.layer_surfaces[idx];
                match assignment {
                    BufferAssignment::NewBuffer(buffer) => {
                        entry.warn_on_buffer_mismatch(&buffer, geometry, buffer_scale);
                        let new_id = buffer.id();
                        let (old_buf, old_id) = replace_buffer(
                            &mut entry.buffer,
                            &mut entry.buffer_id,
                            Some((buffer, new_id.clone())),
                        );
                        release = old_buf;
                        invalidate = old_id;
                        dirty = Some(new_id);
                        entry.last_geometry = Some(geometry);
                    }
                    BufferAssignment::Removed => {
                        let (old_buf, old_id) =
                            replace_buffer(&mut entry.buffer, &mut entry.buffer_id, None);
                        release = old_buf;
                        invalidate = old_id;
                        unmapped = true;
                        // last_geometry stays so the renderer repaints the vacated region.
                    }
                }
            }
            attributes.damage.clear();
        });

        let had_callbacks = !drained_callbacks.is_empty();
        self.pending_layer_frame_callbacks
            .append(&mut drained_callbacks);

        if let Some(buf) = release {
            buf.release();
        }
        if let Some(id) = invalidate {
            self.invalidated_buffers.push(id);
        }
        if let Some(id) = dirty {
            self.dirty_buffers.push(id);
        }
        if had_buffer_change || effects.needs_damage || had_damage || had_callbacks {
            self.mark_full_output_damage();
        }
        self.rearm_screen_edge_on_unmap(surface, unmapped);

        true
    }
}

impl DeckWidgetHandler for CompositorState {
    fn deck_widget_state(&mut self) -> &mut DeckWidgetProtocolState {
        &mut self.deck_widget_state
    }

    fn drop_widget_render_state(&mut self, instance_id: &InstanceId, pid: Option<u32>) {
        self.mark_full_output_damage();
        self.drop_widget_callback_state(instance_id, pid);
        self.drop_widget_render_surface(instance_id);
        self.drop_widget_buffers(instance_id);
    }

    fn forget_widget_lifecycle(&mut self, instance_id: &InstanceId) {
        self.lifecycle.forget(instance_id);
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
        if self.commit_layer_surface(surface) {
            return;
        }

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

        // Track render surface → instance_id mapping for wl_touch event routing.
        // Always insert (not or_insert) so reconnecting widgets update the surface.
        if let Some(ref id) = instance_id {
            self.render_surfaces.insert(id.clone(), surface.clone());
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
            // and expects display feedback — mark damage to trigger a redraw.
            // This covers Slint widgets that render to the same buffer without
            // re-attaching.
            if had_frame_callbacks {
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
        // Track destroyed buffer for texture cache invalidation.
        let buffer_id = buffer.id();
        self.invalidated_buffers.push(buffer_id.clone());

        // Smithay reports destroyed buffers as no longer usable. If a widget
        // still points at this buffer, drop the render candidate too; otherwise
        // scene transitions keep trying to draw an id whose texture was just
        // evicted.
        let removed_instances =
            remove_destroyed_widget_buffers(&mut self.widget_buffers, &buffer_id, WlBuffer::id);
        if !removed_instances.is_empty() {
            self.dirty_buffers.retain(|id| id != &buffer_id);
        }
        for instance_id in removed_instances {
            self.mark_widget_output_damage(&instance_id);
            tracing::debug!(
                "Dropped destroyed buffer {:?} for widget {}",
                buffer_id,
                instance_id
            );
        }

        // Clear a matching layer entry buffer so the renderer does not try to
        // draw a dead texture.
        let cleared_layer = self
            .layer_surfaces
            .iter_mut()
            .find(|e| e.buffer_id.as_ref() == Some(&buffer_id))
            .map(|entry| {
                entry.buffer = None;
                entry.buffer_id = None;
            })
            .is_some();
        if cleared_layer {
            self.dirty_buffers.retain(|id| id != &buffer_id);
            self.mark_full_output_damage();
            tracing::debug!("Dropped destroyed buffer {:?} for layer surface", buffer_id);
        }
    }
}

impl DmabufHandler for CompositorState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
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

    fn focus_changed(&mut self, _: &Seat<Self>, _: Option<&Self::KeyboardFocus>) {}

    fn cursor_image(&mut self, _: &Seat<Self>, _: CursorImageStatus) {}
}

impl XdgShellHandler for CompositorState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
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

    fn new_popup(&mut self, _: PopupSurface, _: PositionerState) {}

    fn grab(&mut self, _: PopupSurface, _: WlSeat, _: Serial) {}

    fn reposition_request(&mut self, _: PopupSurface, _: PositionerState, _: u32) {}
}

impl WlrLayerShellHandler for CompositorState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
        layer: Layer,
        _namespace: String,
    ) {
        self.layer_surfaces.push(LayerEntry::new(surface, layer));
        self.mark_full_output_damage();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        if let Some(pos) = self
            .layer_surfaces
            .iter()
            .position(|e| e.surface.wl_surface() == surface.wl_surface())
        {
            let mut entry = self.layer_surfaces.remove(pos);
            let (old_buf, old_id) = replace_buffer(&mut entry.buffer, &mut entry.buffer_id, None);
            if let Some(buf) = old_buf {
                buf.release();
            }
            if let Some(id) = old_id {
                self.invalidated_buffers.push(id);
            }
            self.mark_full_output_damage();
            let destroyed_surface = surface.wl_surface().clone();
            self.screen_edge_sessions
                .retain(|session| session.surface != destroyed_surface);
        } else {
            tracing::warn!("layer_destroyed for untracked surface");
        }
    }
}

// ---- Image capture protocol handlers ----

impl OutputHandler for CompositorState {
    fn output_bound(&mut self, _: Output, _: WlOutput) {}
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
    fn capture_constraints(&mut self, _: &ImageCaptureSource) -> Option<BufferConstraints> {
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

    fn frame(&mut self, _: &SessionRef, frame: Frame) {
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
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

delegate_compositor!(self::CompositorState);
delegate_shm!(self::CompositorState);
delegate_dmabuf!(self::CompositorState);
delegate_seat!(self::CompositorState);
delegate_xdg_shell!(self::CompositorState);
delegate_layer_shell!(self::CompositorState);
delegate_data_device!(self::CompositorState);
delegate_output!(self::CompositorState);
delegate_image_capture_source!(self::CompositorState);
delegate_output_capture_source!(self::CompositorState);
delegate_image_copy_capture!(self::CompositorState);

wl::delegate_global_dispatch!(
    CompositorState: [DeckWidgetManagerV1: ()] => DeckWidgetProtocolState
);
wl::delegate_dispatch!(
    CompositorState: [DeckWidgetManagerV1: WidgetManagerUserData] => DeckWidgetProtocolState
);
wl::delegate_global_dispatch!(
    CompositorState: [DeckWidgetManagerV2: ()] => DeckWidgetProtocolState
);
wl::delegate_dispatch!(
    CompositorState: [DeckWidgetManagerV2: WidgetManagerUserData] => DeckWidgetProtocolState
);
wl::delegate_dispatch!(
    CompositorState: [DeckWidgetSurfaceV1: WidgetSurfaceUserData] => DeckWidgetProtocolState
);

#[cfg(test)]
mod tests {
    use super::{
        ClientState, CompositorState, OutputDamage, OutputDamageTracker, PendingFrameCallback,
        remove_destroyed_widget_buffers, should_complete_frame_callback,
    };
    use bmc::compositor::{
        Position, Size, WidgetConnectionMode, WidgetGeneration, WidgetInstanceKey, WidgetPlacement,
        WidgetRegistration,
    };
    use bmc_widget_protocol::{DisplayInfo, ViewportShape, WidgetInitialConfig};
    use std::collections::HashMap;
    use std::num::NonZeroU64;
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use smithay::reexports::wayland_server::{
        Display, backend::ObjectId, protocol::wl_callback::WlCallback,
    };

    fn gen_n(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).expect("BUG: test generation must be non-zero")
    }

    #[test]
    fn lifecycle_cutoff_removes_unresolved_callback_by_prebind_pid() {
        for unregister in [false, true] {
            let display = Display::<CompositorState>::new()
                .expect("BUG: test Wayland display should initialize");
            let mut handle = display.handle();
            let (socket, _peer) =
                UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
            let client = handle
                .insert_client(socket, Arc::new(ClientState::default()))
                .expect("BUG: test Wayland client should register");
            let callback = client
                .create_resource::<WlCallback, _, CompositorState>(&handle, 1, ())
                .expect("BUG: test callback should initialize");
            let mut state = CompositorState::new(
                &display,
                480,
                1280,
                480,
                1280,
                60_000,
                "test-seat",
                crate::compositor::settings::caps_for_product(bmc_platform::Product::Bmc100),
            );
            let key = WidgetInstanceKey::from(bmc::scene::WidgetId::generate());
            let instance_id = key.to_string();
            let config = WidgetInitialConfig {
                width: 100,
                height: 100,
                viewport_shape: ViewportShape::Rectangular,
                display: DisplayInfo::BMC100,
                params: serde_json::Map::new(),
                credentials: serde_json::Map::new(),
                credential_secrets: bmc_widget_protocol::CredentialSecrets::default(),
                token: instance_id.clone(),
            };
            state.deck_widget_state.register_widget(
                instance_id.clone(),
                WidgetGeneration(1),
                config.clone(),
            );
            state
                .deck_widget_state
                .set_widget_pid(&instance_id, WidgetGeneration(1), 123);
            state
                .deck_widget_state
                .register_retained_widget(WidgetRegistration {
                    key,
                    connection_mode: WidgetConnectionMode::Accepting,
                    placement: WidgetPlacement {
                        instance_id,
                        position: Position { x: 0, y: 0 },
                        size: Size {
                            width: 100,
                            height: 100,
                        },
                        visible: true,
                    },
                    initial_config: config,
                });
            state.pending_frame_callbacks.push(PendingFrameCallback {
                callback,
                instance_id: None,
                client_pid: Some(123),
                generation: None,
            });

            if unregister {
                state.unregister_retained_widget(key);
            } else {
                state.deactivate_retained_widget(key);
            }

            assert!(state.pending_frame_callbacks.is_empty());
        }
    }

    #[test]
    fn unknown_surface_callback_stays_deferred_until_it_resolves() {
        assert!(!should_complete_frame_callback(
            None,
            Some(gen_n(1)),
            &HashMap::new(),
        ));
    }

    #[test]
    fn presented_generation_completes_equal_or_older_callback() {
        let mut presented = HashMap::new();
        presented.insert(String::from("clock-left"), gen_n(3));

        assert!(should_complete_frame_callback(
            Some(&String::from("clock-left")),
            Some(gen_n(3)),
            &presented,
        ));
        assert!(should_complete_frame_callback(
            Some(&String::from("clock-left")),
            Some(gen_n(2)),
            &presented,
        ));
    }

    #[test]
    fn newer_or_unpresented_widget_callback_is_deferred() {
        let mut eligible_generations = HashMap::new();
        eligible_generations.insert(String::from("clock-left"), gen_n(2));

        assert!(!should_complete_frame_callback(
            Some(&String::from("clock-left")),
            Some(gen_n(3)),
            &eligible_generations,
        ));
        assert!(!should_complete_frame_callback(
            Some(&String::from("clock-right")),
            Some(gen_n(1)),
            &eligible_generations,
        ));
    }

    #[test]
    fn bootstrap_generation_allows_initial_widget_callback() {
        let mut eligible_generations = HashMap::new();
        eligible_generations.insert(String::from("clock-left"), gen_n(1));

        assert!(should_complete_frame_callback(
            Some(&String::from("clock-left")),
            Some(gen_n(1)),
            &eligible_generations,
        ));
    }

    #[test]
    fn unresolved_generation_placeholder_always_passes_for_known_widget() {
        let mut eligible_generations = HashMap::new();
        eligible_generations.insert(String::from("clock-left"), gen_n(5));

        assert!(should_complete_frame_callback(
            Some(&String::from("clock-left")),
            None,
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

    #[test]
    fn destroyed_widget_buffer_is_removed_from_render_set() {
        #[derive(Clone)]
        struct TestBuffer {
            id: ObjectId,
            name: &'static str,
        }

        let destroyed_id = ObjectId::null();
        let survivor_id = ObjectId::null();

        let destroyed_buffer = TestBuffer {
            id: destroyed_id.clone(),
            name: "destroyed",
        };
        let survivor_buffer = TestBuffer {
            id: survivor_id.clone(),
            name: "survivor",
        };

        let mut widget_buffers = vec![
            (destroyed_buffer, String::from("destroyed-instance")),
            (survivor_buffer, String::from("survivor-instance")),
        ];

        let removed = remove_destroyed_widget_buffers(
            &mut widget_buffers,
            &destroyed_id,
            |buffer: &TestBuffer| buffer.id.clone(),
        );

        assert_eq!(removed, vec![String::from("destroyed-instance")]);
        assert_eq!(widget_buffers.len(), 1);
        assert_eq!(widget_buffers[0].0.name, "survivor");
    }
}

#[cfg(test)]
mod keyed_widget_protocol_test {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use bmc::compositor::{
        Position, Size, WidgetConnectionMode, WidgetInstanceKey, WidgetPlacement,
        WidgetRegistration,
    };
    use bmc_widget_protocol::client::{
        deck_widget_manager_v2::{self, DeckWidgetManagerV2},
        deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
    };
    use bmc_widget_protocol::{CredentialSecrets, ViewportShape, WidgetInitialConfig};
    use smithay::reexports::wayland_server::Display;
    use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy as _, QueueHandle};

    use super::{ClientState, CompositorState};

    #[derive(Default)]
    struct TestClient {
        compositor: Option<wl_compositor::WlCompositor>,
        manager: Option<DeckWidgetManagerV2>,
        credentials: Option<String>,
        credential_secrets: Option<String>,
        configure_done: bool,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            (): &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                match interface.as_str() {
                    "wl_compositor" => {
                        state.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                    }
                    "deck_widget_manager_v2" => {
                        state.manager = Some(registry.bind(name, version.min(2), qh, ()));
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<wl_compositor::WlCompositor, ()> for TestClient {
        fn event(
            _: &mut Self,
            _: &wl_compositor::WlCompositor,
            _: wl_compositor::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<DeckWidgetManagerV2, ()> for TestClient {
        fn event(
            _: &mut Self,
            _: &DeckWidgetManagerV2,
            _: deck_widget_manager_v2::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_surface::WlSurface, ()> for TestClient {
        fn event(
            _: &mut Self,
            _: &wl_surface::WlSurface,
            _: wl_surface::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<DeckWidgetSurfaceV1, ()> for TestClient {
        fn event(
            state: &mut Self,
            _: &DeckWidgetSurfaceV1,
            event: deck_widget_surface_v1::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "the test client records only the version-sensitive initial events"
            )]
            match event {
                deck_widget_surface_v1::Event::Credentials { json } => {
                    state.credentials = Some(json);
                }
                deck_widget_surface_v1::Event::CredentialSecrets { json } => {
                    state.credential_secrets = Some(json);
                }
                deck_widget_surface_v1::Event::ConfigureDone => {
                    state.configure_done = true;
                }
                _ => {}
            }
        }
    }

    fn pump(
        display: &mut Display<CompositorState>,
        compositor: &mut CompositorState,
        conn: &Connection,
        queue: &mut EventQueue<TestClient>,
        client: &mut TestClient,
    ) {
        conn.flush()
            .expect("BUG: test client flush should succeed on a live socket pair");
        display
            .dispatch_clients(compositor)
            .expect("BUG: test server dispatch should succeed on a live socket pair");
        display
            .flush_clients()
            .expect("BUG: test server flush should succeed on a live socket pair");
        queue
            .blocking_dispatch(client)
            .expect("BUG: keyed factory should produce events for client dispatch");
    }

    fn new_server() -> (Display<CompositorState>, CompositorState) {
        let display = Display::new().expect("BUG: test Wayland display should initialize");
        let compositor = CompositorState::new(
            &display,
            480,
            1280,
            480,
            1280,
            60_000,
            "test-seat",
            crate::compositor::settings::caps_for_product(bmc_platform::Product::Bmc100),
        );
        (display, compositor)
    }

    fn connect_client(
        display: &mut Display<CompositorState>,
        compositor: &mut CompositorState,
    ) -> (Connection, EventQueue<TestClient>, TestClient) {
        let (server_stream, client_stream) =
            UnixStream::pair().expect("BUG: Unix socket pair should initialize");
        display
            .handle()
            .insert_client(server_stream, Arc::new(ClientState::default()))
            .expect("BUG: test client should register with the display");
        let conn = Connection::from_socket(client_stream)
            .expect("BUG: test client socket should form a Wayland connection");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = TestClient::default();
        conn.display().get_registry(&qh, ());
        pump(display, compositor, &conn, &mut queue, &mut client);
        (conn, queue, client)
    }

    fn request_keyed_surface(
        client: &TestClient,
        qh: &QueueHandle<TestClient>,
        key: String,
    ) -> DeckWidgetSurfaceV1 {
        let wl_compositor = client
            .compositor
            .as_ref()
            .expect("BUG: compositor global should be advertised");
        let manager = client
            .manager
            .as_ref()
            .expect("BUG: keyed manager global should be advertised");
        let surface = wl_compositor.create_surface(qh, ());
        manager.get_widget_surface(key, &surface, qh, ())
    }

    fn pump_protocol_error(
        display: &mut Display<CompositorState>,
        compositor: &mut CompositorState,
        conn: &Connection,
        queue: &mut EventQueue<TestClient>,
        client: &mut TestClient,
    ) -> wayland_client::backend::protocol::ProtocolError {
        conn.flush()
            .expect("BUG: invalid keyed request should reach the server");
        display
            .dispatch_clients(compositor)
            .expect("BUG: rejecting one client must not fail server dispatch");
        display
            .flush_clients()
            .expect("BUG: protocol error should flush to the rejected client");
        assert!(
            queue.blocking_dispatch(client).is_err(),
            "protocol error must terminate the rejected client"
        );
        conn.protocol_error()
            .expect("BUG: rejected client must retain its protocol error")
    }

    #[test]
    fn malformed_key_kills_only_its_client_with_invalid_key() {
        let (mut display, mut compositor) = new_server();
        let (conn, mut queue, mut client) = connect_client(&mut display, &mut compositor);
        let qh = queue.handle();
        request_keyed_surface(&client, &qh, "not-a-uuid".to_owned());

        let error = pump_protocol_error(
            &mut display,
            &mut compositor,
            &conn,
            &mut queue,
            &mut client,
        );
        assert_eq!(error.code, deck_widget_manager_v2::Error::InvalidKey as u32);
        assert_eq!(error.object_interface, "deck_widget_manager_v2");

        let (survivor, _, survivor_state) = connect_client(&mut display, &mut compositor);
        assert!(survivor.protocol_error().is_none());
        assert_eq!(
            survivor_state
                .manager
                .expect("BUG: server must keep advertising after rejecting one client")
                .version(),
            2
        );
    }

    #[test]
    fn unregistered_canonical_key_is_rejected_as_unknown_widget() {
        let (mut display, mut compositor) = new_server();
        let (conn, mut queue, mut client) = connect_client(&mut display, &mut compositor);
        let qh = queue.handle();
        let key = WidgetInstanceKey::from(bmc::scene::WidgetId::generate());
        request_keyed_surface(&client, &qh, key.to_string());

        let error = pump_protocol_error(
            &mut display,
            &mut compositor,
            &conn,
            &mut queue,
            &mut client,
        );
        assert_eq!(
            error.code,
            deck_widget_manager_v2::Error::UnknownWidget as u32
        );
        assert_eq!(error.object_interface, "deck_widget_manager_v2");
    }

    #[test]
    fn keyed_factory_creates_v2_surface_with_initial_credentials() {
        let (mut display, mut compositor) = new_server();
        let key = WidgetInstanceKey::from(bmc::scene::WidgetId::generate());
        let credentials = serde_json::json!({
            "weather": {"type": "weather-api", "account": "home"}
        });
        let credential_secrets = serde_json::json!({
            "weather": {"fields": {"token": "secret-value"}}
        });
        compositor
            .deck_widget_state
            .register_retained_widget(WidgetRegistration {
                key,
                connection_mode: WidgetConnectionMode::Accepting,
                placement: WidgetPlacement {
                    instance_id: key.to_string(),
                    position: Position { x: 0, y: 0 },
                    size: Size {
                        width: 100,
                        height: 100,
                    },
                    visible: true,
                },
                initial_config: WidgetInitialConfig {
                    width: 100,
                    height: 100,
                    viewport_shape: ViewportShape::Rectangular,
                    display: bmc_widget_protocol::DisplayInfo::BMC100,
                    params: serde_json::Map::new(),
                    credentials: credentials
                        .as_object()
                        .expect("BUG: credentials fixture must be an object")
                        .clone(),
                    credential_secrets: CredentialSecrets::new(
                        credential_secrets
                            .as_object()
                            .expect("BUG: credential secrets fixture must be an object")
                            .clone(),
                    ),
                    token: "keyed-v2-test".to_owned(),
                },
            });

        let (conn, mut queue, mut client) = connect_client(&mut display, &mut compositor);
        let qh = queue.handle();
        let manager = client
            .manager
            .clone()
            .expect("BUG: keyed manager global should be advertised");
        assert_eq!(manager.version(), 2);

        let widget_surface = request_keyed_surface(&client, &qh, key.to_string());
        assert_eq!(widget_surface.version(), 2);
        pump(
            &mut display,
            &mut compositor,
            &conn,
            &mut queue,
            &mut client,
        );

        assert_eq!(
            compositor.deck_widget_state.drain_connected(),
            vec![key.to_string()]
        );
        assert!(client.configure_done);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                client
                    .credentials
                    .as_deref()
                    .expect("BUG: v2 keyed surface must receive credentials")
            )
            .expect("BUG: credentials event must contain JSON"),
            credentials
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                client
                    .credential_secrets
                    .as_deref()
                    .expect("BUG: v2 keyed surface must receive credential secrets")
            )
            .expect("BUG: credential secrets event must contain JSON"),
            credential_secrets
        );
    }
}

/// Drives a real in-process Wayland client/server handshake over a
/// `UnixStream::pair()` to lock in the invariant that keeps a callback-only
/// layer-surface commit from starving its own callback: `commit_layer_surface`
/// must mark full output damage whenever it drained any queued frame
/// callbacks, even when the commit carried no buffer and no damage, because
/// `send_layer_frame_callbacks` only ever runs after a render.
#[cfg(test)]
mod layer_frame_callback_damage_test {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use smithay::reexports::wayland_server::Display;
    use wayland_client::protocol::{wl_callback, wl_compositor, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1, zwlr_layer_surface_v1,
    };

    use super::{ClientState, CompositorState, OutputDamage};

    #[derive(Default)]
    struct TestClient {
        compositor: Option<wl_compositor::WlCompositor>,
        layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
        configured: bool,
        frame_done: bool,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            (): &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                match interface.as_str() {
                    "wl_compositor" => {
                        state.compositor =
                            Some(registry.bind::<wl_compositor::WlCompositor, _, _>(
                                name,
                                version.min(6),
                                qh,
                                (),
                            ));
                    }
                    "zwlr_layer_shell_v1" => {
                        state.layer_shell = Some(
                            registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                                name,
                                version.min(4),
                                qh,
                                (),
                            ),
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<wl_compositor::WlCompositor, ()> for TestClient {
        fn event(
            _: &mut Self,
            _: &wl_compositor::WlCompositor,
            _: wl_compositor::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for TestClient {
        fn event(
            _: &mut Self,
            _: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
            _: zwlr_layer_shell_v1::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_surface::WlSurface, ()> for TestClient {
        fn event(
            _: &mut Self,
            _: &wl_surface::WlSurface,
            _: wl_surface::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for TestClient {
        fn event(
            state: &mut Self,
            layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
            event: zwlr_layer_surface_v1::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
                layer_surface.ack_configure(serial);
                state.configured = true;
            }
        }
    }

    impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
        fn event(
            state: &mut Self,
            _: &wl_callback::WlCallback,
            event: wl_callback::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_callback::Event::Done { .. } = event {
                state.frame_done = true;
            }
        }
    }

    /// Flush the client's queued requests, let the server dispatch and flush
    /// them, then dispatch the server's replies client-side. Only call this
    /// where the preceding requests are guaranteed to produce at least one
    /// server event (a fresh registry, a configure, a frame `done`) —
    /// `blocking_dispatch` really does block if nothing is queued to read.
    fn pump(
        display: &mut Display<CompositorState>,
        compositor: &mut CompositorState,
        conn: &Connection,
        queue: &mut EventQueue<TestClient>,
        client: &mut TestClient,
    ) {
        pump_server_only(display, compositor, conn);
        queue
            .blocking_dispatch(client)
            .expect("BUG: test client dispatch should succeed once the server has replied");
    }

    /// Flush the client's queued requests and let the server dispatch and
    /// flush them, without a client-side dispatch. Used for requests that are
    /// not expected to produce a reply (binds, acks, the callback-only probe
    /// commit itself).
    fn pump_server_only(
        display: &mut Display<CompositorState>,
        compositor: &mut CompositorState,
        conn: &Connection,
    ) {
        conn.flush()
            .expect("BUG: test client flush should succeed on a live socket pair");
        display
            .dispatch_clients(compositor)
            .expect("BUG: test server dispatch should succeed on a live socket pair");
        display
            .flush_clients()
            .expect("BUG: test server flush should succeed on a live socket pair");
    }

    #[test]
    fn callback_only_layer_commit_marks_full_output_damage() {
        let mut display: Display<CompositorState> =
            Display::new().expect("BUG: test Wayland display should initialize");
        let mut compositor = CompositorState::new(
            &display,
            480,
            1280,
            480,
            1280,
            60_000,
            "test-seat",
            crate::compositor::settings::caps_for_product(bmc_platform::Product::Bmc100),
        );

        let (server_stream, client_stream) =
            UnixStream::pair().expect("BUG: unix socket pair should be creatable");
        display
            .handle()
            .insert_client(server_stream, Arc::new(ClientState::default()))
            .expect("BUG: test client stream should be insertable into a fresh display");

        let conn = Connection::from_socket(client_stream)
            .expect("BUG: test client socket should form a valid connection");
        let mut queue: EventQueue<TestClient> = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = TestClient::default();

        conn.display().get_registry(&qh, ());
        pump(
            &mut display,
            &mut compositor,
            &conn,
            &mut queue,
            &mut client,
        );

        let compositor_global = client
            .compositor
            .clone()
            .expect("BUG: wl_compositor global should have been advertised");
        let layer_shell = client
            .layer_shell
            .clone()
            .expect("BUG: zwlr_layer_shell_v1 global should have been advertised");

        let surface = compositor_global.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Top,
            "bdk-564-callback-only-commit-test".to_owned(),
            &qh,
            (),
        );
        // An explicit size is required: zero size on an axis is a protocol
        // error unless that axis is anchored to both opposite edges.
        layer_surface.set_size(480, 128);
        // Initial commit with no buffer: this is what makes the compositor
        // send the first Configure.
        surface.commit();
        pump(
            &mut display,
            &mut compositor,
            &conn,
            &mut queue,
            &mut client,
        );
        assert!(
            client.configured,
            "BUG: initial commit should have produced a layer-surface configure"
        );

        pump_server_only(&mut display, &mut compositor, &conn);
        compositor.clear_output_damage();
        assert_eq!(
            compositor.current_output_damage(),
            OutputDamage::Widgets(std::collections::HashSet::new()),
            "test setup should start from a clean damage state before the probe commit"
        );

        // The probe: request a frame callback and commit with no buffer
        // attach and no damage, isolating the callback-only path.
        let callback = surface.frame(&qh, ());
        surface.commit();
        pump_server_only(&mut display, &mut compositor, &conn);

        assert_eq!(
            compositor.pending_layer_frame_callbacks.len(),
            1,
            "callback-only commit should have queued the frame callback"
        );
        assert_eq!(
            compositor.current_output_damage(),
            OutputDamage::Full,
            "callback-only commit must mark full output damage, or its callback \
             would starve forever since send_layer_frame_callbacks only runs after a render"
        );

        // Bonus: firing the queued callback (as a render would) reaches the
        // client as a wl_callback::Done.
        compositor.send_layer_frame_callbacks(0);
        display
            .flush_clients()
            .expect("BUG: test server flush should succeed on a live socket pair");
        queue
            .blocking_dispatch(&mut client)
            .expect("BUG: test client dispatch should succeed once the server has replied");
        assert!(
            client.frame_done,
            "BUG: a fired layer frame callback should reach the client as wl_callback::Done"
        );

        drop(callback);
        drop(layer_surface);
    }
}
