// Copyright (C) 2025  Braiins Systems s.r.o.

//! Shape drawing primitives using tiny-skia.

#![expect(clippy::many_single_char_names, clippy::cast_precision_loss, dead_code)]

use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Convert RGBA u32 (0xRRGGBBAA) to tiny-skia Color.
fn color_from_u32(rgba: u32) -> tiny_skia::Color {
    let red = ((rgba >> 24) & 0xFF) as f32 / 255.0;
    let green = ((rgba >> 16) & 0xFF) as f32 / 255.0;
    let blue = ((rgba >> 8) & 0xFF) as f32 / 255.0;
    let alpha = (rgba & 0xFF) as f32 / 255.0;
    tiny_skia::Color::from_rgba(red, green, blue, alpha).unwrap_or(tiny_skia::Color::TRANSPARENT)
}

/// Fill a rectangle with solid color.
pub fn fill_rect(pixmap: &mut Pixmap, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let mut paint = Paint::default();
    paint.set_color(color_from_u32(color));
    paint.anti_alias = false;

    let rect = tiny_skia::Rect::from_xywh(x as f32, y as f32, w as f32, h as f32);
    if let Some(rect) = rect {
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// Draw a rounded rectangle.
pub fn draw_rounded_rect(
    pixmap: &mut Pixmap,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    color: u32,
) {
    let mut paint = Paint::default();
    paint.set_color(color_from_u32(color));
    paint.anti_alias = true;

    let r = radius as f32;
    let x = x as f32;
    let y = y as f32;
    let w = w as f32;
    let h = h as f32;

    // Clamp radius to half of smallest dimension
    let r = r.min(w / 2.0).min(h / 2.0);

    let mut pb = PathBuilder::new();

    // Top-left corner
    pb.move_to(x + r, y);
    // Top edge
    pb.line_to(x + w - r, y);
    // Top-right corner
    pb.quad_to(x + w, y, x + w, y + r);
    // Right edge
    pb.line_to(x + w, y + h - r);
    // Bottom-right corner
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    // Bottom edge
    pb.line_to(x + r, y + h);
    // Bottom-left corner
    pb.quad_to(x, y + h, x, y + h - r);
    // Left edge
    pb.line_to(x, y + r);
    // Top-left corner
    pb.quad_to(x, y, x + r, y);
    pb.close();

    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Draw a filled circle.
pub fn draw_circle(pixmap: &mut Pixmap, cx: i32, cy: i32, r: u32, color: u32) {
    let mut paint = Paint::default();
    paint.set_color(color_from_u32(color));
    paint.anti_alias = true;

    let cx = cx as f32;
    let cy = cy as f32;
    let r = r as f32;

    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);

    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Stroke a rectangle (border only).
pub fn stroke_rect(
    pixmap: &mut Pixmap,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    border_width: u32,
    color: u32,
) {
    let mut paint = Paint::default();
    paint.set_color(color_from_u32(color));
    paint.anti_alias = false;

    let stroke = Stroke {
        width: border_width as f32,
        ..Stroke::default()
    };

    // Inset by half stroke width so border is fully inside bounds
    let inset = border_width as f32 / 2.0;
    let rect = tiny_skia::Rect::from_xywh(
        x as f32 + inset,
        y as f32 + inset,
        w as f32 - border_width as f32,
        h as f32 - border_width as f32,
    );

    if let Some(rect) = rect {
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// Draw a line with given width.
pub fn draw_line(pixmap: &mut Pixmap, x1: i32, y1: i32, x2: i32, y2: i32, width: u32, color: u32) {
    let mut paint = Paint::default();
    paint.set_color(color_from_u32(color));
    paint.anti_alias = true;

    let stroke = Stroke {
        width: width as f32,
        ..Stroke::default()
    };

    let mut pb = PathBuilder::new();
    pb.move_to(x1 as f32, y1 as f32);
    pb.line_to(x2 as f32, y2 as f32);

    if let Some(path) = pb.finish() {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}
