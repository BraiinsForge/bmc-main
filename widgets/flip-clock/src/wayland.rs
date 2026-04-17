// Copyright (C) 2025  Braiins Systems s.r.o.
//
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
use bmc_widget::surface::{
    DeckWidgetSurfaceClient, PollOutcome, SettingUpdate, WidgetEvent, WidgetSurface,
    XdgSurfaceClient,
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
///
/// Backward wall-clock jumps (common in QEMU VMs) are rejected entirely —
/// digits only advance forward. The hysteresis window also prevents
/// mid-animation resets from rapid clock oscillation.
struct FlipState {
    digits: [u8; 6],
    prev_digits: [u8; 6],
    transition_start: [Option<Instant>; 6],
    /// Total seconds since midnight of the last accepted update.
    /// `None` until the first update (accepts any initial time).
    last_total_seconds: Option<u32>,
}

/// Half a day in seconds — jumps larger than this across midnight
/// (e.g. 23:59:59 → 00:00:00) are treated as forward wraps, not
/// backward jumps.
const HALF_DAY: u32 = 43_200;

impl FlipState {
    fn new() -> Self {
        Self {
            digits: [0; 6],
            prev_digits: [0; 6],
            transition_start: [None; 6],
            last_total_seconds: None,
        }
    }

    /// Update digits from the current wall-clock time.
    /// Rejects backward clock jumps and ignores transitions while
    /// a flip animation is still running.
    fn update(&mut self, hours: u8, minutes: u8, seconds: u8) {
        let total = u32::from(hours) * 3600 + u32::from(minutes) * 60 + u32::from(seconds);

        // Reject backward clock jumps. Allow midnight wrap (large negative
        // delta means the clock crossed 00:00:00). Accept the first update
        // unconditionally.
        if let Some(prev) = self.last_total_seconds {
            let delta = total.wrapping_sub(prev);
            if delta >= HALF_DAY {
                return;
            }
        }
        self.last_total_seconds = Some(total);

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
            let animating =
                self.transition_start[i].is_some_and(|t| t.elapsed().as_secs_f32() < FLIP_DURATION);
            if new_digit != self.digits[i] && !animating {
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
    /// Waiting indefinitely for the compositor's next frame callback.
    WaitingForCallback,
    /// Idle — waiting for the next wall-clock-second boundary.
    WaitingForIdleTimeout,
}

impl LoopPhase {
    fn poll_timeout_ms(self) -> i32 {
        match self {
            Self::RenderPending => 0,
            Self::WaitingForCallback => -1,
            Self::WaitingForIdleTimeout => ms_to_next_second_boundary(),
        }
    }
}

fn ms_to_next_second_boundary() -> i32 {
    let now = std::time::SystemTime::now();
    let millis_into_second = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis())
        .unwrap_or(0);
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
    run_render_loop(&mut surface, animation_mode, timezone)
}

/// Connect in production mode (deck_widget_v1, single Wayland connection).
pub fn connect_production(
    instance_id: &str,
    animation_mode: AnimationMode,
    width: u32,
    height: u32,
    timezone: String,
) -> Result<()> {
    let mut surface = DeckWidgetSurfaceClient::connect(instance_id, width, height)?;
    tracing::info!("Connected to Wayland display (production mode)");
    run_render_loop(&mut surface, animation_mode, timezone)
}

/// Shared render loop that works with any [`WidgetSurface`] backend.
fn run_render_loop(
    surface: &mut dyn WidgetSurface,
    animation_mode: AnimationMode,
    initial_timezone: String,
) -> Result<()> {
    let mut timezone = initial_timezone;
    // Initialize GBM-based EGL
    let mut egl = EglState::new(surface.width(), surface.height())?;
    tracing::info!("GBM-based EGL initialized, starting render loop");

    // Initialize renderer with shaders
    let mut renderer = Renderer::new(egl.gl(), surface.width(), surface.height())?;
    tracing::info!("OpenGL ES renderer initialized");

    // Create digit textures (used for 2D mode)
    let digit_textures = DigitTextures::new(egl.gl())?;
    tracing::info!("Digit textures created");

    // Create 3D digit meshes (used for 3D mode)
    let digit_meshes = if animation_mode == AnimationMode::Extruded {
        Some(Digit3DMeshes::new(egl.gl())?)
    } else {
        None
    };
    if digit_meshes.is_some() {
        tracing::info!("3D digit meshes created");
    }

    let mut flip_state = FlipState::new();
    let mut phase = LoopPhase::RenderPending;

    while surface.running() {
        let outcome = surface.poll_dispatch(phase.poll_timeout_ms())?;

        // A frame callback arrived (the Dispatch impl for wl_callback.done
        // sets needs_render). Consume the flag and arm the next render.
        if surface.take_render_requested() {
            phase = LoopPhase::RenderPending;
        }

        // Idle-timeout expiry — roll into the next second's render.
        if phase == LoopPhase::WaitingForIdleTimeout && outcome == PollOutcome::Timeout {
            phase = LoopPhase::RenderPending;
        }

        // Protocol events (timezone, shutdown).
        for event in surface.drain_events() {
            match event {
                WidgetEvent::Setting(SettingUpdate::Timezone(new_tz)) => {
                    tracing::info!("Timezone updated: {new_tz}");
                    timezone = new_tz;
                    phase = LoopPhase::RenderPending;
                }
                WidgetEvent::Setting(_) | WidgetEvent::Shutdown => {}
            }
        }

        // Resize (independent of phase).
        if surface.take_size_changed() {
            let (w, h) = (surface.width(), surface.height());
            surface.invalidate_cached_buffers();
            egl.resize(w, h);
            renderer.resize(w, h);
        }

        if phase != LoopPhase::RenderPending {
            continue;
        }

        let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
        let now = chrono::Utc::now().with_timezone(&tz);

        #[expect(
            clippy::cast_possible_truncation,
            reason = "hour 0-23, minute/second 0-59 always fit in u8"
        )]
        let (hours, minutes, seconds) = (now.hour() as u8, now.minute() as u8, now.second() as u8);

        flip_state.update(hours, minutes, seconds);
        let layout = ClockLayout::for_viewport(surface.width(), surface.height());

        egl.begin_frame()?;
        egl.clear(0.0, 0.0, 0.0, 1.0);
        let gl = egl.gl();

        render_clock(
            &layout,
            &renderer,
            &digit_textures,
            digit_meshes.as_ref(),
            gl,
            &flip_state,
        );

        let animating = flip_state.is_animating();
        let (dmabuf_info, slot) = egl.end_frame()?;
        surface.submit_buffer(&dmabuf_info, slot, animating)?;

        phase = if animating {
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

/// Render the HH:MM:SS clock face.
fn render_clock(
    layout: &ClockLayout,
    renderer: &Renderer,
    digit_textures: &DigitTextures,
    digit_meshes: Option<&Digit3DMeshes>,
    gl: &glow::Context,
    state: &FlipState,
) {
    let panel_color = [0.10, 0.10, 0.18, 1.0];

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

        renderer.draw_rect(
            gl,
            x,
            0.0,
            layout.panel_width,
            layout.panel_height,
            panel_color,
        );

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

    if digit_changed && flip_progress < 1.0 {
        // No face culling during flip — the digit rotates past 90° so
        // the front face points away from the camera mid-animation.
        let angle = flip_progress * std::f32::consts::PI;
        let (digit, rot_angle) = if angle < std::f32::consts::FRAC_PI_2 {
            (prev_digit, -angle - base_tilt_x)
        } else {
            (current_digit, std::f32::consts::PI - angle - base_tilt_x)
        };
        tracing::trace!(pos = x, digit, prev_digit, current_digit, angle, "3d flip");
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
    } else {
        // Static digits: cull back faces to prevent bleed-through
        unsafe {
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
        }
        tracing::trace!(pos = x, digit = current_digit, "3d static");
        let rotation = Mat4::rotate_x(-base_tilt_x).mul(&Mat4::rotate_y(base_tilt_y));
        let model = Mat4::translate(x, 0.0, 0.01)
            .mul(&rotation)
            .mul(&Mat4::scale(digit_scale, digit_scale, digit_scale));
        let mvp = projection.mul(&model);
        let normal_matrix = model.to_normal_matrix();
        digit_meshes.draw_digit(
            gl,
            current_digit,
            mvp.as_array(),
            &normal_matrix,
            digit_color,
            light_dir,
        );
        unsafe {
            gl.disable(glow::CULL_FACE);
        }
    }

    unsafe {
        gl.disable(glow::DEPTH_TEST);
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
