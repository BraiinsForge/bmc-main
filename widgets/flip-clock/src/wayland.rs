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
use crate::renderer::{Mat4, Renderer};
use anyhow::Result;
use bmc_widget::surface::{
    DeckWidgetSurfaceClient, SettingUpdate, WidgetEvent, WidgetSurface, XdgSurfaceClient,
};
use glow::HasContext;

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

    surface.request_frame();

    while surface.running() {
        surface.blocking_dispatch()?;

        // Process protocol events (timezone updates, shutdown)
        for event in surface.drain_events() {
            match event {
                WidgetEvent::Setting(SettingUpdate::Timezone(new_tz)) => {
                    tracing::info!("Timezone updated: {new_tz}");
                    timezone = new_tz;
                }
                WidgetEvent::Setting(_) | WidgetEvent::Shutdown => {}
            }
        }

        // Handle resize
        if surface.take_size_changed() {
            let (w, h) = (surface.width(), surface.height());
            surface.invalidate_cached_buffers();
            egl.resize(w, h);
            renderer.resize(w, h);
        }

        if surface.take_render_requested() {
            use chrono::Timelike;

            // Get current time in the configured timezone
            let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
            let now = chrono::Utc::now().with_timezone(&tz);

            #[expect(
                clippy::cast_possible_truncation,
                reason = "hour 0-23, minute/second 0-59 always fit in u8"
            )]
            let (hours, minutes, seconds) =
                (now.hour() as u8, now.minute() as u8, now.second() as u8);

            #[expect(
                clippy::integer_division,
                reason = "intentional truncation to milliseconds"
            )]
            let subsec = f32::from((now.nanosecond() / 1_000_000) as u16) / 1000.0;

            egl.begin_frame()?;
            egl.clear(0.0, 0.0, 0.0, 1.0);
            let gl = egl.gl();

            render_clock(
                &renderer,
                &digit_textures,
                digit_meshes.as_ref(),
                gl,
                hours,
                minutes,
                seconds,
                subsec,
                &now,
            );

            let dmabuf_info = egl.end_frame()?;
            let slot = egl.last_rendered_slot();
            surface.commit_cached_buffer(&dmabuf_info, slot, true)?;

            if surface.frame_count().is_multiple_of(60) {
                tracing::debug!("Frame {}", surface.frame_count());
            }
        }
    }

    Ok(())
}

/// Render the HH:MM:SS clock face.
#[expect(clippy::too_many_arguments, reason = "render state passed through")]
#[expect(
    clippy::too_many_lines,
    reason = "rendering logic with animation branches"
)]
fn render_clock<Tz: chrono::TimeZone>(
    renderer: &Renderer,
    digit_textures: &DigitTextures,
    digit_meshes: Option<&Digit3DMeshes>,
    gl: &glow::Context,
    hours: u8,
    minutes: u8,
    seconds: u8,
    subsec: f32,
    now: &chrono::DateTime<Tz>,
) {
    use chrono::Timelike;

    let scale_factor = 0.85;
    let panel_height = (257.0 * scale_factor) / 480.0;
    let panel_width = (200.0 * scale_factor) / 480.0;
    let colon_width = 0.05;
    let gap = 0.02;
    let total_width = 6.0 * panel_width + 2.0 * colon_width + 7.0 * gap;
    let start_x = -total_width / 2.0 + panel_width / 2.0;
    let panel_color = [0.05, 0.05, 0.1, 1.0];

    #[expect(clippy::integer_division, reason = "extracting digit values 0-9")]
    let digits: [u8; 6] = [
        hours / 10,
        hours % 10,
        minutes / 10,
        minutes % 10,
        seconds / 10,
        seconds % 10,
    ];

    let prev = now.clone() - chrono::Duration::seconds(1);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "hour 0-23, minute/second 0-59 always fit in u8"
    )]
    let (prev_hours, prev_minutes, prev_seconds) =
        (prev.hour() as u8, prev.minute() as u8, prev.second() as u8);

    #[expect(clippy::integer_division, reason = "extracting digit values 0-9")]
    let prev_digits: [u8; 6] = [
        prev_hours / 10,
        prev_hours % 10,
        prev_minutes / 10,
        prev_minutes % 10,
        prev_seconds / 10,
        prev_seconds % 10,
    ];

    let flip_duration = 0.35;
    let flip_progress_linear = (subsec / flip_duration).min(1.0);
    let flip_progress = if flip_progress_linear < 0.5 {
        4.0 * flip_progress_linear * flip_progress_linear * flip_progress_linear
    } else {
        1.0 - (-2.0 * flip_progress_linear + 2.0).powi(3) / 2.0
    };

    unsafe {
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
    }

    let mut x = start_x;
    for i in 0..6 {
        if i == 2 || i == 4 {
            let colon_x = x - panel_width / 2.0 - gap - colon_width / 2.0;
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
                colon_x,
                pill_spacing / 2.0,
                pill_width,
                pill_height,
                colon_color,
            );
            renderer.draw_rounded_rect(
                gl,
                colon_x,
                -pill_spacing / 2.0,
                pill_width,
                pill_height,
                colon_color,
            );
        }

        let current_digit = digits[i];
        let prev_digit = prev_digits[i];
        let digit_changed = current_digit != prev_digit;

        renderer.draw_rect(gl, x, 0.0, panel_width, panel_height, panel_color);

        if let Some(digit_meshes) = digit_meshes {
            render_3d_digit(
                renderer,
                digit_meshes,
                gl,
                x,
                panel_height,
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
                panel_width,
                current_digit,
                prev_digit,
                digit_changed,
                flip_progress,
            );
        }

        x += panel_width + gap;
        if i == 1 || i == 3 {
            x += colon_width + gap;
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
    }

    let digit_scale = panel_height * 1.6;
    let projection = renderer.projection();
    let light_dir = [-0.5, 0.5, 0.7];
    let digit_color = [1.0, 1.0, 1.0];
    let base_tilt_x = 0.3;
    let base_tilt_y = 0.2;

    if digit_changed && flip_progress < 1.0 {
        let angle = flip_progress * std::f32::consts::PI;
        let (digit, rot_angle) = if angle < std::f32::consts::FRAC_PI_2 {
            (prev_digit, -angle - base_tilt_x)
        } else {
            (current_digit, std::f32::consts::PI - angle - base_tilt_x)
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
    } else {
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
    }

    unsafe {
        gl.disable(glow::DEPTH_TEST);
    }
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
    panel_width: f32,
    current_digit: u8,
    prev_digit: u8,
    digit_changed: bool,
    flip_progress: f32,
) {
    let aspect_ratio = 257.0 / 200.0;
    let adjusted_panel_height = panel_width * aspect_ratio;
    let half_height = adjusted_panel_height / 2.0;
    let gap_height = panel_width * 4.0 / 200.0;
    let border_width = 0.008;
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
        panel_width + border_width * 2.0,
        adjusted_panel_height + border_width * 2.0,
        frame_color,
    );

    if digit_changed && flip_progress < 1.0 {
        let angle = flip_progress * std::f32::consts::PI;

        renderer.draw_rect_gradient_4(
            gl,
            x,
            0.0,
            panel_width,
            adjusted_panel_height,
            &gradient_colors,
            &gradient_stops,
        );

        renderer.draw_textured_half_rect_split(
            gl,
            x,
            -gap_height / 2.0 - half_height / 2.0,
            panel_width * digit_scale,
            half_height,
            digit_textures.get(current_digit),
            false,
            split_point,
        );
        renderer.draw_textured_half_rect_split(
            gl,
            x,
            gap_height / 2.0 + half_height / 2.0,
            panel_width * digit_scale,
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
                panel_width,
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
                panel_width * digit_scale,
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
                panel_width,
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
                panel_width * digit_scale,
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
            panel_width + border_width,
            gap_height,
            [0.0, 0.0, 0.0, 1.0],
        );
    } else {
        renderer.draw_rect_gradient_4(
            gl,
            x,
            0.0,
            panel_width,
            adjusted_panel_height,
            &gradient_colors,
            &gradient_stops,
        );

        renderer.draw_textured_half_rect_split(
            gl,
            x,
            gap_height / 2.0 + half_height / 2.0,
            panel_width * digit_scale,
            half_height,
            digit_textures.get(current_digit),
            true,
            split_point,
        );
        renderer.draw_textured_half_rect_split(
            gl,
            x,
            -gap_height / 2.0 - half_height / 2.0,
            panel_width * digit_scale,
            half_height,
            digit_textures.get(current_digit),
            false,
            split_point,
        );

        renderer.draw_rect(
            gl,
            x,
            0.0,
            panel_width + border_width,
            gap_height,
            [0.0, 0.0, 0.0, 1.0],
        );
    }
}
