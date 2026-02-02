// Copyright (C) 2025  Braiins Systems s.r.o.

//! Text rendering using cosmic-text.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use cosmic_text::{Attrs, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache};
use tiny_skia::{Paint, Pixmap, Rect, Transform};

/// Draw text onto a pixmap using cosmic-text's built-in draw callback.
#[expect(clippy::too_many_arguments)]
pub fn draw_text(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
    text: &str,
    x: i32,
    y: i32,
    size: u32,
    color: u32,
) {
    let metrics = Metrics::new(size as f32, size as f32 * 1.2);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some(pixmap.width() as f32),
        Some(pixmap.height() as f32),
    );
    buffer.set_text(font_system, text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    let text_color = Color::rgba(
        ((color >> 24) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    );

    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };

    buffer.draw(
        font_system,
        cache,
        text_color,
        |px, py, pw, ph, drawn_color| {
            paint.set_color_rgba8(
                drawn_color.b(),
                drawn_color.g(),
                drawn_color.r(),
                drawn_color.a(),
            );
            if let Some(rect) =
                Rect::from_xywh((x + px) as f32, (y + py) as f32, pw as f32, ph as f32)
            {
                pixmap.fill_rect(rect, &paint, Transform::identity(), None);
            }
        },
    );
}

/// Measure text width.
pub fn measure_text(font_system: &mut FontSystem, text: &str, size: u32) -> u32 {
    let metrics = Metrics::new(size as f32, size as f32 * 1.2);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_text(font_system, text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0) as u32
}
