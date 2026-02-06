// Copyright (C) 2026  Braiins Systems s.r.o.

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

/// Cached paragraph with lazy pixmap rendering.
///
/// Measure path (Taffy callback): shapes buffer, caches dimensions.
/// Render path: renders buffer to pixmap on first access, then drops buffer.
/// Subsequent frames: returns cached pixmap directly (just a blit).
struct ParagraphCacheEntry {
    /// Shaped buffer — kept until pixmap is rendered, then dropped to save memory.
    buffer: Option<Buffer>,
    /// Rendered pixmap — created lazily on first render call.
    pixmap: Option<Pixmap>,
    /// Measured dimensions (cached to skip layout_runs scan on measure hit).
    width: f32,
    height: f32,
    /// Max width used for rendering (needed to size the pixmap).
    max_width: f32,
    /// For GC: last frame this entry was accessed.
    last_used_frame: u64,
}

/// Cache for shaped text buffers to avoid re-shaping every frame.
pub struct ShapedTextCache {
    entries: HashMap<u64, ShapedTextEntry>,
    paragraph_entries: HashMap<u64, ParagraphCacheEntry>,
    max_entries: usize,
    frame_counter: u64,
}

impl std::fmt::Debug for ShapedTextCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapedTextCache")
            .field("entries", &self.entries.len())
            .field("paragraph_entries", &self.paragraph_entries.len())
            .field("max_entries", &self.max_entries)
            .finish_non_exhaustive()
    }
}

impl ShapedTextCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            paragraph_entries: HashMap::new(),
            max_entries,
            frame_counter: 0,
        }
    }

    /// Start a new frame: update counter and GC stale paragraph entries.
    pub fn begin_frame(&mut self, frame_counter: u64) {
        self.frame_counter = frame_counter;
        self.paragraph_entries
            .retain(|_, e| e.last_used_frame + 1 >= frame_counter);
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

    /// Shape a paragraph buffer and cache it. Returns cached (width, height).
    /// Used by the Taffy measure callback — does NOT create a pixmap yet.
    pub fn get_or_shape_paragraph(
        &mut self,
        font_system: &mut FontSystem,
        base_style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
        align: TextAlign,
    ) -> (f32, f32) {
        let key = paragraph_cache_key(base_style, spans, max_width);
        let frame = self.frame_counter;

        // Evict before entry() to avoid double borrow
        if self.paragraph_entries.len() >= 256 && !self.paragraph_entries.contains_key(&key) {
            if let Some(&oldest) = self
                .paragraph_entries
                .iter()
                .min_by_key(|(_, e)| e.last_used_frame)
                .map(|(k, _)| k)
            {
                self.paragraph_entries.remove(&oldest);
            }
        }

        let entry = self.paragraph_entries.entry(key).or_insert_with(|| {
            let (buffer, width, height) =
                shape_paragraph(font_system, base_style, spans, max_width, align);
            ParagraphCacheEntry {
                buffer: Some(buffer),
                pixmap: None,
                width,
                height,
                max_width: max_width.unwrap_or(width),
                last_used_frame: frame,
            }
        });
        entry.last_used_frame = frame;
        (entry.width, entry.height)
    }

    /// Get a rendered paragraph pixmap, creating it lazily if needed.
    /// On first call: renders the shaped buffer to a pixmap, then drops the buffer.
    /// On subsequent calls: returns the cached pixmap directly.
    pub fn get_or_render_paragraph(
        &mut self,
        font_system: &mut FontSystem,
        swash_cache: &mut SwashCache,
        base_style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
        align: TextAlign,
    ) -> (&Pixmap, f32, f32) {
        let key = paragraph_cache_key(base_style, spans, max_width);
        let frame = self.frame_counter;

        // Evict before entry() to avoid double borrow
        if self.paragraph_entries.len() >= 256 && !self.paragraph_entries.contains_key(&key) {
            if let Some(&oldest) = self
                .paragraph_entries
                .iter()
                .min_by_key(|(_, e)| e.last_used_frame)
                .map(|(k, _)| k)
            {
                self.paragraph_entries.remove(&oldest);
            }
        }

        let entry = self.paragraph_entries.entry(key).or_insert_with(|| {
            // Full miss — shape + render in one shot
            let (buffer, width, height) =
                shape_paragraph(font_system, base_style, spans, max_width, align);
            let mw = max_width.unwrap_or(width);
            let pixmap = render_to_pixmap(
                font_system,
                swash_cache,
                &buffer,
                base_style,
                spans,
                mw,
                height,
            );
            ParagraphCacheEntry {
                buffer: None,
                pixmap: Some(pixmap),
                width,
                height,
                max_width: mw,
                last_used_frame: frame,
            }
        });
        entry.last_used_frame = frame;

        // Lazy render: buffer exists but pixmap doesn't yet (measure ran first)
        if entry.pixmap.is_none() {
            if let Some(buffer) = entry.buffer.take() {
                entry.pixmap = Some(render_to_pixmap(
                    font_system,
                    swash_cache,
                    &buffer,
                    base_style,
                    spans,
                    entry.max_width,
                    entry.height,
                ));
            }
        }

        (entry.pixmap.as_ref().unwrap(), entry.width, entry.height)
    }

    /// Clear the cache (e.g., on font change).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.paragraph_entries.clear();
    }
}

/// Compute a cache key for paragraph content. Hashes in-place — no allocations.
fn paragraph_cache_key(base_style: &TextStyle, spans: &[SpanData], max_width: Option<f32>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    base_style.size.hash(&mut hasher);
    base_style.weight.hash(&mut hasher);
    base_style.italic.hash(&mut hasher);
    base_style.line_height.to_bits().hash(&mut hasher);
    (base_style.align as u8).hash(&mut hasher);
    base_style.max_width.hash(&mut hasher);
    base_style.color.hash(&mut hasher);

    spans.len().hash(&mut hasher);
    for span in spans {
        span.text.hash(&mut hasher);
        span.weight.hash(&mut hasher);
        span.color.hash(&mut hasher);
        span.italic.hash(&mut hasher);
        span.underline.hash(&mut hasher);
        span.strikethrough.hash(&mut hasher);
    }

    max_width.map(f32::to_bits).hash(&mut hasher);

    hasher.finish()
}

/// Shape a paragraph: create buffer, set rich text, measure. Returns (buffer, width, height).
fn shape_paragraph(
    font_system: &mut FontSystem,
    base_style: &TextStyle,
    spans: &[SpanData],
    max_width: Option<f32>,
    align: TextAlign,
) -> (Buffer, f32, f32) {
    let line_height = base_style.size as f32 * base_style.line_height;
    let metrics = Metrics::new(base_style.size as f32, line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, max_width, None);

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

    // Set alignment before shaping
    let cosmic_align = match align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    for line in &mut buffer.lines {
        line.set_align(Some(cosmic_align));
    }

    buffer.shape_until_scroll(font_system, false);

    // Measure from layout runs
    let width = buffer
        .layout_runs()
        .map(|run| run.line_w)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0);
    let line_count = buffer.layout_runs().count().max(1);
    let height = line_count as f32 * line_height;

    (buffer, width, height)
}

/// Render a shaped buffer (glyphs + decorations) to a transparent pixmap.
fn render_to_pixmap(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    buffer: &Buffer,
    base_style: &TextStyle,
    spans: &[SpanData],
    max_width: f32,
    height: f32,
) -> Pixmap {
    let pix_w = (max_width.ceil() as u32).max(1);
    let pix_h = (height.ceil() as u32).max(1);
    let mut pixmap = Pixmap::new(pix_w, pix_h).unwrap();

    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };

    // Render glyphs at local coordinates (0, 0)
    buffer.draw(
        font_system,
        swash_cache,
        to_cosmic_color(base_style.color),
        |px, py, pw, ph, drawn_color| {
            paint.set_color_rgba8(
                drawn_color.b(),
                drawn_color.g(),
                drawn_color.r(),
                drawn_color.a(),
            );
            if let Some(rect) = Rect::from_xywh(px as f32, py as f32, pw as f32, ph as f32) {
                pixmap.fill_rect(rect, &paint, Transform::identity(), None);
            }
        },
    );

    // Render decorations (underline, strikethrough) at local coordinates
    draw_decorations(
        &mut pixmap,
        buffer,
        base_style,
        spans,
        0,
        0,
        max_width,
        None,
        None,
    );

    pixmap
}

/// Blit a cached paragraph pixmap onto the destination with premultiplied alpha compositing.
/// Supports optional Y-axis clipping for scrollable regions.
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::integer_division
)]
fn blit_paragraph(
    dst: &mut Pixmap,
    src: &Pixmap,
    x: i32,
    y: i32,
    clip_top: Option<i32>,
    clip_bottom: Option<i32>,
) {
    let dst_w = dst.width() as i32;
    let dst_h = dst.height() as i32;
    let src_w = src.width() as i32;
    let src_h = src.height() as i32;

    let clip_top = clip_top.unwrap_or(0);
    let clip_bottom = clip_bottom.unwrap_or(dst_h);

    let x_start = 0_i32.max(-x);
    let x_end = src_w.min(dst_w - x);
    if x_start >= x_end {
        return;
    }

    let src_data = src.data();
    let dst_data = dst.data_mut();

    for src_row in 0..src_h {
        let dst_y = y + src_row;
        if dst_y < clip_top || dst_y >= clip_bottom || dst_y < 0 || dst_y >= dst_h {
            continue;
        }

        let src_row_offset = (src_row * src_w) as usize * 4;
        let dst_row_offset = (dst_y * dst_w) as usize * 4;

        for src_col in x_start..x_end {
            let si = src_row_offset + src_col as usize * 4;
            let sa = src_data[si + 3] as u32;
            if sa == 0 {
                continue;
            }

            let di = dst_row_offset + (x + src_col) as usize * 4;

            if sa == 255 {
                dst_data[di..di + 4].copy_from_slice(&src_data[si..si + 4]);
            } else {
                // Premultiplied alpha compositing: dst = src + dst * (1 - src_alpha)
                let inv_a = 255 - sa;
                dst_data[di] = (src_data[si] as u32 + dst_data[di] as u32 * inv_a / 255) as u8;
                dst_data[di + 1] =
                    (src_data[si + 1] as u32 + dst_data[di + 1] as u32 * inv_a / 255) as u8;
                dst_data[di + 2] =
                    (src_data[si + 2] as u32 + dst_data[di + 2] as u32 * inv_a / 255) as u8;
                dst_data[di + 3] = (sa + dst_data[di + 3] as u32 * inv_a / 255) as u8;
            }
        }
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

/// Measure a paragraph with multiple spans (cached).
/// Returns (width, height). Only shapes the buffer — no pixmap created yet.
pub fn measure_paragraph(
    font_system: &mut FontSystem,
    text_cache: &mut ShapedTextCache,
    base_style: &TextStyle,
    spans: &[SpanData],
    max_width: Option<f32>,
) -> (f32, f32) {
    text_cache.get_or_shape_paragraph(font_system, base_style, spans, max_width, base_style.align)
}

/// Render a paragraph with multiple styled spans.
/// On first call: renders glyphs + decorations to a cached pixmap.
/// On subsequent calls: blits the cached pixmap directly (no shaping, no per-pixel callbacks).
#[expect(clippy::too_many_arguments)]
pub fn render_paragraph(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
    base_style: &TextStyle,
    spans: &[SpanData],
    x: i32,
    y: i32,
    width: u32,
) {
    let max_width = if base_style.max_width > 0 {
        (base_style.max_width as f32).min(width as f32)
    } else {
        width as f32
    };

    let (src, _, _) = text_cache.get_or_render_paragraph(
        font_system,
        swash_cache,
        base_style,
        spans,
        Some(max_width),
        base_style.align,
    );

    blit_paragraph(pixmap, src, x, y, None, None);
}

/// Render a paragraph with Y-axis clipping (for scrollable areas).
/// Same pixmap caching as `render_paragraph`, with clip bounds during blit.
#[expect(clippy::too_many_arguments)]
pub fn render_paragraph_clipped(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
    base_style: &TextStyle,
    spans: &[SpanData],
    x: i32,
    y: i32,
    width: u32,
    clip_top: i32,
    clip_bottom: i32,
) {
    let max_width = if base_style.max_width > 0 {
        (base_style.max_width as f32).min(width as f32)
    } else {
        width as f32
    };

    let (src, _, _) = text_cache.get_or_render_paragraph(
        font_system,
        swash_cache,
        base_style,
        spans,
        Some(max_width),
        base_style.align,
    );

    blit_paragraph(pixmap, src, x, y, Some(clip_top), Some(clip_bottom));
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
