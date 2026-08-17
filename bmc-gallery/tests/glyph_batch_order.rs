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

//! Glyphs spread over several atlas pages still draw in submission order.
//!
//! One batch goes out per run of glyphs sharing a page,
//! so a scene whose glyphs alternate pages submits `A, B, A` —
//! and grouping those by page to save texture binds
//! would move the third batch under the second.
//! The scene here overlaps the ink of all three draws, which makes that move
//! visible; the same scene from a cache holding one page is the reference.
//!
//! Lives in the gallery rather than in bmc-render because the question belongs
//! downstream: the caller sees `FemtoVgRenderer` and public colours,
//! not the cache or its keys.

use bmc_render::colors::Color;
use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_render::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};

const W: u32 = 512;
const H: u32 = 256;
const ROW: usize = W as usize;

/// Under femtovg's 92 px direct-path cutoff, so the cache serves every glyph —
/// and large enough that a few dozen of them fill a 512 px page.
const FONT_PX: f32 = 90.0;

/// Sizes drawn before [`FONT_PX`] purely to take up page space.
/// Size is part of the cache key, so the same corpus at another size
/// is a fresh population of glyphs — enough to push the corpus proper
/// across a page boundary.
const FILLER_PX: [f32; 2] = [86.0, 88.0];

const CORPUS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Where the three overlapping draws start, and how far apart they sit.
/// The step is narrower than a glyph at [`FONT_PX`], asserted below:
/// ink that does not overlap would make the comparison pass on anything.
const ORIGIN_X: f32 = 24.0;
const ORIGIN_Y: f32 = 40.0;
const STEP_X: f32 = 30.0;

const FIRST: Color = Color::from_rgb(0xF0, 0x30, 0x30);
const SECOND: Color = Color::from_rgb(0x30, 0xF0, 0x50);
const THIRD: Color = Color::from_rgb(0x50, 0x90, 0xFF);

fn new_renderer(harness: &GlHarness, screen_fbo: u32) -> FemtoVgRenderer {
    // SAFETY: the loader resolves against the context the harness made current
    // on this thread, and nothing else has made another current since.
    unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, screen_fbo, 0) }
        .expect("BUG: renderer init failed")
}

/// Draw the three-glyph scene in the order given, with fixed positions,
/// so only submission order differs between calls.
fn draw_scene(renderer: &mut FemtoVgRenderer, order: [(&str, f32, Color); 3]) {
    renderer.begin_frame(W, H, 1.0);
    for (glyph, x, color) in order {
        renderer.draw_text(glyph, x, ORIGIN_Y, FONT_PX, color);
    }
    renderer.flush();
}

/// The three draws' bounding box, which is where reordering them shows.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the scene's positions are small whole pixel counts"
)]
fn band(frame: &[[u8; 4]]) -> Vec<[u8; 4]> {
    let left = ORIGIN_X as usize;
    let right = (ORIGIN_X + 2.0 * STEP_X + FONT_PX) as usize;
    let top = ORIGIN_Y as usize;
    let bottom = (ORIGIN_Y + 1.4 * FONT_PX) as usize;
    (top..bottom)
        .flat_map(|y| frame[y * ROW + left..y * ROW + right].iter().copied())
        .collect()
}

/// How two frames differ inside the band, or `None` if they do not.
/// Reported rather than compared with `assert_eq!`, which would print
/// every one of the band's fifteen thousand pixels on failure.
fn band_diff(left: &[[u8; 4]], right: &[[u8; 4]]) -> Option<String> {
    let (left, right) = (band(left), band(right));
    let first = left
        .iter()
        .zip(&right)
        .position(|(one, other)| one != other)?;
    let differing = left
        .iter()
        .zip(&right)
        .filter(|(one, other)| one != other)
        .count();
    Some(format!(
        "{differing} of {} band pixels differ, first at offset {first}: {:?} against {:?}",
        left.len(),
        left[first],
        right[first],
    ))
}

/// Rasterize every glyph of `CORPUS` at `size` in its own draw,
/// and report the page each one landed on.
/// One batch per draw, so the record lines up with the corpus.
fn page_per_glyph(
    renderer: &mut FemtoVgRenderer,
    glyphs: &[String],
    size: f32,
) -> Vec<femtovg::ImageId> {
    renderer.begin_frame(W, H, 1.0);
    for glyph in glyphs {
        renderer.draw_text(glyph, 0.0, 0.0, size, FIRST);
    }
    renderer.flush();

    let pages = renderer.pages_touched_last_frame().to_vec();
    assert_eq!(
        renderer.batches_past_record_last_frame(),
        0,
        "BUG: the priming frame outran the batch record, so the pages it \
         reports stop partway through the corpus",
    );
    assert_eq!(
        pages.len(),
        glyphs.len(),
        "BUG: the priming frame did not submit one batch per glyph, so no \
         glyph can be told which page it is on",
    );
    pages
}

#[test]
fn overlapping_glyphs_from_two_pages_draw_in_submission_order() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, screen_fbo) = create_readback_fbo(&harness.gl, W, H);

    let glyphs: Vec<String> = CORPUS.chars().map(|c| c.to_string()).collect();
    let mut churned = new_renderer(&harness, screen_fbo);
    for filler in FILLER_PX {
        page_per_glyph(&mut churned, &glyphs, filler);
    }
    let pages = page_per_glyph(&mut churned, &glyphs, FONT_PX);

    // The scene's premise: two glyphs sharing a page and one somewhere else.
    // Read off the record rather than assumed — a corpus that stopped spanning
    // a page boundary would leave every assertion below trivially satisfied.
    let home = pages[0];
    let same_page = pages
        .iter()
        .skip(1)
        .position(|page| *page == home)
        .map(|index| index + 1)
        .expect("BUG: no second glyph shares the first one's page");
    let other_page = pages
        .iter()
        .position(|page| *page != home)
        .expect("BUG: the corpus no longer spans two pages, so nothing churns");

    let scene = [
        (glyphs[0].as_str(), ORIGIN_X, FIRST),
        (glyphs[other_page].as_str(), ORIGIN_X + STEP_X, SECOND),
        (glyphs[same_page].as_str(), ORIGIN_X + 2.0 * STEP_X, THIRD),
    ];
    // Ink has to overlap for order to reach the pixels at all.
    let advance = churned.measure_text(glyphs[0].as_str(), FONT_PX);
    assert!(
        advance > STEP_X,
        "BUG: glyphs {advance} px wide {STEP_X} px apart do not overlap",
    );

    draw_scene(&mut churned, scene);
    let churned_pages = churned.pages_touched_last_frame();
    assert_eq!(churned_pages.len(), 3, "BUG: expected one batch per draw");
    assert!(
        churned_pages[0] == churned_pages[2] && churned_pages[0] != churned_pages[1],
        "BUG: the scene has to revisit a page — {churned_pages:?} never does, \
         so grouping the batches by page would be a no-op",
    );
    let churned_pixels = read_pixels_top_down(&harness.gl, fbo, W, H);

    // The same scene from a cache that has only ever seen these three glyphs:
    // one page, so there is nothing for a page-grouping pass to reorder.
    let mut single = new_renderer(&harness, screen_fbo);
    draw_scene(&mut single, scene);
    let single_pages = single.pages_touched_last_frame();
    assert!(
        single_pages.iter().all(|page| *page == single_pages[0]),
        "BUG: the reference render is not single-page: {single_pages:?}",
    );
    let single_pixels = read_pixels_top_down(&harness.gl, fbo, W, H);

    let churn_diff = band_diff(&churned_pixels, &single_pixels);
    assert!(
        churn_diff.is_none(),
        "BUG: page churn changed the overlap, so the batches went out \
         reordered — {}",
        churn_diff.unwrap_or_default(),
    );

    // Proof that the comparison above can fail: the page-grouped order
    // (`A, A, B`) is what a batch-merging pass would produce,
    // and it has to read as different pixels.
    draw_scene(&mut single, [scene[0], scene[2], scene[1]]);
    let regrouped_pixels = read_pixels_top_down(&harness.gl, fbo, W, H);
    assert!(
        band_diff(&regrouped_pixels, &single_pixels).is_some(),
        "BUG: the overlap does not record draw order, so comparing it proves \
         nothing",
    );
}
