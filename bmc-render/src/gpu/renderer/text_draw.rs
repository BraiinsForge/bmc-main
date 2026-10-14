// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! FemtoVG's side of [`crate::renderer::RenderText`]: shaped lines, paragraphs and canvas text.

#![expect(clippy::cast_precision_loss)]

use super::FemtoVgRenderer;
use crate::gpu::curved_text::arc_glyph_layout;
use crate::gpu::glyph_cache::GlyphCache;
use crate::gpu::text::LineStyle;
use crate::gpu::text::{
    DIRECT_PATH_CUTOFF_PX, FemtovgPages, FontTable, LineGlyphs, autofit_bounds,
    baseline_to_alphabetic, build_cached_curved_glyph_commands, build_glyph_commands,
    curved_glyph_origin_x, draw_line_glyphs, outline_glyph_commands, search_fit_size,
    submit_glyph_commands,
};
use crate::renderer::prelude::*;
use crate::renderer::{GlyphCacheCounters, TextLayoutCounters};
use crate::tree::FontFamily;
use crate::tree::FontWeight;
use crate::tree::{AutoFit, SpanData, TextAlign, TextOverflow, TextStyle, VerticalAlign};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{ArcAnchor, ArcTextFacing, WHITE};
use femtovg::renderer::OpenGl;
use femtovg::{Canvas, Paint};
use std::borrow::Cow;

/// Draw shaped lines anchored at `(x, y)` by `baseline`.
///
/// The anchor resolves against each line's own metrics, and the first line's
/// layout advance is removed so a lone line lands exactly on the anchor.
#[expect(clippy::too_many_arguments, reason = "one line's full draw state")]
fn draw_anchored_lines(
    canvas: &mut Canvas<OpenGl>,
    glyph_cache: &mut GlyphCache<femtovg::ImageId>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut cosmic_text::FontSystem,
    font_table: &FontTable,
    lines: &[LineGlyphs],
    x: f32,
    y: f32,
    baseline: femtovg::Baseline,
    paint: &Paint,
    font_size: f32,
) {
    let Some(first) = lines.first() else {
        return;
    };
    let first_advance = first.baseline_y;
    for line in lines {
        let alphabetic_y = baseline_to_alphabetic(y, baseline, line.max_ascent, line.max_descent)
            + line.baseline_y
            - first_advance;
        draw_line_glyphs(
            canvas,
            glyph_cache,
            swash,
            font_system,
            font_table,
            &line.glyphs,
            x,
            alphabetic_y,
            paint,
            font_size,
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one outlined line's full draw state"
)]
fn draw_anchored_lines_with_outline(
    canvas: &mut Canvas<OpenGl>,
    glyph_cache: &mut GlyphCache<femtovg::ImageId>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut cosmic_text::FontSystem,
    font_table: &FontTable,
    lines: &[LineGlyphs],
    x: f32,
    y: f32,
    baseline: femtovg::Baseline,
    outline_paint: &Paint,
    paint: &Paint,
    font_size: f32,
    rings: u32,
) {
    if font_size > DIRECT_PATH_CUTOFF_PX {
        for ring in 1..=rings {
            let d = ring as f32;
            for (dx, dy) in [
                (d, 0.0),
                (-d, 0.0),
                (0.0, d),
                (0.0, -d),
                (d, d),
                (-d, -d),
                (d, -d),
                (-d, d),
            ] {
                draw_anchored_lines(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    lines,
                    x + dx,
                    y + dy,
                    baseline,
                    outline_paint,
                    font_size,
                );
            }
        }
        draw_anchored_lines(
            canvas,
            glyph_cache,
            swash,
            font_system,
            font_table,
            lines,
            x,
            y,
            baseline,
            paint,
            font_size,
        );
        return;
    }

    let Some(first) = lines.first() else {
        return;
    };
    let first_advance = first.baseline_y;
    let mut prepared = Vec::with_capacity(lines.len());
    for line in lines {
        let alphabetic_y = baseline_to_alphabetic(y, baseline, line.max_ascent, line.max_descent)
            + line.baseline_y
            - first_advance;
        prepared.push(build_glyph_commands(
            &mut FemtovgPages { canvas },
            glyph_cache,
            swash,
            font_system,
            font_table,
            &line.glyphs,
            x,
            alphabetic_y,
            font_size,
        ));
    }

    for commands in &prepared {
        submit_glyph_commands(
            canvas,
            glyph_cache,
            outline_glyph_commands(commands, rings),
            outline_paint,
            font_size,
        );
    }
    for commands in prepared {
        submit_glyph_commands(canvas, glyph_cache, commands, paint, font_size);
    }
}

impl RenderText for FemtoVgRenderer {
    // -- Simple text --

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        let paint = Paint::color(self.paint_color(color));
        let Self {
            canvas,
            font_system,
            paragraph_cache,
            glyph_cache,
            swash,
            font_table,
            ..
        } = self;
        let entry = paragraph_cache.layout_single_line(font_system, sans_line_style(size), text);
        draw_anchored_lines(
            canvas,
            glyph_cache,
            swash,
            font_system,
            font_table,
            &entry.lines,
            x,
            y,
            femtovg::Baseline::Top,
            &paint,
            size,
        );
    }

    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        self.paragraph_cache
            .measure_single_line(&mut self.font_system, sans_line_style(size), text)
            .width
    }

    // -- Canvas text --

    fn draw_canvas_text(&mut self, text: &str, x: f32, y: f32, style: &TextStyle) {
        let size = style.size as f32;
        let baseline = femtovg_baseline(style.vertical_align);
        let paint = Paint::color(self.paint_color(style.color));
        let outline_paint = Paint::color(self.paint_color(style.outline_color));
        let (width, draw_x) = {
            let Self {
                canvas,
                font_system,
                paragraph_cache,
                glyph_cache,
                swash,
                font_table,
                ..
            } = self;
            let entry =
                paragraph_cache.layout_single_line(font_system, line_style(style, size), text);
            let width = entry.width;
            let draw_x = match style.align {
                TextAlign::Left => x,
                TextAlign::Center => x - width / 2.0,
                TextAlign::Right => x - width,
            };

            if style.outline_color != crate::colors::TRANSPARENT && style.outline_width > 0.0 {
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let rings = style.outline_width.ceil() as u32;
                draw_anchored_lines_with_outline(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    &entry.lines,
                    draw_x,
                    y,
                    baseline,
                    &outline_paint,
                    &paint,
                    size,
                    rings,
                );
            } else {
                draw_anchored_lines(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    &entry.lines,
                    draw_x,
                    y,
                    baseline,
                    &paint,
                    size,
                );
            }
            (width, draw_x)
        };

        // Decorations
        if style.underline || style.strikethrough {
            let thickness = (size / 14.0).max(1.0);

            if style.underline {
                let uy = y + size * 0.1;
                self.fill_rect(draw_x, uy, width, thickness, style.color);
            }
            if style.strikethrough {
                let sy = y - size * 0.3;
                self.fill_rect(draw_x, sy, width, thickness, style.color);
            }
        }
    }

    fn draw_curved_text(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        anchor: ArcAnchor,
        facing: ArcTextFacing,
        text: &str,
        style: &TextStyle,
    ) {
        if radius <= 0.0 || text.is_empty() {
            return;
        }

        let size = style.size as f32;
        let paint = Paint::color(self.paint_color(style.color));
        // Shaped as one string, never glyph by glyph: per-character layouts
        // would break clusters and ligatures and lose every kern between
        // neighbours, which is exactly what the arc advances are made of.
        let Self {
            canvas,
            font_system,
            paragraph_cache,
            glyph_cache,
            swash,
            font_table,
            ..
        } = self;
        let entry = paragraph_cache.layout_single_line(font_system, line_style(style, size), text);
        let Some(line) = entry.lines.first() else {
            return;
        };

        let widths: Vec<f32> = line.glyphs.iter().map(|glyph| glyph.w).collect();
        let alphabetic_y = baseline_to_alphabetic(
            0.0,
            femtovg::Baseline::Middle,
            line.max_ascent,
            line.max_descent,
        );

        if size > DIRECT_PATH_CUTOFF_PX {
            for (glyph, placement) in line
                .glyphs
                .iter()
                .zip(arc_glyph_layout(&widths, radius, angle, anchor, facing))
            {
                let px = cx + radius * placement.theta.sin();
                let py = cy - radius * placement.theta.cos();
                canvas.save();
                canvas.translate(px, py);
                canvas.rotate(placement.rotation);
                draw_line_glyphs(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    std::slice::from_ref(glyph),
                    curved_glyph_origin_x(glyph),
                    alphabetic_y,
                    &paint,
                    size,
                );
                canvas.restore();
            }
            return;
        }

        let commands = build_cached_curved_glyph_commands(
            &mut FemtovgPages { canvas },
            glyph_cache,
            swash,
            font_system,
            &line.glyphs,
            alphabetic_y,
        );
        for (command, placement) in commands
            .into_iter()
            .zip(arc_glyph_layout(&widths, radius, angle, anchor, facing))
        {
            let Some(command) = command else {
                continue;
            };
            let px = cx + radius * placement.theta.sin();
            let py = cy - radius * placement.theta.cos();
            canvas.save();
            canvas.translate(px, py);
            canvas.rotate(placement.rotation);
            submit_glyph_commands(canvas, glyph_cache, std::iter::once(command), &paint, size);
            canvas.restore();
        }
    }

    fn draw_autofit_text(
        &mut self,
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        text: &str,
        style: &TextStyle,
        mode: AutoFit,
        min_size: u16,
        max_size: u16,
    ) {
        if text.is_empty() || box_width <= 0.0 || box_height <= 0.0 {
            return;
        }
        // Autofit fits by wrapping, and canvas text ignores the overflow mode.
        let style = &TextStyle {
            text_overflow: TextOverflow::Wrap,
            ..*style
        };
        let spans = [SpanData {
            text: text.to_string(),
            weight: None,
            color: None,
            size: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }];

        let (lower, upper) = autofit_bounds(
            style.size,
            u32::from(min_size),
            u32::from(max_size),
            mode,
            Some(box_height),
            style.line_height,
        );
        let fitted = search_fit_size(lower, upper, Some(box_width), Some(box_height), |size| {
            // Performance enhancement: Normalize the color, so the cache is not invalidated on repeated measurements e.g., during fade.
            let probe = TextStyle {
                size,
                color: WHITE,
                ..*style
            };
            self.paragraph_cache
                .measure(&mut self.font_system, &probe, &spans, Some(box_width))
        });

        let sized = TextStyle {
            size: fitted,
            ..self.dim_style(style)
        };
        let (_, block_h) =
            self.paragraph_cache
                .measure(&mut self.font_system, &sized, &spans, Some(box_width));

        let draw_y = match style.vertical_align {
            VerticalAlign::Top => y,
            // Currently the Baseline behaves the same as the Center option.
            VerticalAlign::Center | VerticalAlign::Baseline => {
                y + ((box_height - block_h) / 2.0).max(0.0)
            }
            VerticalAlign::Bottom => y + (box_height - block_h).max(0.0),
        };

        self.paragraph_cache.draw(
            &mut self.font_system,
            &mut self.canvas,
            &mut self.glyph_cache,
            &mut self.swash,
            &self.font_table,
            &sized,
            &spans,
            x,
            draw_y,
            box_width,
        );
    }

    // -- Rich text paragraphs --

    fn measure_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        self.paragraph_cache
            .measure(&mut self.font_system, style, spans, max_width)
    }

    fn draw_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    ) {
        let style = self.dim_style(style);
        let spans = self.dim_spans(spans);
        self.paragraph_cache.draw(
            &mut self.font_system,
            &mut self.canvas,
            &mut self.glyph_cache,
            &mut self.swash,
            &self.font_table,
            &style,
            &spans,
            x,
            y,
            max_width,
        );
    }

    fn draw_paragraph_clipped(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
        clip_top: f32,
        clip_bottom: f32,
    ) {
        self.push_scissor(x, clip_top, max_width, clip_bottom - clip_top);
        self.draw_paragraph(style, spans, x, y, max_width);
        self.pop_scissor();
    }

    // -- Text profiling --

    fn text_atlas_resident_bytes(&self) -> u64 {
        as_u64(self.glyph_cache.resident_atlas_bytes())
    }

    fn glyph_cache_counters(&self) -> GlyphCacheCounters {
        *self.glyph_cache.counters()
    }

    fn glyph_cache_metadata_capacity_bytes(&self) -> u64 {
        as_u64(self.glyph_cache.metadata_capacity_bytes())
    }

    fn text_layout_counters(&self) -> TextLayoutCounters {
        self.paragraph_cache.counters()
    }

    fn paragraph_cache_stats(&self) -> (u32, usize) {
        self.paragraph_cache.stats()
    }
}

impl FemtoVgRenderer {
    #[cfg(test)]
    pub(super) fn layout_line(&mut self, style: LineStyle, text: &str) -> (Vec<LineGlyphs>, f32) {
        let entry = self
            .paragraph_cache
            .layout_single_line(&mut self.font_system, style, text);
        (entry.lines.clone(), entry.width)
    }

    fn dim_style(&self, style: &TextStyle) -> TextStyle {
        TextStyle {
            color: self.dim(style.color),
            outline_color: self.dim(style.outline_color),
            ..*style
        }
    }

    /// Borrowed while nothing dims, so a lit paragraph clones no spans.
    fn dim_spans<'s>(&self, spans: &'s [SpanData]) -> Cow<'s, [SpanData]> {
        if self.brightness >= 1.0 {
            return Cow::Borrowed(spans);
        }
        spans
            .iter()
            .map(|span| SpanData {
                color: span.color.map(|c| self.dim(c)),
                ..span.clone()
            })
            .collect()
    }
}

/// The line style behind [`RenderText::draw_text`] and [`RenderText::measure_text`],
/// whose signatures carry no family, weight or slant.
pub(crate) fn sans_line_style(size: f32) -> LineStyle {
    LineStyle {
        family: FontFamily::Sans,
        weight: FontWeight::REGULAR,
        italic: false,
        size,
    }
}

/// The line style of a canvas [`TextStyle`], whose `size` is whole-pixel `u32`
/// while animated callers pass the fractional size separately.
fn line_style(style: &TextStyle, size: f32) -> LineStyle {
    LineStyle {
        family: style.family,
        weight: style.weight,
        italic: style.italic,
        size,
    }
}

pub(super) fn femtovg_baseline(vertical_align: VerticalAlign) -> femtovg::Baseline {
    match vertical_align {
        VerticalAlign::Top => femtovg::Baseline::Top,
        VerticalAlign::Center => femtovg::Baseline::Middle,
        VerticalAlign::Bottom => femtovg::Baseline::Bottom,
        VerticalAlign::Baseline => femtovg::Baseline::Alphabetic,
    }
}

fn as_u64(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}
