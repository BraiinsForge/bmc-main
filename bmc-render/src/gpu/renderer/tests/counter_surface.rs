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

use super::FemtoVgRenderer;
use crate::renderer::RenderTarget;
use crate::renderer::prelude::*;
use crate::test_harness::{GlHarness, create_readback_fbo};
use bmc_wasm_protocol::Color;

const W: u32 = 320;
const H: u32 = 320;
const WHITE: Color = Color::from_rgb(255, 255, 255);

/// Wide enough that a few frames of it exhaust the page budget, which is
/// what makes a miss evict instead of finding free space.
const CORPUS: &str = concat!(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "abcdefghijklmnopqrstuvwxyz",
    "0123456789.,:;!?@#$%&*()[]{}<>/+-=_~^",
);

/// One frame per size, because entries made in the current frame are
/// eviction-protected: churning inside one frame drops glyphs instead.
/// A half-pixel step keys distinct entries while keeping every glyph large,
/// so the page budget runs out well inside the loop.
const CHURN_FRAMES: u8 = 48;
const CHURN_TOP_PX: f32 = 92.0;
const CHURN_STEP_PX: f32 = 0.5;

fn text_frame(target: &mut RenderTarget<'_, '_, '_>, text: &str, size: f32) {
    target.begin_frame(W, H, 1.0);
    target.draw_text(text, 4.0, 4.0, size, WHITE);
    target.flush();
}

/// The wrapper's methods are defaulted on the trait, so a missing
/// delegation reports zeros through it while the renderer's own cache is
/// busy — every assertion here separates those two outcomes.
#[test]
fn render_target_reports_the_inner_renderers_text_counters() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let mut target = RenderTarget::new(&mut renderer, None);

    text_frame(&mut target, CORPUS, 32.0);
    text_frame(&mut target, CORPUS, 32.0);

    let warm = target.glyph_cache_counters();
    assert!(warm.misses > 0, "the first frame must miss every glyph");
    assert!(warm.hits > 0, "the repeated frame must hit the same glyphs");

    let layout = target.text_layout_counters();
    assert_eq!(
        layout.layout_cache_shapes, 1,
        "the string must shape exactly once across both frames"
    );
    assert!(
        layout.layout_cache_hits > 0,
        "the repeated frame must reuse the shaped line"
    );
    assert_eq!(layout.layout_cache_peak_entries, 1);
    assert_eq!(layout.layout_cache_capacity_evictions, 0);

    assert!(
        target.text_atlas_resident_bytes() > 0,
        "drawn glyphs must leave resident atlas pages"
    );
    assert!(
        target.glyph_cache_metadata_capacity_bytes() > 0,
        "the metadata ceiling must be observable through the wrapper"
    );

    for frame in 0..CHURN_FRAMES {
        let size = CHURN_TOP_PX - CHURN_STEP_PX * f32::from(frame);
        text_frame(&mut target, CORPUS, size);
        if target.glyph_cache_counters().evictions > 0 {
            break;
        }
    }
    assert!(
        target.glyph_cache_counters().evictions > 0,
        "the churn must outgrow the page budget and evict"
    );
}
