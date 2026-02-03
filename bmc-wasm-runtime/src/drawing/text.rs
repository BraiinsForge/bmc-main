// Copyright (C) 2025  Braiins Systems s.r.o.

//! Text rendering using cosmic-text.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight,
};
use tiny_skia::{Paint, Pixmap, Rect, Transform};

use crate::tree::{SpanData, TextAlign, TextStyle};

/// Convert our color format (RGBA u32) to cosmic_text Color
fn to_cosmic_color(color: u32) -> Color {
    Color::rgba(
        ((color >> 24) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

/// Build cosmic_text Attrs from a resolved TextStyle
fn build_attrs(style: &TextStyle) -> Attrs<'static> {
    let mut attrs = Attrs::new()
        .family(Family::SansSerif)
        .weight(Weight(style.weight));

    if style.italic {
        attrs = attrs.style(Style::Italic);
    }

    attrs.color(to_cosmic_color(style.color))
}

/// Measure a paragraph with multiple spans.
/// Returns (width, height).
pub fn measure_paragraph(
    font_system: &mut FontSystem,
    base_style: &TextStyle,
    spans: &[SpanData],
    max_width: Option<f32>,
) -> (f32, f32) {
    let line_height = base_style.size as f32 * base_style.line_height;
    let metrics = Metrics::new(base_style.size as f32, line_height);
    let mut buffer = Buffer::new(font_system, metrics);

    buffer.set_size(font_system, max_width, None);

    // Build rich text spans
    let rich_spans: Vec<_> = spans
        .iter()
        .map(|span| {
            let resolved = span.resolve_style(base_style);
            (span.text.as_str(), build_attrs(&resolved))
        })
        .collect();

    buffer.set_rich_text(
        font_system,
        rich_spans,
        &build_attrs(base_style),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    // Calculate width from layout runs
    let width = buffer
        .layout_runs()
        .map(|run| run.line_w)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0);

    // Calculate height from line count
    let line_count = buffer.layout_runs().count().max(1);
    let height = line_count as f32 * line_height;

    (width, height)
}

/// Render a paragraph with multiple styled spans.
#[expect(clippy::too_many_arguments)]
pub fn render_paragraph(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
    base_style: &TextStyle,
    spans: &[SpanData],
    x: i32,
    y: i32,
    width: u32,
) {
    let line_height = base_style.size as f32 * base_style.line_height;
    let metrics = Metrics::new(base_style.size as f32, line_height);
    let mut buffer = Buffer::new(font_system, metrics);

    // Determine max width for wrapping
    let max_width = if base_style.max_width > 0 {
        (base_style.max_width as f32).min(width as f32)
    } else {
        width as f32
    };

    buffer.set_size(font_system, Some(max_width), None);

    // Build rich text spans
    let rich_spans: Vec<_> = spans
        .iter()
        .map(|span| {
            let resolved = span.resolve_style(base_style);
            (span.text.as_str(), build_attrs(&resolved))
        })
        .collect();

    buffer.set_rich_text(
        font_system,
        rich_spans,
        &build_attrs(base_style),
        Shaping::Advanced,
        None,
    );

    // Set alignment on all lines
    let cosmic_align = match base_style.align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    for line in &mut buffer.lines {
        line.set_align(Some(cosmic_align));
    }

    buffer.shape_until_scroll(font_system, false);

    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };

    // Draw glyphs using cosmic_text's draw callback
    buffer.draw(
        font_system,
        cache,
        to_cosmic_color(base_style.color),
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

    // Draw decorations (underline, strikethrough)
    draw_decorations(pixmap, &buffer, base_style, spans, x, y, max_width);
}

/// Draw underline and strikethrough decorations
fn draw_decorations(
    pixmap: &mut Pixmap,
    buffer: &Buffer,
    base_style: &TextStyle,
    spans: &[SpanData],
    base_x: i32,
    base_y: i32,
    max_width: f32,
) {
    let font_size = base_style.size as f32;

    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };

    for run in buffer.layout_runs() {
        let run_y = base_y as f32 + run.line_y;

        // Calculate alignment offset for this run
        let align_offset = match base_style.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (max_width - run.line_w) / 2.0,
            TextAlign::Right => max_width - run.line_w,
        };

        for glyph in run.glyphs {
            // Find which span this glyph belongs to
            let mut current_offset = 0_usize;
            let mut span_style: Option<TextStyle> = None;

            for span in spans {
                let span_end = current_offset + span.text.len();
                if glyph.start >= current_offset && glyph.start < span_end {
                    span_style = Some(span.resolve_style(base_style));
                    break;
                }
                current_offset = span_end;
            }

            let style = span_style.unwrap_or(*base_style);
            let color = to_cosmic_color(style.color);
            paint.set_color_rgba8(color.b(), color.g(), color.r(), color.a());

            let glyph_x = base_x as f32 + glyph.x + align_offset;
            let glyph_w = glyph.w;

            // Draw underline
            if style.underline {
                let underline_y = run_y + font_size * 0.1; // Slightly below baseline
                let underline_h = (font_size * 0.07).max(1.0);
                if let Some(rect) = Rect::from_xywh(glyph_x, underline_y, glyph_w, underline_h) {
                    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                }
            }

            // Draw strikethrough
            if style.strikethrough {
                let strike_y = run_y - font_size * 0.3; // Middle of text
                let strike_h = (font_size * 0.07).max(1.0);
                if let Some(rect) = Rect::from_xywh(glyph_x, strike_y, glyph_w, strike_h) {
                    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                }
            }
        }
    }
}

/// Draw simple text onto a pixmap.
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

    let text_color = to_cosmic_color(color);

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
