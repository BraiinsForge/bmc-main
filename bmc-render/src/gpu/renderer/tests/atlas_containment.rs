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

//! Every text entry point must leave femtovg's own glyph atlas empty.
//!
//! Needs [`FemtoVgRenderer::font_atlas_texture_count`], which femtovg only
//! exposes under its `debug_inspector` feature — hence the extra gate. CI
//! builds with `--all-features`, so this runs there.

use super::FemtoVgRenderer;
use crate::renderer::prelude::*;
use crate::test_harness::{GlHarness, create_readback_fbo};
use bmc_ui::text::{ArcAnchor, ArcTextFacing};
use bmc_ui::{AutoFit, Color};
use bmc_ui::{Span, TextStyle};

const W: u32 = 320;
const H: u32 = 320;
const WHITE: Color = Color::from_rgb(255, 255, 255);

/// Straddles the direct-path cutoff on both sides and one size either way,
/// so an off-by-one in the comparison shows up as a populated atlas.
/// 17 and 91 are odd, which is what puts odd-width rasters
/// — the ones whose rows need padding — through the same draw.
const SIZES: [u32; 4] = [17, 91, 92, 93];

/// Greek variant letters no Braiins face carries, so they can only shape
/// through the fallback. Plain Greek would prove nothing: the Braiins faces
/// cover it and the primary face would answer.
const GREEK_FALLBACK: &str = "ϖϑϰϱϵ";

fn styled(size: u32, italic: bool) -> TextStyle {
    TextStyle {
        size,
        color: WHITE,
        italic,
        ..TextStyle::default()
    }
}

fn span(text: &str) -> Span {
    Span {
        text: text.to_owned(),
        weight: None,
        color: Some(WHITE),
        size: None,
        italic: false,
        underline: false,
        strikethrough: false,
    }
}

/// The fallback string must actually reach the fallback face;
/// otherwise the atlas assertion below covers only the primary font's glyphs
/// and the fallback path — where the font id comes from the shaper
/// rather than the caller's style — stays untested.
fn assert_shapes_through_fallback(renderer: &mut FemtoVgRenderer, size: u32) {
    #[expect(clippy::cast_precision_loss, reason = "test sizes are small")]
    let (lines, _) = renderer.layout_line(super::sans_line_style(size as f32), GREEK_FALLBACK);
    let glyphs = &lines
        .first()
        .expect("BUG: fallback string shaped no line")
        .glyphs;
    assert_eq!(
        glyphs.len(),
        GREEK_FALLBACK.chars().count(),
        "BUG: fallback fixture did not shape one glyph per character",
    );
    for glyph in glyphs {
        let face = renderer
            .font_system
            .db()
            .face(glyph.font_id)
            .expect("BUG: shaped glyph names a face the database does not have");
        let family = &face
            .families
            .first()
            .expect("BUG: face carries no family name")
            .0;
        assert!(
            family.contains("Noto"),
            "BUG: fallback fixture stayed on {family}; it no longer exercises fallback",
        );
    }
}

#[test]
fn no_text_entry_point_populates_the_femtovg_atlas() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    for size in SIZES {
        assert_shapes_through_fallback(&mut renderer, size);
        for italic in [false, true] {
            #[expect(clippy::cast_precision_loss, reason = "test sizes are small")]
            let px_size = size as f32;
            let style = styled(size, italic);
            let outlined = TextStyle {
                outline_color: Color::from_rgb(10, 10, 10),
                outline_width: 2.0,
                ..style
            };
            renderer.begin_frame(W, H, 1.0);
            for text in ["Wij. AV", GREEK_FALLBACK] {
                renderer.draw_paragraph(
                    &style,
                    &[span(text), span("gq")],
                    4.0,
                    4.0,
                    f32::from(u16::try_from(W).expect("BUG: canvas width exceeds u16")),
                );
                renderer.draw_text(text, 4.0, 100.0, px_size, WHITE);
                renderer.draw_canvas_text(text, 4.0, 160.0, &outlined);
                renderer.draw_curved_text(
                    160.0,
                    160.0,
                    140.0,
                    0.0,
                    ArcAnchor::Center,
                    ArcTextFacing::Outward,
                    text,
                    &style,
                );
                renderer.draw_autofit_text(
                    4.0,
                    240.0,
                    300.0,
                    70.0,
                    text,
                    &style,
                    AutoFit::ShrinkAndGrow,
                    size.try_into().unwrap_or(u16::MAX),
                    size.try_into().unwrap_or(u16::MAX),
                );
            }
            renderer.flush();
            assert_eq!(
                renderer.font_atlas_texture_count(),
                0,
                "BUG: femtovg atlased text at {size} px (italic: {italic})",
            );
        }
    }
}
