// Copyright (C) 2026  Braiins Systems s.r.o.

// LED glow rendering as a continuous diffuse reflection on the table surface.
// Uses a triangle mesh with per-vertex colors for smooth LED-to-LED blending.

use bmc_virt_ipc::{LED_COUNT, LedState};

/// Fraction of device width the LED strip covers (centered).
const LED_STRIP_WIDTH_FRAC: f32 = 0.55;

/// Number of vertices along the strip (more = smoother color gradient).
const STRIP_VERTS: usize = 64;

/// Render LED light as a continuous diffuse glow on the table below the device.
///
/// Built as a single triangle-strip mesh with per-vertex colors interpolated
/// from the LED array, giving a perfectly smooth color gradient.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "LED color/position math on small positive values"
)]
pub fn draw_led_glow(
    painter: &egui::Painter,
    device_rect: egui::Rect,
    led_cache: &[LedState; LED_COUNT],
) {
    // Check if any LED is actually on
    let any_on = led_cache
        .iter()
        .any(|led| led.brightness > 0 && (led.r > 0 || led.g > 0 || led.b > 0));
    if !any_on {
        return;
    }

    let dev_w = device_rect.width();
    let dev_h = device_rect.height();

    let strip_width = dev_w * LED_STRIP_WIDTH_FRAC;
    let strip_left = device_rect.center().x - strip_width / 2.0;
    let table_top = device_rect.min.y + dev_h * 0.82;
    let glow_center_y = table_top + (device_rect.max.y - table_top) * 0.7;
    let glow_half_h = dev_h * 0.015;

    // Build mesh layers: wide soft outer glow + tighter bright core.
    // More layers at lower alpha = softer gaussian-like blur.
    for &(height_scale, alpha_mul) in &[
        (2.0_f32, 0.03_f32), // very wide, barely visible haze
        (1.4, 0.05),         // wide soft glow
        (1.0, 0.08),
        (0.6, 0.10),
        (0.3, 0.14), // tight bright core
    ] {
        let band_top = glow_center_y - glow_half_h * height_scale;
        let band_bot = glow_center_y + glow_half_h * height_scale;

        let mut mesh = egui::Mesh::default();
        let white_uv = egui::epaint::WHITE_UV;

        for vi in 0..STRIP_VERTS {
            let frac = vi as f32 / (STRIP_VERTS - 1) as f32;

            // Interpolate LED color at this position
            let led_pos = frac * (LED_COUNT as f32 - 1.0);
            let led_idx = (led_pos as usize).min(LED_COUNT - 2);
            let blend = led_pos - led_idx as f32;

            let led_a = &led_cache[led_idx];
            let led_b = &led_cache[led_idx + 1];
            let bright_a = f32::from(led_a.brightness) / 31.0;
            let bright_b = f32::from(led_b.brightness) / 31.0;
            let red = lerp(
                f32::from(led_a.r) * bright_a,
                f32::from(led_b.r) * bright_b,
                blend,
            );
            let green = lerp(
                f32::from(led_a.g) * bright_a,
                f32::from(led_b.g) * bright_b,
                blend,
            );
            let blue = lerp(
                f32::from(led_a.b) * bright_a,
                f32::from(led_b.b) * bright_b,
                blend,
            );

            // Scale alpha by actual LED intensity so unlit sections are transparent
            let intensity = (red + green + blue) / (255.0 * 3.0);
            let edge = (frac * 8.0).min(1.0).min(((1.0 - frac) * 8.0).min(1.0));
            let alpha = (edge * alpha_mul * intensity * 255.0) as u8;

            let color = egui::Color32::from_rgba_unmultiplied(
                red.min(255.0) as u8,
                green.min(255.0) as u8,
                blue.min(255.0) as u8,
                alpha,
            );

            let vert_x = strip_left + frac * strip_width;

            // Two vertices per column: top and bottom of the band
            let top_idx = mesh.vertices.len() as u32;
            mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(vert_x, band_top),
                uv: white_uv,
                color,
            });
            mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(vert_x, band_bot),
                uv: white_uv,
                color,
            });

            // Triangle strip: connect to previous column
            if vi > 0 {
                let prev_top = top_idx - 2;
                let prev_bot = top_idx - 1;
                mesh.indices.extend_from_slice(&[
                    prev_top,
                    prev_bot,
                    top_idx,
                    top_idx,
                    prev_bot,
                    top_idx + 1,
                ]);
            }
        }

        painter.add(egui::Shape::mesh(mesh));
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
