// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Test renderer for WASM widget - bypasses WASM loading and renders test content
//! directly using FemtoVG to verify the rendering pipeline works correctly.

use bmc_render::colors::*;
use bmc_render::renderer::Renderer;

/// Render a test frame showcasing all primitives.
///
/// This function demonstrates all the rendering capabilities that the WASM runtime
/// exposes to guest modules, helping diagnose rendering pipeline issues.
pub fn render_test_frame(renderer: &mut dyn Renderer, frame: u32, width: u32, height: u32) {
    let w = width as f32;
    let h = height as f32;

    // Background
    renderer.fill_rect(0.0, 0.0, w, h, Color::from_hex(0x1A_1A_2E));

    // Title
    renderer.draw_text("WASM Runtime Test Mode", 20.0, 15.0, 24.0, GRAY_10);
    renderer.draw_text(
        &format!("Frame: {} | {}x{}", frame, width, height),
        20.0,
        45.0,
        14.0,
        GRAY_50,
    );

    // Divider line
    renderer.draw_line(20.0, 70.0, w - 20.0, 70.0, 2.0, GRAY_70);

    // Section 1: Basic Shapes (left)
    render_shapes_section(renderer, 20.0, 90.0);

    // Section 2: Text rendering (center-left)
    render_text_section(renderer, 280.0, 90.0);

    // Section 3: Color palette (center-right)
    render_colors_section(renderer, 540.0, 90.0);

    // Section 4: Animation (right)
    render_animation_section(renderer, 800.0, 90.0, frame);

    // Section 5: Transforms (far right)
    render_transforms_section(renderer, 1050.0, 90.0, frame);
}

// Commented out for minimal test - uncomment to enable full test suite
#[allow(dead_code)]
fn render_shapes_section(renderer: &mut dyn Renderer, x: f32, y: f32) {
    renderer.draw_text("Shapes", x, y, 18.0, GRAY_10);

    let mut cy = y + 30.0;

    // fill_rect
    renderer.draw_text("fill_rect", x, cy, 12.0, GRAY_50);
    renderer.fill_rect(x + 100.0, cy - 2.0, 60.0, 20.0, VIOLET_50);
    cy += 35.0;

    // fill_rounded_rect
    renderer.draw_text("rounded_rect", x, cy, 12.0, GRAY_50);
    renderer.fill_rounded_rect(x + 100.0, cy - 2.0, 60.0, 20.0, 6.0, GREEN_50);
    cy += 35.0;

    // fill_circle
    renderer.draw_text("fill_circle", x, cy, 12.0, GRAY_50);
    renderer.fill_circle(x + 130.0, cy + 8.0, 12.0, RED_50);
    cy += 35.0;

    // stroke_rect
    renderer.draw_text("stroke_rect", x, cy, 12.0, GRAY_50);
    renderer.stroke_rect(x + 100.0, cy - 2.0, 60.0, 20.0, 2.0, ORANGE_50);
    cy += 35.0;

    // draw_line
    renderer.draw_text("draw_line", x, cy, 12.0, GRAY_50);
    renderer.draw_line(x + 100.0, cy + 8.0, x + 160.0, cy + 8.0, 3.0, GRAY_30);
    renderer.draw_line(x + 100.0, cy + 3.0, x + 160.0, cy + 13.0, 2.0, VIOLET_50);
}

#[allow(dead_code)]
fn render_text_section(renderer: &mut dyn Renderer, x: f32, y: f32) {
    renderer.draw_text("Text", x, y, 18.0, GRAY_10);

    let mut cy = y + 30.0;

    // Different sizes
    renderer.draw_text("Size 10", x, cy, 10.0, GRAY_30);
    cy += 18.0;
    renderer.draw_text("Size 14", x, cy, 14.0, GRAY_30);
    cy += 22.0;
    renderer.draw_text("Size 20", x, cy, 20.0, GRAY_30);
    cy += 30.0;
    renderer.draw_text("Size 28", x, cy, 28.0, GRAY_30);
    cy += 40.0;

    // Colored text
    renderer.draw_text("Violet", x, cy, 16.0, VIOLET_50);
    renderer.draw_text("Green", x + 60.0, cy, 16.0, GREEN_50);
    renderer.draw_text("Red", x + 120.0, cy, 16.0, RED_50);
    cy += 30.0;

    // Long text
    renderer.draw_text("Hello, WASM World!", x, cy, 14.0, GRAY_10);
}

#[allow(dead_code)]
fn render_colors_section(renderer: &mut dyn Renderer, x: f32, y: f32) {
    renderer.draw_text("Colors", x, y, 18.0, GRAY_10);

    let size = 28.0;
    let gap = 4.0;
    let mut cy = y + 30.0;

    // Gray palette
    renderer.draw_text("Grays", x, cy, 12.0, GRAY_50);
    cy += 18.0;
    let grays = [GRAY_10, GRAY_30, GRAY_50, GRAY_70, GRAY_90];
    for (i, &color) in grays.iter().enumerate() {
        renderer.fill_rect(x + (i as f32) * (size + gap), cy, size, size, color);
    }
    cy += size + 15.0;

    // Brand colors
    renderer.draw_text("Brand", x, cy, 12.0, GRAY_50);
    cy += 18.0;
    let brand = [VIOLET_50, GREEN_50, RED_50, ORANGE_50];
    for (i, &color) in brand.iter().enumerate() {
        renderer.fill_rect(x + (i as f32) * (size + gap), cy, size, size, color);
    }
    cy += size + 15.0;

    // Alpha blending test
    renderer.draw_text("Alpha", x, cy, 12.0, GRAY_50);
    cy += 18.0;
    // Background stripe
    renderer.fill_rect(x, cy, 150.0, size, GRAY_30);
    // Overlapping semi-transparent rects
    renderer.fill_rect(
        x + 10.0,
        cy + 4.0,
        40.0,
        20.0,
        Color::from_hex(0x9B_59_B6).with_alpha(0.8),
    ); // violet with alpha
    renderer.fill_rect(
        x + 40.0,
        cy + 4.0,
        40.0,
        20.0,
        Color::from_hex(0x2E_CC_71).with_alpha(0.8),
    ); // green with alpha
    renderer.fill_rect(
        x + 70.0,
        cy + 4.0,
        40.0,
        20.0,
        Color::from_hex(0xE7_4C_3C).with_alpha(0.8),
    ); // red with alpha
}

#[allow(dead_code)]
fn render_animation_section(renderer: &mut dyn Renderer, x: f32, y: f32, frame: u32) {
    renderer.draw_text("Animation", x, y, 18.0, GRAY_10);

    let cy = y + 30.0;

    // Bouncing circle
    let bounce_y = cy + 60.0 + (((frame as f32) * 0.1).sin() * 40.0);
    renderer.fill_circle(x + 60.0, bounce_y, 15.0, VIOLET_50);

    // Color cycling rectangle
    let hue_shift = (frame % 360) as f32;
    let color = Color::from_hsv(hue_shift, 0.7, 0.9);
    renderer.fill_rounded_rect(x + 100.0, cy + 40.0, 80.0, 40.0, 8.0, color);

    // Pulsing circle
    let pulse = 10.0 + (((frame as f32) * 0.15).sin() * 5.0);
    renderer.fill_circle(x + 60.0, cy + 160.0, pulse, GREEN_50);

    // Progress bar
    let progress = ((frame % 200) as f32) / 200.0;
    renderer.fill_rounded_rect(x, cy + 200.0, 180.0, 12.0, 4.0, GRAY_70);
    renderer.fill_rounded_rect(x, cy + 200.0, 180.0 * progress, 12.0, 4.0, VIOLET_50);

    // Frame counter
    renderer.draw_text(&format!("Frame: {}", frame), x, cy + 230.0, 14.0, GRAY_30);
}

#[allow(dead_code)]
fn render_transforms_section(renderer: &mut dyn Renderer, x: f32, y: f32, frame: u32) {
    renderer.draw_text("Transforms", x, y, 18.0, GRAY_10);

    let cy = y + 60.0;
    let cx = x + 80.0;

    // Rotating square using save/restore and rotate
    renderer.save();
    renderer.translate(cx, cy);
    let angle = (frame as f32) * 0.05;
    renderer.rotate(angle);
    renderer.fill_rect(-20.0, -20.0, 40.0, 40.0, RED_50);
    renderer.restore();

    // Second rotating element (opposite direction)
    renderer.save();
    renderer.translate(cx, cy + 100.0);
    renderer.rotate(-angle * 0.7);
    renderer.fill_rounded_rect(-25.0, -15.0, 50.0, 30.0, 6.0, GREEN_50);
    renderer.restore();

    // Scissor clipping demo
    let clip_y = cy + 160.0;
    renderer.draw_text("Scissor clip:", x, clip_y, 12.0, GRAY_50);
    renderer.push_scissor(x, clip_y + 18.0, 100.0, 30.0);
    // This text would be longer but gets clipped
    renderer.draw_text(
        "This text is clipped by scissor!",
        x,
        clip_y + 20.0,
        14.0,
        ORANGE_50,
    );
    renderer.fill_rect(x, clip_y + 40.0, 200.0, 20.0, VIOLET_50); // Also clipped
    renderer.pop_scissor();

    // Show clip boundary
    renderer.stroke_rect(x, clip_y + 18.0, 100.0, 30.0, 1.0, GRAY_50);
}

// hsv_to_rgb removed — use Color::from_hsv() once added to the Color type
