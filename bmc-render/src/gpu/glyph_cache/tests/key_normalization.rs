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

use super::*;

const SANS: &str = "Braiins Sans";
const DECK_SANS: &str = "Braiins Deck Sans";

/// Shape through the embedded faces, exactly as the draw path does,
/// and take the first glyph's physical key — so `font_id`, `font_weight`
/// and flags are the ones production really produces.
fn shape_one_glyph(text: &str, font_size: f32, family: &str) -> cosmic_text::CacheKey {
    let mut font_system = crate::gpu::renderer::build_font_system();
    let mut buffer = cosmic_text::Buffer::new(
        &mut font_system,
        cosmic_text::Metrics::new(font_size, font_size),
    );
    buffer.set_text(
        &mut font_system,
        text,
        &cosmic_text::Attrs::new().family(cosmic_text::Family::Name(family)),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, false);

    buffer
        .layout_runs()
        .next()
        .expect("BUG: shaping produced no layout run")
        .glyphs
        .first()
        .expect("BUG: shaped layout run carries no glyph")
        .physical((0.0, 0.0), 1.0)
        .cache_key
}

fn any_key(x_bin: SubpixelBin, y_bin: SubpixelBin) -> cosmic_text::CacheKey {
    let mut key = shape_one_glyph("A", 17.0, SANS);
    key.x_bin = x_bin;
    key.y_bin = y_bin;
    key
}

fn with_other_font(key: cosmic_text::CacheKey) -> cosmic_text::CacheKey {
    let other = shape_one_glyph("A", 17.0, DECK_SANS);
    assert_ne!(
        other.font_id, key.font_id,
        "the two families must resolve to different faces for this to separate anything"
    );
    cosmic_text::CacheKey {
        font_id: other.font_id,
        ..key
    }
}

fn with_weight(key: cosmic_text::CacheKey, weight: u16) -> cosmic_text::CacheKey {
    cosmic_text::CacheKey {
        font_weight: cosmic_text::fontdb::Weight(weight),
        ..key
    }
}

fn with_fake_italic(key: cosmic_text::CacheKey) -> cosmic_text::CacheKey {
    cosmic_text::CacheKey {
        flags: key.flags | cosmic_text::CacheKeyFlags::FAKE_ITALIC,
        ..key
    }
}

#[test]
fn all_sixteen_bin_combinations_normalize_to_one_key() {
    let bins = [
        SubpixelBin::Zero,
        SubpixelBin::One,
        SubpixelBin::Two,
        SubpixelBin::Three,
    ];
    let base = GlyphKey::normalize(any_key(SubpixelBin::Zero, SubpixelBin::Zero));
    for x in bins {
        for y in bins {
            assert_eq!(GlyphKey::normalize(any_key(x, y)), base);
        }
    }
}

/// Normalization erases the bins and nothing else:
/// cosmic-text picks the face, the variable-font weight
/// and synthetic italic per glyph,
/// so folding any of those together would show one rasterization
/// for two visibly different glyphs.
#[test]
fn font_weight_and_flags_do_not_alias() {
    let base = any_key(SubpixelBin::Zero, SubpixelBin::Zero);
    for variant in [
        with_other_font(base),
        with_weight(base, 700),
        with_fake_italic(base),
    ] {
        assert_ne!(GlyphKey::normalize(variant), GlyphKey::normalize(base));
    }
}

#[test]
#[should_panic(expected = "PIXEL_FONT")]
fn pixel_font_flag_is_rejected() {
    let mut key = any_key(SubpixelBin::Zero, SubpixelBin::Zero);
    key.flags = cosmic_text::CacheKeyFlags::PIXEL_FONT;
    let _ = GlyphKey::normalize(key);
}
