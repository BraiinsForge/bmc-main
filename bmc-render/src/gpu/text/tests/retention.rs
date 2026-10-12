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

use cosmic_text::{FontSystem, fontdb::Database};

use super::ParagraphLayoutCache;
use crate::tree::{SpanData, TextStyle};

/// The renderer's own regular face — cosmic-text panics when shaping
/// against an empty database, and these tests only need *some* font.
fn font_system() -> FontSystem {
    let mut db = Database::new();
    db.load_font_data(
        include_bytes!("../../../../../assets/fonts/BraiinsSans-Regular.otf").to_vec(),
    );
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}

fn span(text: &str) -> [SpanData; 1] {
    [SpanData {
        text: text.to_owned(),
        weight: None,
        color: None,
        size: None,
        italic: false,
        underline: false,
        strikethrough: false,
    }]
}

/// Draw one slot's text, as `layout_and_render` would: the shared counter
/// ticks once per slot render, then that slot measures its own paragraphs.
fn render_slot(cache: &mut ParagraphLayoutCache, fonts: &mut FontSystem, text: &str) -> u32 {
    cache.begin_frame();
    cache.measure(fonts, &TextStyle::default(), &span(text), None);
    cache.stats().0
}

/// A single widget re-drawing every frame must never re-shape: this is the
/// steady state the paragraph cache exists to produce.
#[test]
fn one_slot_redrawing_every_frame_never_reshapes() {
    let (mut cache, mut fonts) = (ParagraphLayoutCache::new(), font_system());

    assert_eq!(render_slot(&mut cache, &mut fonts, "alpha"), 1, "cold");
    for counter in 2..8 {
        let misses = render_slot(&mut cache, &mut fonts, "alpha");
        assert_eq!(misses, 0, "frame {counter} re-shaped a cached paragraph");
    }
}

/// Two slots alternating through the shared counter: each re-stamps its
/// entries every other increment, so a one-frame retention window evicts
/// whichever drew second-to-last and both re-shape forever.
#[test]
fn two_alternating_slots_evict_each_other_every_frame() {
    let (mut cache, mut fonts) = (ParagraphLayoutCache::new(), font_system());

    render_slot(&mut cache, &mut fonts, "alpha");
    render_slot(&mut cache, &mut fonts, "beta");

    let mut counter = 3;
    for _ in 0..4 {
        for text in ["alpha", "beta"] {
            let misses = render_slot(&mut cache, &mut fonts, text);
            assert_eq!(
                misses, 0,
                "slot '{text}' re-shaped at counter {counter}: the retention \
                 window is narrower than the slot round-robin"
            );
            counter += 1;
        }
    }
}
