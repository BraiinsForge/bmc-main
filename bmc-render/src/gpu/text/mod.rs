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

//! Paragraph layout (cosmic-text) and rendering (FemtoVG).
//!
//! cosmic-text is the only shaper: it produces every glyph and every position,
//! and FemtoVG draws them as cached atlas quads (or, above
//! [`DIRECT_PATH_CUTOFF_PX`], as pre-positioned glyph runs). FemtoVG's own
//! shaper is never invoked, so its kerning divergence from cosmic-text
//! can no longer reach the screen.
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
//! Glyph quads are placed against `line_y`, so vertical placement matches what
//! cosmic-text's own `buffer.draw()` / swash path would produce. Decorations
//! (underline, strikethrough) are positioned relative to this baseline. Callers
//! anchoring text by any other [`femtovg::Baseline`] convert through
//! [`baseline_to_alphabetic`] first.
//!
//! [`LayoutRun`]: cosmic_text::LayoutRun

mod autofit;
mod glyphs;
mod layout_cache;
mod shaping;

pub(crate) use autofit::{autofit_bounds, search_fit_size};
pub use glyphs::to_femtovg_color;
pub(crate) use glyphs::{
    DIRECT_PATH_CUTOFF_PX, FemtovgPages, FontTable, build_cached_curved_glyph_commands,
    build_font_table, build_glyph_commands, curved_glyph_origin_x, draw_line_glyphs,
    outline_glyph_commands, submit_glyph_commands,
};
pub use layout_cache::ParagraphLayoutCache;
pub use shaping::build_attrs;
pub(crate) use shaping::{LineGlyphs, LineStyle, baseline_to_alphabetic};

#[cfg(feature = "glyph-alloc-gate")]
pub(crate) use glyphs::rasterize_glyph;
#[cfg(test)]
pub(crate) use glyphs::snap;

/// A pair the two shapers demonstrably kern apart at `size`,
/// from a fixed candidate list.
/// The module header records that cosmic-text and femtovg kern differently
/// without naming a pair, so the pair is discovered rather than pinned —
/// a font swap that made every candidate agree has to fail loudly
/// instead of leaving the tests built on it comparing nothing.
#[cfg(test)]
pub(crate) fn divergent_kerning_pair(
    font_system: &mut cosmic_text::FontSystem,
    size: f32,
) -> &'static str {
    const CANDIDATES: [&str; 7] = ["AV", "To", "Ta", "LT", "AW", "Yo", "P,"];
    /// Below this the two agree to within their own rounding.
    const MIN_DIVERGENCE_PX: f32 = 0.5;

    let fonts = femtovg::TextContext::default();
    let mut paint = femtovg::Paint::color(femtovg::Color::white());
    paint.set_font(&[fonts
        .add_font_mem(include_bytes!(
            "../../../../assets/fonts/BraiinsSans-Regular.otf"
        ))
        .expect("BUG: font registration failed")]);
    paint.set_font_size(size);

    let mut layout = ParagraphLayoutCache::new();
    CANDIDATES
        .into_iter()
        .find(|pair| {
            let cosmic = layout
                .layout_single_line(
                    font_system,
                    crate::gpu::renderer::sans_line_style(size),
                    pair,
                )
                .width;
            #[expect(
                clippy::disallowed_methods,
                reason = "test compares shapers on purpose"
            )]
            let femtovg = fonts
                .measure_text(0.0, 0.0, pair, &paint)
                .expect("BUG: femtovg cannot measure the fixture")
                .width();
            (cosmic - femtovg).abs() > MIN_DIVERGENCE_PX
        })
        .expect("BUG: no candidate separates the shapers; the list needs a new pair")
}

#[cfg(test)]
mod tests;
