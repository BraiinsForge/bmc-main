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

use cosmic_text::fontdb;

use super::{FontSystem, rasterize_glyph};
use crate::gpu::glyph_cache::PAGE_SIZE_PX;

const CORPUS_SIZE_PX: f32 = 92.0;

fn key_for(
    face: (fontdb::ID, fontdb::Weight),
    glyph_id: u16,
    flags: cosmic_text::CacheKeyFlags,
) -> cosmic_text::CacheKey {
    cosmic_text::CacheKey {
        font_id: face.0,
        glyph_id,
        font_size_bits: CORPUS_SIZE_PX.to_bits(),
        x_bin: cosmic_text::SubpixelBin::Zero,
        y_bin: cosmic_text::SubpixelBin::Zero,
        font_weight: face.1,
        flags,
    }
}

/// `FaceInfo` carries no glyph count, and `db()` borrows the font system
/// immutably while `get_font` needs it mutably — so faces are snapshotted
/// in two owned passes before anything is rasterized.
fn corpus(font_system: &mut FontSystem) -> Vec<(fontdb::ID, fontdb::Weight, u16)> {
    let faces: Vec<(fontdb::ID, fontdb::Weight)> = font_system
        .db()
        .faces()
        .map(|face| (face.id, face.weight))
        .collect();
    faces
        .into_iter()
        .map(|(id, weight)| {
            let font = font_system
                .get_font(id, weight)
                .expect("BUG: font database face has no loaded font");
            let count = font.as_swash().glyph_metrics(&[]).glyph_count();
            (id, weight, count)
        })
        .collect()
}

#[test]
fn corpus_rasterizes_within_page_bounds() {
    let mut font_system = super::font_system();
    let mut swash = cosmic_text::SwashCache::new();

    let mut rasterized = 0_usize;
    let mut odd_width = 0_usize;
    let mut oversized = 0_usize;
    for (id, weight, glyph_count) in corpus(&mut font_system) {
        for glyph_id in 0..glyph_count {
            let key = key_for((id, weight), glyph_id, cosmic_text::CacheKeyFlags::empty());
            let Some(glyph) = rasterize_glyph(&mut swash, &mut font_system, key) else {
                continue;
            };
            assert_eq!(
                glyph.coverage.len(),
                glyph.width * glyph.height,
                "coverage must be a tightly packed mask of its placement"
            );
            assert!(
                glyph.width > 0 && glyph.height > 0,
                "empty coverage must be reported as a miss, not a zero-sized glyph"
            );
            rasterized += 1;
            odd_width += usize::from(glyph.width % 2 == 1);
            oversized +=
                usize::from(glyph.width + 2 > PAGE_SIZE_PX || glyph.height + 2 > PAGE_SIZE_PX);
        }
    }

    assert!(rasterized > 0, "the shipped fonts must rasterize something");
    assert!(
        odd_width > 0,
        "the corpus must exercise odd widths, which stress row padding"
    );
    assert_eq!(
        oversized, 0,
        "no shipped glyph at {CORPUS_SIZE_PX} px may exceed a page once padded"
    );
}

#[test]
fn fake_italic_key_produces_skewed_coverage() {
    let mut font_system = super::font_system();
    let mut swash = cosmic_text::SwashCache::new();

    let (id, weight, _) = corpus(&mut font_system)
        .into_iter()
        .next()
        .expect("BUG: font database is empty");
    let font = font_system
        .get_font(id, weight)
        .expect("BUG: font database face has no loaded font");
    let glyph_id = font.as_swash().charmap().map('W');

    let upright = rasterize_glyph(
        &mut swash,
        &mut font_system,
        key_for((id, weight), glyph_id, cosmic_text::CacheKeyFlags::empty()),
    )
    .expect("BUG: 'W' does not rasterize");
    let italic = rasterize_glyph(
        &mut swash,
        &mut font_system,
        key_for(
            (id, weight),
            glyph_id,
            cosmic_text::CacheKeyFlags::FAKE_ITALIC,
        ),
    )
    .expect("BUG: skewed 'W' does not rasterize");

    assert_ne!(
        (upright.width, upright.coverage),
        (italic.width, italic.coverage),
        "FAKE_ITALIC is part of the key because it changes the raster"
    );
}
