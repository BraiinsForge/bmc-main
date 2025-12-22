// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! Wayland client implementation for flip-clock widget
//!
//! Handles connection to the Wayland compositor, surface creation,
//! and frame callback management.

use crate::AnimationMode;
use crate::digits::DigitTextures;
use crate::digits3d::Digit3DMeshes;
use crate::egl::{DmaBufInfo, EglState};
use crate::renderer::{Mat4, Renderer};
use anyhow::{Context, Result};
use glow::HasContext;
use std::os::fd::AsFd;
use std::time::{SystemTime, UNIX_EPOCH};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// Wayland client state
pub struct WaylandClient {
    /// Wayland connection
    #[expect(dead_code, reason = "kept alive for protocol operations")]
    conn: Connection,
    /// Event queue
    queue: EventQueue<WaylandState>,
    /// Client state
    state: WaylandState,
    /// Animation mode
    animation_mode: AnimationMode,
}

/// Internal state for Wayland protocol handling
#[expect(
    clippy::struct_excessive_bools,
    reason = "simple state flags for protocol handling"
)]
struct WaylandState {
    /// Whether we should keep running
    running: bool,
    /// Compositor global
    compositor: Option<wl_compositor::WlCompositor>,
    /// XDG shell global
    xdg_wm_base: Option<xdg_wm_base::XdgWmBase>,
    /// Linux DMA-BUF global
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    /// Our surface
    surface: Option<wl_surface::WlSurface>,
    /// XDG surface wrapper
    xdg_surface: Option<xdg_surface::XdgSurface>,
    /// Toplevel role
    xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    /// Whether the surface is configured
    configured: bool,
    /// Current width
    width: u32,
    /// Current height
    height: u32,
    /// Frame count (for animation)
    frame_count: u32,
    /// Whether we need to render
    needs_render: bool,
    /// Whether size changed (needs EGL resize)
    size_changed: bool,
}

impl WaylandClient {
    /// Connect to the Wayland display
    pub fn connect(animation_mode: AnimationMode) -> Result<Self> {
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;

        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = WaylandState {
            running: true,
            compositor: None,
            xdg_wm_base: None,
            linux_dmabuf: None,
            surface: None,
            xdg_surface: None,
            xdg_toplevel: None,
            configured: false,
            width: 640,
            height: 480,
            frame_count: 0,
            needs_render: false,
            size_changed: false,
        };

        // Roundtrip to get globals
        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

        // Verify we have required globals
        let compositor = state
            .compositor
            .as_ref()
            .context("Compositor not available")?;
        let xdg_wm_base = state
            .xdg_wm_base
            .as_ref()
            .context("XDG shell not available")?;

        // Create surface
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &qh, ());
        let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());

        xdg_toplevel.set_title("Flip Clock".to_owned());
        xdg_toplevel.set_app_id("bmc-flip-clock".to_owned());

        // Commit to trigger configure
        surface.commit();

        state.surface = Some(surface);
        state.xdg_surface = Some(xdg_surface);
        state.xdg_toplevel = Some(xdg_toplevel);

        // Wait for configure
        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for configure")?;

        tracing::info!("Surface configured: {}x{}", state.width, state.height);

        Ok(Self {
            conn,
            queue,
            state,
            animation_mode,
        })
    }

    /// Run the event loop with EGL rendering
    #[expect(clippy::too_many_lines, reason = "rendering loop with setup code")]
    pub fn run(&mut self) -> Result<()> {
        let qh = self.queue.handle();

        // Initialize GBM-based EGL
        let mut egl = EglState::new(self.state.width, self.state.height)?;

        tracing::info!("GBM-based EGL initialized, starting render loop");

        // Initialize renderer with shaders
        let mut renderer = Renderer::new(egl.gl(), self.state.width, self.state.height)?;

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

        // Verify we have linux-dmabuf
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 not available")?
            .clone();

        // Request first frame callback
        if let Some(ref surface) = self.state.surface {
            surface.frame(&qh, ());
            surface.commit();
        }

        while self.state.running {
            // Dispatch Wayland events
            self.queue
                .blocking_dispatch(&mut self.state)
                .context("Wayland dispatch failed")?;

            // Handle resize if needed
            if self.state.size_changed {
                egl.resize(self.state.width, self.state.height);
                renderer.resize(self.state.width, self.state.height);
                self.state.size_changed = false;
            }

            // Render if we got a frame callback
            if self.state.needs_render {
                self.state.needs_render = false;

                // Get actual system time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("BUG: system time before UNIX epoch");
                let total_secs = now.as_secs();
                // subsec_millis returns 0-999, which fits in u16 and f32 without precision loss
                #[expect(clippy::cast_possible_truncation, reason = "0-999 fits in u16")]
                let subsec = f32::from(now.subsec_millis() as u16) / 1000.0;

                // Current time components (modulo values always fit in u8)
                #[expect(clippy::integer_division, reason = "time calculation")]
                let (hours, minutes, seconds) = (
                    ((total_secs / 3600) % 24) as u8,
                    ((total_secs / 60) % 60) as u8,
                    (total_secs % 60) as u8,
                );

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
                let prev_total_secs = if total_secs > 0 { total_secs - 1 } else { 0 };
                #[expect(clippy::integer_division, reason = "time calculation")]
                let (prev_hours, prev_minutes, prev_seconds) = (
                    ((prev_total_secs / 3600) % 24) as u8,
                    ((prev_total_secs / 60) % 60) as u8,
                    (prev_total_secs % 60) as u8,
                );

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
                                    0.5, // start at 50% of gradient (center)
                                    0.5, // show top 50% (from center to top)
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
                                    0.0, // start at 0% of gradient (bottom)
                                    0.5, // show bottom 50% (from bottom to center)
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

                // End frame and get DMA-BUF
                let dmabuf_info = egl.end_frame()?;

                // Create wl_buffer from DMA-BUF
                let buffer = create_buffer_from_dmabuf(&linux_dmabuf, &dmabuf_info, &qh);

                // Attach buffer to surface
                if let Some(ref surface) = self.state.surface {
                    surface.attach(Some(&buffer), 0, 0);
                    #[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
                    surface.damage_buffer(
                        0,
                        0,
                        dmabuf_info.width as i32,
                        dmabuf_info.height as i32,
                    );
                    surface.frame(&qh, ());
                    surface.commit();
                }

                if self.state.frame_count % 60 == 0 {
                    tracing::debug!("Frame {}", self.state.frame_count);
                }
            }
        }

        Ok(())
    }

    /// Get surface dimensions
    #[expect(dead_code)]
    pub fn dimensions(&self) -> (u32, u32) {
        (self.state.width, self.state.height)
    }
}

/// Create a wl_buffer from DMA-BUF info using linux-dmabuf protocol
fn create_buffer_from_dmabuf(
    linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    info: &DmaBufInfo,
    qh: &QueueHandle<WaylandState>,
) -> wl_buffer::WlBuffer {
    // Create buffer params
    let params = linux_dmabuf.create_params(qh, ());

    // Add the plane (single plane for XRGB8888)
    let modifier: u64 = info.modifier.into();
    let modifier_hi = (modifier >> 32) as u32;
    let modifier_lo = (modifier & 0xFFFF_FFFF) as u32;

    params.add(
        info.fd.as_fd(),
        0, // plane index
        0, // offset
        info.stride,
        modifier_hi,
        modifier_lo,
    );

    // Create buffer immediately (synchronous)
    #[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
    params.create_immed(
        info.width as i32,
        info.height as i32,
        info.format as u32,
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    )
}

// === Protocol Implementations ===

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
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
                    let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version.min(6),
                        qh,
                        (),
                    );
                    tracing::debug!("Bound wl_compositor v{}", version.min(6));
                    state.compositor = Some(compositor);
                }
                "xdg_wm_base" => {
                    let xdg_wm_base =
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, version.min(6), qh, ());
                    tracing::debug!("Bound xdg_wm_base v{}", version.min(6));
                    state.xdg_wm_base = Some(xdg_wm_base);
                }
                "zwp_linux_dmabuf_v1" => {
                    let dmabuf = registry.bind::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    tracing::debug!("Bound zwp_linux_dmabuf_v1 v{}", version.min(4));
                    state.linux_dmabuf = Some(dmabuf);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // No events for wl_compositor
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Surface events (enter/leave output) - not needed for now
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WaylandState {
    fn event(
        _: &mut Self,
        xdg_wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            xdg_wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for WaylandState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
            tracing::debug!("XDG surface configured");
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 && height > 0 {
                    #[expect(
                        clippy::cast_sign_loss,
                        reason = "width/height are positive after check"
                    )]
                    {
                        let new_width = width as u32;
                        let new_height = height as u32;
                        if new_width != state.width || new_height != state.height {
                            state.width = new_width;
                            state.height = new_height;
                            state.size_changed = true;
                        }
                    }
                    tracing::debug!("Toplevel configured: {}x{}", width, height);
                }
            }
            xdg_toplevel::Event::Close => {
                tracing::info!("Close requested");
                state.running = false;
            }
            xdg_toplevel::Event::ConfigureBounds { .. }
            | xdg_toplevel::Event::WmCapabilities { .. }
            | _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frame_count = state.frame_count.wrapping_add(1);
            state.needs_render = true;
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        event: zwp_linux_dmabuf_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_linux_dmabuf_v1::Event::Format { format } => {
                tracing::trace!("DMA-BUF format supported: 0x{:08x}", format);
            }
            zwp_linux_dmabuf_v1::Event::Modifier {
                format,
                modifier_hi,
                modifier_lo,
            } => {
                let modifier = (u64::from(modifier_hi) << 32) | u64::from(modifier_lo);
                tracing::trace!(
                    "DMA-BUF format 0x{:08x} with modifier 0x{:016x}",
                    format,
                    modifier
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_linux_buffer_params_v1::Event::Failed = event {
            tracing::error!("DMA-BUF buffer creation failed");
        }
        // Other events (Created) are for async path - we use create_immed
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for WaylandState {
    fn event(
        _: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            // Buffer is no longer in use by the compositor
            // We can reuse or destroy it
            buffer.destroy();
        }
    }
}
