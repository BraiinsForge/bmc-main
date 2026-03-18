// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! Wayland client implementation for flip-clock widget.
//!
//! Uses [`bmc_widget::surface::XdgSurfaceClient`] for Wayland connection,
//! surface management, and DMA-BUF buffer submission. This module only
//! contains the flip-clock render loop and animation logic.

use crate::AnimationMode;
use crate::digits::DigitTextures;
use crate::digits3d::Digit3DMeshes;
use crate::egl::EglState;
use crate::ipc::EventHandler;
use crate::renderer::{Mat4, Renderer};
use anyhow::Result;
use bmc_widget::surface::XdgSurfaceClient;
use bmc_widget::wayland::WidgetProtocolClient;
use chrono::{Timelike, Utc};
use chrono_tz::Tz;
use glow::HasContext;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

/// Wayland client for the flip-clock widget.
pub struct WaylandClient {
    surface: XdgSurfaceClient,
    animation_mode: AnimationMode,
    timezone: Arc<RwLock<String>>,
    shutdown: Arc<AtomicBool>,
}

impl WaylandClient {
    /// Connect to the Wayland display.
    pub fn connect(
        animation_mode: AnimationMode,
        width: u32,
        height: u32,
        timezone: Arc<RwLock<String>>,
        shutdown: Arc<AtomicBool>,
    ) -> Result<Self> {
        let surface = XdgSurfaceClient::connect(width, height, "Flip Clock", "bmc-flip-clock")?;

        Ok(Self {
            surface,
            animation_mode,
            timezone,
            shutdown,
        })
    }

    /// Run the event loop with EGL rendering.
    ///
    /// If `protocol` is provided, protocol events are polled after each frame.
    #[expect(clippy::too_many_lines, reason = "rendering loop with setup code")]
    pub fn run(
        &mut self,
        mut protocol: Option<(WidgetProtocolClient, EventHandler)>,
    ) -> Result<()> {
        let state = self.surface.state();

        // Initialize GBM-based EGL
        let mut egl = EglState::new(state.width, state.height)?;

        tracing::info!("GBM-based EGL initialized, starting render loop");

        // Initialize renderer with shaders
        let mut renderer = Renderer::new(egl.gl(), state.width, state.height)?;

        tracing::info!("OpenGL ES renderer initialized");

        // Create digit textures (used for 2D mode)
        let digit_textures = DigitTextures::new(egl.gl())?;
        tracing::info!("Digit textures created");

        // Create 3D digit meshes (used for 3D mode)
        let digit_meshes = if self.animation_mode == AnimationMode::Extruded {
            Some(Digit3DMeshes::new(egl.gl())?)
        } else {
            None
        };
        if digit_meshes.is_some() {
            tracing::info!("3D digit meshes created");
        }

        // Request first frame callback
        self.surface.request_frame();

        while self.surface.state().running {
            // Dispatch Wayland events
            self.surface.blocking_dispatch()?;

            // Poll protocol events if connected
            if let Some((ref mut client, ref mut handler)) = protocol
                && client.poll_events().is_ok()
            {
                client.process_events(handler);
            }

            // Check shutdown flag (set by protocol handler)
            if self.shutdown.load(Ordering::Relaxed) {
                tracing::info!("Shutdown requested via protocol");
                break;
            }

            // Handle resize if needed
            if self.surface.state().size_changed {
                let (w, h) = (self.surface.state().width, self.surface.state().height);
                self.surface.invalidate_cached_buffers();
                egl.resize(w, h);
                renderer.resize(w, h);
                self.surface.state_mut().size_changed = false;
            }

            let state = self.surface.state();

            // Render if we got a frame callback
            if state.needs_render {
                self.surface.state_mut().needs_render = false;

                // Get current time in the configured timezone
                let tz_str = self
                    .timezone
                    .read()
                    .expect("BUG: timezone lock poisoned")
                    .clone();
                let tz: Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
                let now = Utc::now().with_timezone(&tz);

                // Timelike::hour() returns 0-23, minute()/second() return 0-59
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "hour 0-23, minute/second 0-59 always fit in u8"
                )]
                let (hours, minutes, seconds) =
                    (now.hour() as u8, now.minute() as u8, now.second() as u8);

                // nanosecond() / 1_000_000 gives 0-999 which fits in u16
                #[expect(
                    clippy::integer_division,
                    reason = "intentional truncation to milliseconds"
                )]
                let subsec = f32::from((now.nanosecond() / 1_000_000) as u16) / 1000.0;

                // Begin frame
                egl.begin_frame()?;

                // Clear to black background
                egl.clear(0.0, 0.0, 0.0, 1.0);

                // Get GL context for rendering
                let gl = egl.gl();

                // Compositor advertises 1280x480 (landscape) to widgets
                // We render a normal horizontal HH:MM:SS layout
                // Compositor handles rotation to physical 480x1280 portrait panel
                //
                // Coordinate system: width=1.0, height scales by aspect ratio
                // For 1280x480: aspect = 2.67, so height = 1.0 / 2.67 = 0.375 units

                // Draw 6 digit panels (HH:MM:SS layout) - horizontal row
                // Panel dimensions: 200x257 pixels (design spec for HH:MM)
                // Scale down 85% to fit 6 digits (HH:MM:SS) without squishing
                let scale_factor = 0.85;
                let panel_height = (257.0 * scale_factor) / 480.0;
                let panel_width = (200.0 * scale_factor) / 480.0;

                let colon_width = 0.05; // Width allocated for colon
                let gap = 0.02; // Small gap between panels

                // Total width: 6 panels + 2 colons + gaps
                let total_width = 6.0 * panel_width + 2.0 * colon_width + 7.0 * gap;
                let start_x = -total_width / 2.0 + panel_width / 2.0;

                // Colors for digit panels (dark to match background)
                let panel_color = [0.05, 0.05, 0.1, 1.0];

                // Digit values: HH:MM:SS
                #[expect(clippy::integer_division, reason = "extracting digit values 0-9")]
                let digits: [u8; 6] = [
                    hours / 10,
                    hours % 10,
                    minutes / 10,
                    minutes % 10,
                    seconds / 10,
                    seconds % 10,
                ];

                // Previous digit values (for flip animation when digit changes)
                // Calculate what the previous second's digits would be
                let prev = now - chrono::Duration::seconds(1);
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

                // Animation progress: flip happens in first 0.35 seconds after digit change
                let flip_duration = 0.35;
                let flip_progress_linear = (subsec / flip_duration).min(1.0);

                // Add easing for smooth, natural motion (ease-in-out cubic)
                // Accelerates at start, decelerates at end like a real mechanical flip
                let flip_progress = if flip_progress_linear < 0.5 {
                    4.0 * flip_progress_linear * flip_progress_linear * flip_progress_linear
                } else {
                    1.0 - (-2.0 * flip_progress_linear + 2.0).powi(3) / 2.0
                };

                // Enable blending for digit textures (white on transparent)
                unsafe {
                    gl.enable(glow::BLEND);
                    gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                }

                // Draw digits in a horizontal row (left to right: HH:MM:SS)
                let mut x = start_x;
                for i in 0..6 {
                    // Add colon after HH (index 2) and MM (index 4)
                    if i == 2 || i == 4 {
                        // Draw colon - two rounded rectangles stacked vertically
                        // From SVG: 19x60 viewbox, two pills at y=0-19.74 and y=40.26-60
                        let colon_x = x - panel_width / 2.0 - gap - colon_width / 2.0;

                        // Scale to match our coordinate system
                        // SVG: 19 wide, each pill ~19.74 tall
                        let pill_width = colon_width * 0.8;
                        let pill_height = pill_width * (19.74 / 19.0);

                        // SVG spacing: top pill at 9.87, bottom pill at 50.13 (center positions)
                        // Total height 60, so normalized: top at 0.165, bottom at -0.165
                        let pill_spacing = pill_height * 1.5;

                        let colon_color = [
                            f32::from(0xC6_u8) / 255.0,
                            f32::from(0xC6_u8) / 255.0,
                            f32::from(0xC6_u8) / 255.0,
                            1.0,
                        ]; // #C6C6C6

                        // Upper pill
                        renderer.draw_rounded_rect(
                            gl,
                            colon_x,
                            pill_spacing / 2.0,
                            pill_width,
                            pill_height,
                            colon_color,
                        );
                        // Lower pill
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

                    // Animate any digit that changed
                    let digit_changed = current_digit != prev_digit;

                    // Draw panel background
                    renderer.draw_rect(gl, x, 0.0, panel_width, panel_height, panel_color);

                    // Render digit based on animation mode
                    if let Some(ref digit_meshes) = digit_meshes {
                        // 3D extruded mode
                        unsafe {
                            gl.enable(glow::DEPTH_TEST);
                        }

                        let digit_scale = panel_height * 1.6;
                        let projection = renderer.projection();
                        let light_dir = [-0.5, 0.5, 0.7];
                        let digit_color = [1.0, 1.0, 1.0];

                        // Base tilt to show 3D depth (gray side faces visible)
                        let base_tilt_x = 0.3;
                        let base_tilt_y = 0.2;

                        if digit_changed && flip_progress < 1.0 {
                            // 3D flip animation: rotate whole digit around X axis
                            let angle = flip_progress * std::f32::consts::PI;

                            if angle < std::f32::consts::FRAC_PI_2 {
                                // Old digit rotating away
                                let rotation = Mat4::rotate_x(-angle - base_tilt_x)
                                    .mul(&Mat4::rotate_y(base_tilt_y));
                                let model = Mat4::translate(x, 0.0, 0.01)
                                    .mul(&rotation)
                                    .mul(&Mat4::scale(digit_scale, digit_scale, digit_scale));
                                let mvp = projection.mul(&model);
                                let normal_matrix = model.to_normal_matrix();

                                digit_meshes.draw_digit(
                                    gl,
                                    prev_digit,
                                    mvp.as_array(),
                                    &normal_matrix,
                                    digit_color,
                                    light_dir,
                                );
                            } else {
                                // New digit rotating in
                                let rotation =
                                    Mat4::rotate_x(std::f32::consts::PI - angle - base_tilt_x)
                                        .mul(&Mat4::rotate_y(base_tilt_y));
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
                        } else {
                            // Static digit with tilt
                            let rotation =
                                Mat4::rotate_x(-base_tilt_x).mul(&Mat4::rotate_y(base_tilt_y));
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
                    } else {
                        // 2D flat mode - classic split-flap display design
                        // Panel aspect ratio: 200x257 (width x height)
                        let aspect_ratio = 257.0 / 200.0;
                        let adjusted_panel_height = panel_width * aspect_ratio;
                        let half_height = adjusted_panel_height / 2.0;

                        // Dividing line: 4px converted to normalized coordinates
                        // Assuming panel_width corresponds to ~200px, 4px = 4/200 = 0.02 of panel_width
                        let gap_height = panel_width * 4.0 / 200.0;
                        let border_width = 0.008; // thin border around panel

                        // Split point for digit texture (< 0.5 makes top smaller)
                        let split_point = 0.45;

                        // 8-stop gradient for ultra-smooth transitions
                        // Base: #272727 (0%) -> #0F0F0F (48.51%) -> #1C1C1C (50.44%) -> #101010 (85.69%)
                        // With interpolated stops for smoothness
                        let gradient_colors = [
                            [
                                f32::from(0x27_u8) / 255.0,
                                f32::from(0x27_u8) / 255.0,
                                f32::from(0x27_u8) / 255.0,
                                1.0,
                            ], // 0%: #272727
                            [
                                f32::from(0x1B_u8) / 255.0,
                                f32::from(0x1B_u8) / 255.0,
                                f32::from(0x1B_u8) / 255.0,
                                1.0,
                            ], // 25%: interpolated
                            [
                                f32::from(0x0F_u8) / 255.0,
                                f32::from(0x0F_u8) / 255.0,
                                f32::from(0x0F_u8) / 255.0,
                                1.0,
                            ], // 48.51%: #0F0F0F
                            [
                                f32::from(0x16_u8) / 255.0,
                                f32::from(0x16_u8) / 255.0,
                                f32::from(0x16_u8) / 255.0,
                                1.0,
                            ], // 49.5%: interpolated
                            [
                                f32::from(0x1C_u8) / 255.0,
                                f32::from(0x1C_u8) / 255.0,
                                f32::from(0x1C_u8) / 255.0,
                                1.0,
                            ], // 50.44%: #1C1C1C
                            [
                                f32::from(0x18_u8) / 255.0,
                                f32::from(0x18_u8) / 255.0,
                                f32::from(0x18_u8) / 255.0,
                                1.0,
                            ], // 65%: interpolated
                            [
                                f32::from(0x14_u8) / 255.0,
                                f32::from(0x14_u8) / 255.0,
                                f32::from(0x14_u8) / 255.0,
                                1.0,
                            ], // 75%: interpolated
                            [
                                f32::from(0x10_u8) / 255.0,
                                f32::from(0x10_u8) / 255.0,
                                f32::from(0x10_u8) / 255.0,
                                1.0,
                            ], // 85.69%: #101010
                        ];
                        let gradient_stops = [
                            1.0 - 0.25,
                            1.0 - 0.4851,
                            1.0 - 0.495,
                            1.0 - 0.5044,
                            1.0 - 0.65,
                            1.0 - 0.75,
                            1.0 - 0.8569,
                        ];

                        let frame_color = [0.15, 0.15, 0.15, 1.0]; // dark gray frame

                        // Draw panel frame (border around entire digit)
                        renderer.draw_rect(
                            gl,
                            x,
                            0.0,
                            panel_width + border_width * 2.0,
                            adjusted_panel_height + border_width * 2.0,
                            frame_color,
                        );

                        if digit_changed && flip_progress < 1.0 {
                            // Split-flap animation in progress
                            let angle = flip_progress * std::f32::consts::PI;

                            // Draw ONE full-height background panel with 4-stop gradient
                            renderer.draw_rect_gradient_4(
                                gl,
                                x,
                                0.0, // centered
                                panel_width,
                                adjusted_panel_height, // full height
                                &gradient_colors,
                                &gradient_stops,
                            );

                            // Digit texture aspect ratio: 128:256 = 1:2
                            // Scale digit width to maintain proportions (narrower than panel)
                            let digit_scale = 0.7; // make digits narrower

                            // Draw static bottom half digit (NEW digit revealed as flap falls)
                            renderer.draw_textured_half_rect_split(
                                gl,
                                x,
                                -gap_height / 2.0 - half_height / 2.0,
                                panel_width * digit_scale,
                                half_height,
                                digit_textures.get(current_digit),
                                false, // bottom half of texture
                                split_point,
                            );

                            // Draw static top half digit (OLD digit, stays until flap covers)
                            renderer.draw_textured_half_rect_split(
                                gl,
                                x,
                                gap_height / 2.0 + half_height / 2.0,
                                panel_width * digit_scale,
                                half_height,
                                digit_textures.get(prev_digit),
                                true, // top half of texture
                                split_point,
                            );

                            // Flipping flap with gradient and digit
                            if angle < std::f32::consts::FRAC_PI_2 {
                                // First half: show top portion of gradient (50% to 100%)
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
                                    0.5,   // start at 50% of gradient (center)
                                    0.5,   // show top 50% (from center to top)
                                    false, // normal gradient direction
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
                                    false, // bottom half of texture
                                    false,
                                    split_point,
                                );
                            } else {
                                // Second half: show bottom portion of gradient (0% to 50%)
                                // Flip gradient because we're viewing the back of the flap
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
                                    0.0,  // start at 0% of gradient (bottom)
                                    0.5,  // show bottom 50% (from bottom to center)
                                    true, // flip gradient (viewing back of flap)
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
                                    true, // top half of texture
                                    true, // flip vertically
                                    split_point,
                                );
                            }

                            // Draw center split line (visible gap between halves)
                            renderer.draw_rect(
                                gl,
                                x,
                                0.0,
                                panel_width + border_width,
                                gap_height,
                                [0.0, 0.0, 0.0, 1.0], // black gap
                            );
                        } else {
                            // Static digit - draw ONE full background with 4-stop gradient
                            renderer.draw_rect_gradient_4(
                                gl,
                                x,
                                0.0, // centered
                                panel_width,
                                adjusted_panel_height, // full height
                                &gradient_colors,
                                &gradient_stops,
                            );

                            // Digit texture aspect ratio: 128:256 = 1:2
                            // Scale digit width to maintain proportions
                            let digit_scale = 0.7;

                            // Draw top half of digit
                            renderer.draw_textured_half_rect_split(
                                gl,
                                x,
                                gap_height / 2.0 + half_height / 2.0,
                                panel_width * digit_scale,
                                half_height,
                                digit_textures.get(current_digit),
                                true, // top half of texture
                                split_point,
                            );

                            // Draw bottom half of digit
                            renderer.draw_textured_half_rect_split(
                                gl,
                                x,
                                -gap_height / 2.0 - half_height / 2.0,
                                panel_width * digit_scale,
                                half_height,
                                digit_textures.get(current_digit),
                                false, // bottom half of texture
                                split_point,
                            );

                            // Center split line (4px gap)
                            renderer.draw_rect(
                                gl,
                                x,
                                0.0,
                                panel_width + border_width,
                                gap_height,
                                [0.0, 0.0, 0.0, 1.0], // black gap
                            );
                        }
                    }

                    x += panel_width + gap;
                    if i == 1 || i == 3 {
                        x += colon_width + gap; // extra gap for colon
                    }
                }

                // Disable blending
                unsafe {
                    gl.disable(glow::BLEND);
                }

                // End frame and submit via cached buffer path
                let dmabuf_info = egl.end_frame()?;
                let slot = egl.last_rendered_slot();
                self.surface
                    .commit_cached_buffer(&dmabuf_info, slot, true)?;

                let state = self.surface.state();
                if state.frame_count.is_multiple_of(60) {
                    tracing::debug!("Frame {}", state.frame_count);
                }
            }
        }

        Ok(())
    }
}
