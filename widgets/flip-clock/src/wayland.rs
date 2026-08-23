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

//! Wayland client implementation for flip-clock widget.
//!
//! Production mode uses [`DeckWidgetSurfaceClient`] (single Wayland connection
//! for both rendering and protocol events). Standalone mode uses
//! [`XdgSurfaceClient`] (XDG toplevel, no compositor protocol).

use crate::AnimationMode;
use crate::digits::DigitTextures;
use crate::digits3d::Digit3DMeshes;
use crate::egl::EglState;
use crate::layout::ClockLayout;
use crate::renderer::{Mat4, Renderer};
use anyhow::Result;
use bmc_gpu_render_lock::GpuRenderLock;
use bmc_widget::surface::{
    DeckWidgetSurfaceClient, LifecycleState, PollOutcome, SettingUpdate, WidgetEvent,
    WidgetSurface, XdgSurfaceClient,
};
use chrono::Timelike;
use glow::HasContext;
use std::time::Instant;

/// Flip animation duration in seconds.
const FLIP_DURATION: f32 = 0.35;

/// Persistent animation state for the 6-digit clock (HH:MM:SS).
///
/// Tracks which digit value is currently displayed and, for each position,
/// when it last changed. Animation progress is derived from monotonic
/// [`Instant`] elapsed time, making it immune to wall-clock jitter.
struct FlipState {
    digits: [u8; 6],
    prev_digits: [u8; 6],
    transition_start: [Option<Instant>; 6],
}

impl FlipState {
    fn new() -> Self {
        Self {
            digits: [0; 6],
            prev_digits: [0; 6],
            transition_start: [None; 6],
        }
    }

    /// Update digits from the current wall-clock time. Any digit
    /// whose value differs from the displayed one starts a fresh
    /// flip animation; backward and large jumps animate the same
    /// way as a normal one-second tick.
    fn update(&mut self, hours: u8, minutes: u8, seconds: u8) {
        #[expect(clippy::integer_division, reason = "extracting digit values 0-9")]
        let new_digits: [u8; 6] = [
            hours / 10,
            hours % 10,
            minutes / 10,
            minutes % 10,
            seconds / 10,
            seconds % 10,
        ];

        let now = Instant::now();
        for (i, &new_digit) in new_digits.iter().enumerate() {
            if new_digit != self.digits[i] {
                self.prev_digits[i] = self.digits[i];
                self.digits[i] = new_digit;
                self.transition_start[i] = Some(now);
            }
        }
    }

    /// Returns `true` while any digit is mid-flip animation.
    fn is_animating(&self) -> bool {
        self.transition_start
            .iter()
            .any(|t| t.is_some_and(|start| start.elapsed().as_secs_f32() < FLIP_DURATION))
    }

    /// Eased animation progress for digit position `i` (0.0 .. 1.0).
    fn flip_progress(&self, i: usize) -> f32 {
        let Some(start) = self.transition_start[i] else {
            return 1.0;
        };
        let t = start.elapsed().as_secs_f32();
        let linear = (t / FLIP_DURATION).min(1.0);
        // Cubic ease-in-out
        if linear < 0.5 {
            4.0 * linear * linear * linear
        } else {
            1.0 - (-2.0 * linear + 2.0).powi(3) / 2.0
        }
    }
}

/// Render-loop phase. One enum replaces a pile of `needs_render` / `animating`
/// / `mark_needs_render` / `take_render_requested` conditionals.
///
/// Transitions:
/// - start → `RenderPending`
/// - after render: → `WaitingForCallback` if animating, else `WaitingForIdleTimeout`
/// - in `WaitingForCallback`: `wl_callback.done` arrival → `RenderPending`
/// - in `WaitingForIdleTimeout`: poll timeout expiry → `RenderPending`
/// - any state: timezone update event → `RenderPending`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopPhase {
    /// A render is due now.
    RenderPending,
    /// A render is due once the compositor releases the next export slot.
    WaitingForBufferRelease,
    /// Waiting indefinitely for the compositor's next frame callback.
    WaitingForCallback,
    /// Idle — waiting for the next wall-clock-second boundary.
    WaitingForIdleTimeout,
}

impl LoopPhase {
    fn poll_timeout_ms(self) -> i32 {
        match self {
            Self::RenderPending => 0,
            Self::WaitingForBufferRelease | Self::WaitingForCallback => -1,
            Self::WaitingForIdleTimeout => ms_to_next_second_boundary(),
        }
    }
}

fn ms_to_next_second_boundary() -> i32 {
    let now = std::time::SystemTime::now();
    let millis_into_second = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_millis());
    #[expect(clippy::cast_possible_wrap, reason = "millis < 1000, fits i32")]
    let remaining = (1_000 - millis_into_second) as i32;
    remaining.max(1)
}

/// Connect in standalone mode (XDG toplevel, no compositor protocol).
pub fn connect_standalone(
    animation_mode: AnimationMode,
    width: u32,
    height: u32,
    timezone: String,
) -> Result<()> {
    let mut surface = XdgSurfaceClient::connect(width, height, "Flip Clock", "bmc-flip-clock")?;
    tracing::info!("Connected to Wayland display (standalone mode)");
    run_render_loop(&mut surface, animation_mode, timezone, None)
}

/// Connect in production mode and return the surface client together with
/// the compositor-delivered initial state (size, params, settings). The
/// caller decodes params into widget-specific config, then hands the
/// surface to [`run_render_loop`].
pub fn connect_production() -> Result<(DeckWidgetSurfaceClient, bmc_widget::surface::InitialState)>
{
    let (surface, initial) = DeckWidgetSurfaceClient::connect_keyed()?;
    tracing::info!(
        "Connected to Wayland display (production mode): {}x{} viewport_shape={:?}",
        initial.width,
        initial.height,
        initial.viewport_shape
    );
    Ok((surface, initial))
}

/// Drive the render loop for a production-mode surface whose initial
/// state has already been collected.
pub fn run_production(
    mut surface: DeckWidgetSurfaceClient,
    animation_mode: AnimationMode,
    timezone_override: Option<String>,
    initial_settings: &[SettingUpdate],
) -> Result<()> {
    let mut system_timezone: Option<String> = None;
    for setting in initial_settings {
        if let SettingUpdate::Timezone(tz) = setting {
            system_timezone = Some(tz.clone());
        }
    }
    let system_timezone = system_timezone.unwrap_or_else(|| {
        tracing::warn!("No timezone in initial settings, defaulting to UTC");
        "UTC".to_owned()
    });
    run_render_loop(
        &mut surface,
        animation_mode,
        system_timezone,
        timezone_override,
    )
}

/// Mutable state carried across a single render-loop iteration.
struct RenderLoopState {
    animation_mode: AnimationMode,
    system_timezone: String,
    timezone_override: Option<String>,
    digit_meshes: Option<Digit3DMeshes>,
    flip_state: FlipState,
    phase: LoopPhase,
    /// Latest lifecycle state from the compositor; `None` until the first
    /// emission after the initial configure batch. `None` is treated as
    /// "not yet known" and gates rendering the same way as `Dormant` so
    /// we do not allocate DMA-BUF export buffers before the compositor
    /// has announced whether this widget should be visible.
    lifecycle: Option<LifecycleState>,
}

impl RenderLoopState {
    fn effective_timezone(&self) -> &str {
        self.timezone_override
            .as_deref()
            .unwrap_or(self.system_timezone.as_str())
    }

    fn apply_param_update(
        &mut self,
        manifest: crate::widget_protocol::ManifestParams,
        gl: &glow::Context,
        surface: &mut dyn WidgetSurface,
    ) -> Result<()> {
        let new_mode = AnimationMode::from(manifest.mode);
        if new_mode != self.animation_mode {
            tracing::info!("Animation mode updated: {:?}", new_mode);
            self.animation_mode = new_mode;
            if self.animation_mode == AnimationMode::Extruded && self.digit_meshes.is_none() {
                self.digit_meshes = Some(Digit3DMeshes::new(gl)?);
                tracing::info!("3D digit meshes created on mode flip");
            }
            surface.mark_needs_render();
        }

        if manifest.timezone != self.timezone_override {
            tracing::info!(
                "Per-widget timezone override updated: {:?}",
                manifest.timezone
            );
            self.timezone_override = manifest.timezone;
            self.phase = LoopPhase::RenderPending;
        }
        Ok(())
    }

    fn handle_events(
        &mut self,
        events: impl Iterator<Item = WidgetEvent>,
        gl: &glow::Context,
        surface: &mut dyn WidgetSurface,
    ) -> Result<()> {
        for event in events {
            match event {
                WidgetEvent::Setting(SettingUpdate::Timezone(new_tz)) => {
                    tracing::info!("System timezone updated: {new_tz}");
                    self.system_timezone = new_tz;
                    self.phase = LoopPhase::RenderPending;
                }
                WidgetEvent::ParamUpdate(params) => {
                    let manifest: crate::widget_protocol::ManifestParams =
                        match serde_json::from_value(params.into()) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::error!(
                                    "ParamUpdate: failed to decode manifest-validated \
                                     params — compositor and widget schemas have \
                                     diverged: {e}"
                                );
                                continue;
                            }
                        };
                    self.apply_param_update(manifest, gl, surface)?;
                }
                // This widget declares no credential slots, so a resolution
                // never applies to it and there is no secret for it to spend.
                WidgetEvent::Setting(_)
                | WidgetEvent::CredentialsUpdate(_)
                | WidgetEvent::SecretsUpdate(_)
                | WidgetEvent::Shutdown
                | WidgetEvent::TransitionIncoming
                | WidgetEvent::TouchDown { .. }
                | WidgetEvent::TouchMotion { .. }
                | WidgetEvent::TouchUp { .. }
                | WidgetEvent::TouchCancel => {}
                WidgetEvent::Lifecycle(_) => {
                    tracing::warn!("Lifecycle event reached handle_events; caller should filter");
                }
            }
        }
        Ok(())
    }
}

/// Shared render loop that works with any [`WidgetSurface`] backend.
#[expect(
    clippy::too_many_lines,
    reason = "single event loop keeps state transitions local"
)]
fn run_render_loop(
    surface: &mut dyn WidgetSurface,
    animation_mode: AnimationMode,
    system_timezone: String,
    timezone_override: Option<String>,
) -> Result<()> {
    let gpu_lock = GpuRenderLock::from_env()?;
    let mut egl = EglState::new(surface.width(), surface.height())?;
    tracing::info!("GBM-based EGL initialized, starting render loop");

    let renderer = Renderer::new(egl.gl(), surface.width(), surface.height())?;
    tracing::info!("OpenGL ES renderer initialized");

    let digit_textures = DigitTextures::new(egl.gl())?;
    tracing::info!("Digit textures created");

    // Lazily instantiated on first Extruded request so runtime mode
    // flips don't require a respawn; kept alive across
    // Flat→Extruded→Flat cycles to avoid re-uploading mesh data.
    let digit_meshes = if animation_mode == AnimationMode::Extruded {
        let meshes = Digit3DMeshes::new(egl.gl())?;
        tracing::info!("3D digit meshes created");
        Some(meshes)
    } else {
        None
    };

    let mut state = RenderLoopState {
        animation_mode,
        system_timezone,
        timezone_override,
        digit_meshes,
        flip_state: FlipState::new(),
        phase: LoopPhase::RenderPending,
        lifecycle: None,
    };

    while surface.running() {
        let outcome = surface.poll_dispatch(state.phase.poll_timeout_ms())?;

        if surface.take_render_requested() {
            state.phase = LoopPhase::RenderPending;
        }

        if state.phase == LoopPhase::WaitingForIdleTimeout && outcome == PollOutcome::Timeout {
            state.phase = LoopPhase::RenderPending;
        }

        let events = surface.drain_events();
        let mut other_events = Vec::with_capacity(events.len());
        for event in events {
            if let WidgetEvent::Lifecycle(new_state) = event {
                apply_lifecycle_change(&mut state, &mut egl, surface, new_state);
            } else {
                other_events.push(event);
            }
        }
        state.handle_events(other_events.into_iter(), egl.gl(), surface)?;

        let released_slots = surface.drain_released_slots();
        if !released_slots.is_empty() {
            egl.mark_released_slots(released_slots);
            if is_on_screen(state.lifecycle) && state.phase == LoopPhase::WaitingForBufferRelease {
                state.phase = LoopPhase::RenderPending;
            }
        }

        release_released_offscreen_buffers(&state, &mut egl, surface);

        if !is_on_screen(state.lifecycle) {
            state.phase = LoopPhase::WaitingForIdleTimeout;
            continue;
        }

        if state.phase != LoopPhase::RenderPending {
            continue;
        }

        if !egl.current_buffer_available() {
            state.phase = LoopPhase::WaitingForBufferRelease;
            continue;
        }

        let tz: chrono_tz::Tz = state.effective_timezone().parse().unwrap_or(chrono_tz::UTC);
        let now = chrono::Utc::now().with_timezone(&tz);

        #[expect(
            clippy::cast_possible_truncation,
            reason = "hour 0-23, minute/second 0-59 always fit in u8"
        )]
        let (hours, minutes, seconds) = (now.hour() as u8, now.minute() as u8, now.second() as u8);

        state.flip_state.update(hours, minutes, seconds);
        let layout = ClockLayout::for_viewport(surface.width(), surface.height());

        let lock = gpu_lock.lock("flip_clock")?;
        egl.begin_frame()?;
        egl.clear(0.0, 0.0, 0.0, 1.0);
        let gl = egl.gl();

        let meshes_for_frame = if state.animation_mode == AnimationMode::Extruded {
            state.digit_meshes.as_ref()
        } else {
            None
        };

        render_clock(
            &layout,
            &renderer,
            &digit_textures,
            meshes_for_frame,
            gl,
            &state.flip_state,
        );

        let animating = state.flip_state.is_animating();
        egl.wait_for_gpu();
        drop(lock);
        let (dmabuf_info, slot) = egl.export_frame()?;
        surface.submit_buffer(&dmabuf_info, slot, animating)?;

        state.phase = if animating {
            LoopPhase::WaitingForCallback
        } else {
            LoopPhase::WaitingForIdleTimeout
        };

        tracing::trace!(
            frame = surface.frame_count(),
            time = %format_args!("{hours:02}:{minutes:02}:{seconds:02}"),
            animating,
            "rendered",
        );
        if surface.frame_count().is_multiple_of(60) {
            tracing::debug!("Frame {}", surface.frame_count());
        }
    }

    Ok(())
}

fn is_on_screen(lifecycle: Option<LifecycleState>) -> bool {
    matches!(
        lifecycle,
        Some(LifecycleState::Visible | LifecycleState::Entering | LifecycleState::Leaving)
    )
}

fn release_released_offscreen_buffers(
    state: &RenderLoopState,
    egl: &mut EglState,
    surface: &mut dyn WidgetSurface,
) {
    if is_on_screen(state.lifecycle) {
        return;
    }
    let slots = egl.destroy_released_buffers();
    surface.invalidate_cached_buffer_slots(&slots);
}

/// Apply a new lifecycle state from the compositor.
///
/// Buffers are kept allocated while the widget is currently visible to the
/// user — `Visible`, and the transitional `Entering` / `Leaving` states that
/// fire during a drag and remain on-screen at the seam. `Prepared` and
/// `Dormant` both release buffers; on the next transition back into a
/// visible state the render loop is marked dirty and the existing
/// lazy-allocation path in [`EglState::begin_frame`] reallocates.
fn apply_lifecycle_change(
    state: &mut RenderLoopState,
    egl: &mut EglState,
    surface: &mut dyn WidgetSurface,
    new_state: LifecycleState,
) {
    if state.lifecycle == Some(new_state) {
        return;
    }
    let previous = state.lifecycle;
    state.lifecycle = Some(new_state);

    match new_state {
        LifecycleState::Visible | LifecycleState::Entering | LifecycleState::Leaving => {
            tracing::info!(
                ?previous,
                ?new_state,
                "lifecycle: -> on-screen; buffers will reallocate on next frame"
            );
            surface.mark_needs_render();
            state.phase = LoopPhase::RenderPending;
        }
        LifecycleState::Dormant | LifecycleState::Prepared => {
            tracing::info!(
                ?previous,
                ?new_state,
                "lifecycle: -> off-screen; releasing available DMA-BUF export buffers"
            );
            release_released_offscreen_buffers(state, egl, surface);
        }
        _ => {
            tracing::info!(?previous, ?new_state, "lifecycle: unknown future variant");
        }
    }
}

/// Render the HH:MM:SS clock face.
fn render_clock(
    layout: &ClockLayout,
    renderer: &Renderer,
    digit_textures: &DigitTextures,
    digit_meshes: Option<&Digit3DMeshes>,
    gl: &glow::Context,
    state: &FlipState,
) {
    unsafe {
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
    }

    let mut x = layout.start_x;
    for i in 0..6 {
        if i == 2 || i == 4 {
            let colon_x = x - layout.panel_width / 2.0 - layout.gap - layout.colon_width / 2.0;

            if let Some(digit_meshes) = digit_meshes {
                render_3d_colon(renderer, digit_meshes, gl, colon_x, layout.panel_height);
            } else {
                render_2d_colon(renderer, gl, colon_x, layout.colon_width);
            }
        }

        let current_digit = state.digits[i];
        let prev_digit = state.prev_digits[i];
        let flip_progress = state.flip_progress(i);
        let digit_changed = current_digit != prev_digit;

        if let Some(digit_meshes) = digit_meshes {
            render_3d_digit(
                renderer,
                digit_meshes,
                gl,
                x,
                layout.panel_height,
                current_digit,
                prev_digit,
                digit_changed,
                flip_progress,
            );
        } else {
            render_2d_digit(
                renderer,
                digit_textures,
                gl,
                x,
                layout,
                current_digit,
                prev_digit,
                digit_changed,
                flip_progress,
            );
        }

        x += layout.panel_width + layout.gap;
        if i == 1 || i == 3 {
            x += layout.colon_width + layout.gap;
        }
    }

    unsafe {
        gl.disable(glow::BLEND);
    }
}

#[expect(clippy::too_many_arguments, reason = "render parameters")]
fn render_3d_digit(
    renderer: &Renderer,
    digit_meshes: &Digit3DMeshes,
    gl: &glow::Context,
    x: f32,
    panel_height: f32,
    current_digit: u8,
    prev_digit: u8,
    digit_changed: bool,
    flip_progress: f32,
) {
    unsafe {
        gl.enable(glow::DEPTH_TEST);
        // Cull the face pointing away from the camera across both static
        // and flip paths. GL culling is decided in screen-space winding,
        // which tracks the mesh through rotation — the back-face triangles
        // are built with reversed winding so they become screen-CCW once
        // the digit flips past edge-on, keeping the currently-visible face
        // rendered and the hidden one occluded. Without this, the back
        // face's unoccluded silhouette sliver rasterised as bright-white
        // wedges at the start of each flip.
        gl.enable(glow::CULL_FACE);
        gl.cull_face(glow::BACK);
        // Disable blending for opaque 3D geometry — inherited BLEND
        // from render_clock causes back-face fragments to bleed through
        // as lighter rectangles around the digits.
        gl.disable(glow::BLEND);
    }

    let digit_scale = panel_height * 0.9;
    let projection = renderer.projection();
    let light_dir = [-0.5, 0.5, 0.7];
    let digit_color = [1.0, 1.0, 1.0];
    let base_tilt_x = 0.3;
    let base_tilt_y = 0.2;

    let (digit, rot_angle) = if digit_changed && flip_progress < 1.0 {
        let angle = flip_progress * std::f32::consts::PI;
        if angle < std::f32::consts::FRAC_PI_2 {
            (prev_digit, -angle - base_tilt_x)
        } else {
            (current_digit, std::f32::consts::PI - angle - base_tilt_x)
        }
    } else {
        (current_digit, -base_tilt_x)
    };

    let rotation = Mat4::rotate_x(rot_angle).mul(&Mat4::rotate_y(base_tilt_y));
    let model = Mat4::translate(x, 0.0, 0.01)
        .mul(&rotation)
        .mul(&Mat4::scale(digit_scale, digit_scale, digit_scale));
    let mvp = projection.mul(&model);
    let normal_matrix = model.to_normal_matrix();
    digit_meshes.draw_digit(
        gl,
        digit,
        mvp.as_array(),
        &normal_matrix,
        digit_color,
        light_dir,
    );

    unsafe {
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.enable(glow::BLEND);
    }
}

fn render_3d_colon(
    renderer: &Renderer,
    digit_meshes: &Digit3DMeshes,
    gl: &glow::Context,
    x: f32,
    panel_height: f32,
) {
    let base_tilt_x = 0.3;
    let base_tilt_y = 0.2;
    let colon_scale = panel_height * 0.45;
    let colon_color = [
        f32::from(0xC6_u8) / 255.0,
        f32::from(0xC6_u8) / 255.0,
        f32::from(0xC6_u8) / 255.0,
    ];
    let light_dir = [-0.5, 0.5, 0.7];

    unsafe {
        gl.enable(glow::DEPTH_TEST);
        gl.enable(glow::CULL_FACE);
        gl.cull_face(glow::BACK);
        gl.disable(glow::BLEND);
    }

    let rotation = Mat4::rotate_x(-base_tilt_x).mul(&Mat4::rotate_y(base_tilt_y));
    let model = Mat4::translate(x, 0.0, 0.01)
        .mul(&rotation)
        .mul(&Mat4::scale(colon_scale, colon_scale, colon_scale));
    let mvp = renderer.projection().mul(&model);
    let normal_matrix = model.to_normal_matrix();
    digit_meshes.draw_colon(gl, mvp.as_array(), &normal_matrix, colon_color, light_dir);

    unsafe {
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.enable(glow::BLEND);
    }
}

fn render_2d_colon(renderer: &Renderer, gl: &glow::Context, x: f32, colon_width: f32) {
    let pill_width = colon_width * 0.8;
    let pill_height = pill_width * (19.74 / 19.0);
    let pill_spacing = pill_height * 1.5;
    let colon_color = [
        f32::from(0xC6_u8) / 255.0,
        f32::from(0xC6_u8) / 255.0,
        f32::from(0xC6_u8) / 255.0,
        1.0,
    ];
    renderer.draw_rounded_rect(
        gl,
        x,
        pill_spacing / 2.0,
        pill_width,
        pill_height,
        colon_color,
    );
    renderer.draw_rounded_rect(
        gl,
        x,
        -pill_spacing / 2.0,
        pill_width,
        pill_height,
        colon_color,
    );
}

#[expect(clippy::too_many_arguments, reason = "render parameters")]
#[expect(
    clippy::too_many_lines,
    reason = "2D split-flap rendering with animation"
)]
fn render_2d_digit(
    renderer: &Renderer,
    digit_textures: &DigitTextures,
    gl: &glow::Context,
    x: f32,
    layout: &ClockLayout,
    current_digit: u8,
    prev_digit: u8,
    digit_changed: bool,
    flip_progress: f32,
) {
    let half_height = layout.panel_height / 2.0;
    let split_point = 0.45;
    let digit_scale = 0.7;

    let gradient_colors = [
        [0x27_u8, 0x27, 0x27],
        [0x1B, 0x1B, 0x1B],
        [0x0F, 0x0F, 0x0F],
        [0x16, 0x16, 0x16],
        [0x1C, 0x1C, 0x1C],
        [0x18, 0x18, 0x18],
        [0x14, 0x14, 0x14],
        [0x10, 0x10, 0x10],
    ]
    .map(|[r, g, b]| {
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            1.0,
        ]
    });
    let gradient_stops = [
        1.0 - 0.25,
        1.0 - 0.4851,
        1.0 - 0.495,
        1.0 - 0.5044,
        1.0 - 0.65,
        1.0 - 0.75,
        1.0 - 0.8569,
    ];
    let frame_color = [0.15, 0.15, 0.15, 1.0];

    renderer.draw_rect(
        gl,
        x,
        0.0,
        layout.panel_width + layout.border_width * 2.0,
        layout.panel_height + layout.border_width * 2.0,
        frame_color,
    );

    if digit_changed && flip_progress < 1.0 {
        let angle = flip_progress * std::f32::consts::PI;

        renderer.draw_rect_gradient_4(
            gl,
            x,
            0.0,
            layout.panel_width,
            layout.panel_height,
            &gradient_colors,
            &gradient_stops,
        );

        renderer.draw_textured_half_rect_split(
            gl,
            x,
            -layout.gap_height / 2.0 - half_height / 2.0,
            layout.panel_width * digit_scale,
            half_height,
            digit_textures.get(current_digit),
            false,
            split_point,
        );
        renderer.draw_textured_half_rect_split(
            gl,
            x,
            layout.gap_height / 2.0 + half_height / 2.0,
            layout.panel_width * digit_scale,
            half_height,
            digit_textures.get(prev_digit),
            true,
            split_point,
        );

        if angle < std::f32::consts::FRAC_PI_2 {
            renderer.draw_flap_gradient_partial(
                gl,
                x,
                0.0,
                layout.panel_width,
                half_height,
                -angle,
                false,
                &gradient_colors,
                &gradient_stops,
                0.5,
                0.5,
                false,
            );
            renderer.draw_textured_flap_split(
                gl,
                x,
                0.0,
                layout.panel_width * digit_scale,
                half_height,
                -angle,
                false,
                digit_textures.get(prev_digit),
                false,
                false,
                split_point,
            );
        } else {
            renderer.draw_flap_gradient_partial(
                gl,
                x,
                0.0,
                layout.panel_width,
                half_height,
                -angle,
                false,
                &gradient_colors,
                &gradient_stops,
                0.0,
                0.5,
                true,
            );
            renderer.draw_textured_flap_split(
                gl,
                x,
                0.0,
                layout.panel_width * digit_scale,
                half_height,
                -angle,
                false,
                digit_textures.get(current_digit),
                true,
                true,
                split_point,
            );
        }

        renderer.draw_rect(
            gl,
            x,
            0.0,
            layout.panel_width + layout.border_width,
            layout.gap_height,
            [0.0, 0.0, 0.0, 1.0],
        );
    } else {
        renderer.draw_rect_gradient_4(
            gl,
            x,
            0.0,
            layout.panel_width,
            layout.panel_height,
            &gradient_colors,
            &gradient_stops,
        );

        renderer.draw_textured_half_rect_split(
            gl,
            x,
            layout.gap_height / 2.0 + half_height / 2.0,
            layout.panel_width * digit_scale,
            half_height,
            digit_textures.get(current_digit),
            true,
            split_point,
        );
        renderer.draw_textured_half_rect_split(
            gl,
            x,
            -layout.gap_height / 2.0 - half_height / 2.0,
            layout.panel_width * digit_scale,
            half_height,
            digit_textures.get(current_digit),
            false,
            split_point,
        );

        renderer.draw_rect(
            gl,
            x,
            0.0,
            layout.panel_width + layout.border_width,
            layout.gap_height,
            [0.0, 0.0, 0.0, 1.0],
        );
    }
}
