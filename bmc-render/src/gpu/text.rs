// Copyright (C) 2026  Braiins Systems s.r.o.

//! Paragraph layout (cosmic-text) and rendering (FemtoVG).
//!
//! cosmic-text handles shaping + line-breaking, FemtoVG renders on the GPU.
//! Both use rustybuzz internally with the same BraiinsSans font binary,
//! so glyph advances match.
//!
//! # Coordinate model
//!
//! cosmic-text's [`LayoutRun`] exposes two Y fields:
//!
//! - **`line_top`** — top of the line box (advances by `line_height` per line).
//! - **`line_y`** — the **alphabetic baseline**, computed as
//!   `line_top + centering_offset + max_ascent` where
//!   `centering_offset = (line_height − glyph_height) / 2`.
//!
//! We render each text segment with [`femtovg::Baseline::Alphabetic`] at
//! `run.line_y` so that vertical placement matches what cosmic-text's own
//! `buffer.draw()` / swash path would produce.  Decorations (underline,
//! strikethrough) are positioned relative to this baseline.
//!
//! [`LayoutRun`]: cosmic_text::LayoutRun

#![expect(clippy::cast_precision_loss, clippy::string_slice)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, Weight,
};
use femtovg::{Canvas, FontId, Paint, renderer::OpenGl};

use crate::tree::{SpanData, TextAlign, TextStyle};

// ── Paragraph layout cache ──────────────────────────────────────────

/// Cached paragraph layout (cosmic-text Buffer kept for layout_runs during render).
struct ParagraphLayoutEntry {
    buffer: Buffer,
    width: f32,
    height: f32,
    #[expect(dead_code)]
    max_width: f32,
    last_used_frame: u64,
    /// Concatenated span text (precomputed to avoid per-draw allocation).
    full_text: String,
    /// Byte-offset → span-index lookup (precomputed).
    span_offsets: Vec<(usize, usize, usize)>,
}

/// Frame-based paragraph layout cache. Evicts entries not accessed in the last 2 frames.
pub struct ParagraphLayoutCache {
    entries: HashMap<u64, ParagraphLayoutEntry>,
    frame_counter: u64,
}

impl std::fmt::Debug for ParagraphLayoutCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParagraphLayoutCache")
            .field("entries", &self.entries.len())
            .field("frame_counter", &self.frame_counter)
            .finish()
    }
}

impl ParagraphLayoutCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            frame_counter: 0,
        }
    }

    /// Advance frame counter and evict stale entries.
    pub fn begin_frame(&mut self, frame_counter: u64) {
        self.frame_counter = frame_counter;
        self.entries
            .retain(|_, e| e.last_used_frame + 1 >= frame_counter);
    }

    /// Measure paragraph dimensions, shaping if not cached.
    pub fn measure(
        &mut self,
        font_system: &mut FontSystem,
        base_style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let key = cache_key(base_style, spans, max_width);
        let frame = self.frame_counter;

        // Evict oldest if at capacity
        if self.entries.len() >= 256
            && !self.entries.contains_key(&key)
            && let Some(&oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_used_frame)
                .map(|(k, _)| k)
        {
            self.entries.remove(&oldest);
        }

        let entry = self.entries.entry(key).or_insert_with(|| {
            let (buffer, width, height) =
                shape_paragraph(font_system, base_style, spans, max_width);
            let full_text: String = spans.iter().map(|s| s.text.as_str()).collect();
            let span_offsets = build_span_offsets(spans);
            ParagraphLayoutEntry {
                buffer,
                width,
                height,
                max_width: max_width.unwrap_or(width),
                last_used_frame: frame,
                full_text,
                span_offsets,
            }
        });
        entry.last_used_frame = frame;
        (entry.width, entry.height)
    }

    /// Draw a paragraph using cached layout. Calls FemtoVG fill_text per span segment.
    #[expect(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        font_system: &mut FontSystem,
        canvas: &mut Canvas<OpenGl>,
        font_regular: FontId,
        font_bold: FontId,
        base_style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    ) {
        // Ensure layout is cached (also precomputes full_text + span_offsets)
        self.measure(font_system, base_style, spans, Some(max_width));
        let key = cache_key(base_style, spans, Some(max_width));
        let entry = &self.entries[&key];

        let full_text = &entry.full_text;
        let span_offsets = &entry.span_offsets;

        for run in entry.buffer.layout_runs() {
            let align_offset = match base_style.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (max_width - run.line_w) / 2.0,
                TextAlign::Right => max_width - run.line_w,
            };

            // run.line_y is the alphabetic baseline (not line top — see module docs)
            let baseline_y = y + run.line_y;

            let mut current_span = usize::MAX;
            let mut segment_start_x = 0.0_f32;
            let mut segment_text = String::new();

            for glyph in run.glyphs {
                let span_idx = find_span(glyph.start, span_offsets);

                if span_idx != current_span && !segment_text.is_empty() {
                    // Flush previous segment
                    let style = spans[current_span].resolve_style(base_style);
                    draw_text_segment(
                        canvas,
                        font_regular,
                        font_bold,
                        &segment_text,
                        x + segment_start_x + align_offset,
                        baseline_y,
                        &style,
                    );
                    let w = segment_width(canvas, font_regular, font_bold, &segment_text, &style);
                    draw_decorations_for_segment(
                        canvas,
                        x + segment_start_x + align_offset,
                        baseline_y,
                        w,
                        &style,
                    );
                    segment_text.clear();
                }

                if segment_text.is_empty() {
                    segment_start_x = glyph.x;
                    current_span = span_idx;
                }

                if glyph.end <= full_text.len() {
                    segment_text.push_str(&full_text[glyph.start..glyph.end]);
                }
            }

            // Flush last segment
            if !segment_text.is_empty() && current_span != usize::MAX {
                let style = spans[current_span].resolve_style(base_style);
                draw_text_segment(
                    canvas,
                    font_regular,
                    font_bold,
                    &segment_text,
                    x + segment_start_x + align_offset,
                    baseline_y,
                    &style,
                );
                let w = segment_width(canvas, font_regular, font_bold, &segment_text, &style);
                draw_decorations_for_segment(
                    canvas,
                    x + segment_start_x + align_offset,
                    baseline_y,
                    w,
                    &style,
                );
            }
        }
    }
}

// ── Internal helpers ────────────────────────────────────────────────

/// Compute cache key from style + spans + max_width.
fn cache_key(base_style: &TextStyle, spans: &[SpanData], max_width: Option<f32>) -> u64 {
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

/// Shape a paragraph using cosmic-text. Returns (buffer, width, height).
fn shape_paragraph(
    font_system: &mut FontSystem,
    base_style: &TextStyle,
    spans: &[SpanData],
    max_width: Option<f32>,
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

    let cosmic_align = match base_style.align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    for line in &mut buffer.lines {
        line.set_align(Some(cosmic_align));
    }

    buffer.shape_until_scroll(font_system, false);

    let width = buffer
        .layout_runs()
        .map(|run| run.line_w)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0)
        .ceil();
    let line_count = buffer.layout_runs().count().max(1);
    let height = line_count as f32 * line_height;

    (buffer, width, height)
}

/// Build cosmic_text Attrs from a resolved TextStyle.
#[must_use]
pub fn build_attrs(style: &TextStyle) -> Attrs<'static> {
    let mut attrs = Attrs::new()
        .family(Family::Name("Braiins Sans"))
        .weight(Weight(style.weight));

    if style.italic {
        attrs = attrs.style(Style::Italic);
    }

    attrs.color(Color::rgba(
        style.color.red(),
        style.color.green(),
        style.color.blue(),
        style.color.alpha(),
    ))
}

/// Build byte-offset → span-index lookup table.
fn build_span_offsets(spans: &[SpanData]) -> Vec<(usize, usize, usize)> {
    let mut offsets = Vec::with_capacity(spans.len());
    let mut pos = 0;
    for (i, span) in spans.iter().enumerate() {
        let end = pos + span.text.len();
        offsets.push((pos, end, i));
        pos = end;
    }
    offsets
}

/// Find which span a byte offset belongs to.
fn find_span(byte_offset: usize, span_offsets: &[(usize, usize, usize)]) -> usize {
    for &(start, end, idx) in span_offsets {
        if byte_offset >= start && byte_offset < end {
            return idx;
        }
    }
    0
}

/// Select FemtoVG font based on weight.
fn select_font(font_regular: FontId, font_bold: FontId, weight: u16) -> FontId {
    if weight >= 600 {
        font_bold
    } else {
        font_regular
    }
}

/// Convert RGBA u32 to femtovg Color.
#[must_use]
pub fn to_femtovg_color(color: u32) -> femtovg::Color {
    femtovg::Color::rgba(
        ((color >> 24) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

/// Draw a text segment with FemtoVG at the given **baseline** y-coordinate.
///
/// Uses [`femtovg::Baseline::Alphabetic`] because the y-coordinate comes from
/// cosmic-text's `LayoutRun::line_y` which is the alphabetic baseline
/// (= `line_top + centering_offset + max_ascent`).
fn draw_text_segment(
    canvas: &mut Canvas<OpenGl>,
    font_regular: FontId,
    font_bold: FontId,
    text: &str,
    x: f32,
    baseline_y: f32,
    style: &TextStyle,
) {
    let font = select_font(font_regular, font_bold, style.weight);
    let mut paint = Paint::color(to_femtovg_color(style.color.to_u32()));
    paint.set_font(&[font]);
    paint.set_font_size(style.size as f32);
    paint.set_text_baseline(femtovg::Baseline::Alphabetic);
    let _ = canvas.fill_text(x, baseline_y, text, &paint);
}

/// Measure text segment width using FemtoVG.
fn segment_width(
    canvas: &mut Canvas<OpenGl>,
    font_regular: FontId,
    font_bold: FontId,
    text: &str,
    style: &TextStyle,
) -> f32 {
    let font = select_font(font_regular, font_bold, style.weight);
    let mut paint = Paint::color(to_femtovg_color(style.color.to_u32()));
    paint.set_font(&[font]);
    paint.set_font_size(style.size as f32);
    canvas
        .measure_text(0.0, 0.0, text, &paint)
        .map_or(0.0, |m| m.width())
}

/// Draw underline / strikethrough for a text segment.
/// `baseline_y` is the text baseline (from cosmic-text `line_y`).
fn draw_decorations_for_segment(
    canvas: &mut Canvas<OpenGl>,
    x: f32,
    baseline_y: f32,
    w: f32,
    style: &TextStyle,
) {
    let fs = style.size as f32;

    if style.underline {
        let uy = baseline_y + fs * 0.1;
        let uh = (fs * 0.07).max(1.0);
        let mut path = femtovg::Path::new();
        path.rect(x, uy, w, uh);
        canvas.fill_path(&path, &Paint::color(to_femtovg_color(style.color.to_u32())));
    }

    if style.strikethrough {
        let sy = baseline_y - fs * 0.3;
        let sh = (fs * 0.07).max(1.0);
        let mut path = femtovg::Path::new();
        path.rect(x, sy, w, sh);
        canvas.fill_path(&path, &Paint::color(to_femtovg_color(style.color.to_u32())));
    }
}
