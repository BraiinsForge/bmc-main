// Copyright (C) 2025  Braiins Systems s.r.o.

//! Text rendering using cosmic-text with shaped buffer caching.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight,
};
use tiny_skia::{Paint, Pixmap, Rect, Transform};

use crate::tree::{SpanData, TextAlign, TextStyle};

/// Cache key for shaped text buffers.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ShapedTextKey {
    text: String,
    size: u32,
    weight: u16,
    italic: bool,
    max_width: Option<u32>,
}

/// Cached shaped text buffer with metadata.
struct ShapedTextEntry {
    buffer: Buffer,
}

/// Cache for shaped text buffers to avoid re-shaping every frame.
pub struct ShapedTextCache {
    entries: HashMap<u64, ShapedTextEntry>,
    max_entries: usize,
}

impl std::fmt::Debug for ShapedTextCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapedTextCache")
            .field("entries", &self.entries.len())
            .field("max_entries", &self.max_entries)
            .finish()
    }
}

impl ShapedTextCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
        }
    }

    /// Compute hash for cache key.
    fn hash_key(key: &ShapedTextKey) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    /// Get or create a shaped buffer for simple text.
    pub fn get_or_shape_simple(
        &mut self,
        font_system: &mut FontSystem,
        text: &str,
        size: u32,
        max_width: Option<f32>,
    ) -> &Buffer {
        let key = ShapedTextKey {
            text: text.to_owned(),
            size,
            weight: 400,
            italic: false,
            max_width: max_width.map(|w| w as u32),
        };
        let hash = Self::hash_key(&key);

        if !self.entries.contains_key(&hash) {
            // Evict if cache is full (simple FIFO - could use LRU later)
            if self.entries.len() >= self.max_entries {
                if let Some(&first_key) = self.entries.keys().next() {
                    self.entries.remove(&first_key);
                }
            }

            let shaping = select_shaping(text);
            let line_height = size as f32 * 1.2;
            let metrics = Metrics::new(size as f32, line_height);
            let mut buffer = Buffer::new(font_system, metrics);

            buffer.set_size(font_system, max_width, None);
            buffer.set_text(font_system, text, &Attrs::new(), shaping, None);
            buffer.shape_until_scroll(font_system, false);

            self.entries.insert(hash, ShapedTextEntry { buffer });
        }

        &self.entries[&hash].buffer
    }

    /// Clear the cache (e.g., on font change).
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Determine if text needs advanced (complex) shaping or can use basic shaping.
/// Basic shaping is much faster but doesn't handle ligatures, RTL, etc.
fn select_shaping(text: &str) -> Shaping {
    // Use basic shaping if all characters are ASCII printable
    // This covers English UI text efficiently
    if text.bytes().all(|b| b.is_ascii() && b >= 0x20) {
        Shaping::Basic
    } else {
        Shaping::Advanced
    }
}

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
    draw_decorations(
        pixmap, &buffer, base_style, spans, x, y, max_width, None, None,
    );
}

/// Render a paragraph with Y-axis clipping (for scrollable areas).
#[expect(clippy::too_many_arguments)]
pub fn render_paragraph_clipped(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
    base_style: &TextStyle,
    spans: &[SpanData],
    x: i32,
    y: i32,
    width: u32,
    clip_top: i32,
    clip_bottom: i32,
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

    let clip_top_f = clip_top as f32;
    let clip_bottom_f = clip_bottom as f32;

    // Draw glyphs with Y clipping
    buffer.draw(
        font_system,
        cache,
        to_cosmic_color(base_style.color),
        |px, py, pw, ph, drawn_color| {
            let glyph_y = (y + py) as f32;
            let glyph_bottom = glyph_y + ph as f32;

            // Skip if completely outside clip region
            if glyph_bottom <= clip_top_f || glyph_y >= clip_bottom_f {
                return;
            }

            paint.set_color_rgba8(
                drawn_color.b(),
                drawn_color.g(),
                drawn_color.r(),
                drawn_color.a(),
            );

            // Clip the rect to the visible region
            let clipped_y = glyph_y.max(clip_top_f);
            let clipped_bottom = glyph_bottom.min(clip_bottom_f);
            let clipped_h = clipped_bottom - clipped_y;

            if clipped_h > 0.0 {
                if let Some(rect) =
                    Rect::from_xywh((x + px) as f32, clipped_y, pw as f32, clipped_h)
                {
                    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                }
            }
        },
    );

    // Draw decorations with clipping
    draw_decorations(
        pixmap,
        &buffer,
        base_style,
        spans,
        x,
        y,
        max_width,
        Some(clip_top),
        Some(clip_bottom),
    );
}

/// Draw underline and strikethrough decorations
#[expect(clippy::too_many_arguments)]
fn draw_decorations(
    pixmap: &mut Pixmap,
    buffer: &Buffer,
    base_style: &TextStyle,
    spans: &[SpanData],
    base_x: i32,
    base_y: i32,
    max_width: f32,
    clip_top: Option<i32>,
    clip_bottom: Option<i32>,
) {
    let font_size = base_style.size as f32;
    let clip_top_f = clip_top.map_or(f32::MIN, |v| v as f32);
    let clip_bottom_f = clip_bottom.map_or(f32::MAX, |v| v as f32);

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

            // Draw underline (with clipping)
            if style.underline {
                let underline_y = run_y + font_size * 0.1;
                let underline_h = (font_size * 0.07).max(1.0);
                if underline_y + underline_h > clip_top_f && underline_y < clip_bottom_f {
                    if let Some(rect) = Rect::from_xywh(glyph_x, underline_y, glyph_w, underline_h)
                    {
                        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
            }

            // Draw strikethrough (with clipping)
            if style.strikethrough {
                let strike_y = run_y - font_size * 0.3;
                let strike_h = (font_size * 0.07).max(1.0);
                if strike_y + strike_h > clip_top_f && strike_y < clip_bottom_f {
                    if let Some(rect) = Rect::from_xywh(glyph_x, strike_y, glyph_w, strike_h) {
                        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
            }
        }
    }
}

/// Draw simple text onto a pixmap with caching.
#[expect(clippy::too_many_arguments)]
pub fn draw_text(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
    text: &str,
    x: i32,
    y: i32,
    size: u32,
    color: u32,
) {
    let max_width = Some(pixmap.width() as f32);

    // Get or create shaped buffer
    let buffer = text_cache.get_or_shape_simple(font_system, text, size, max_width);

    let text_color = to_cosmic_color(color);

    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };

    buffer.draw(
        font_system,
        swash_cache,
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
