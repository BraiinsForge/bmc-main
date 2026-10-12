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

//! Placement of cached quads and delegated runs: the snapping rule,
//! the contiguous batching fold, and the synthetic-italic transform.

#![expect(clippy::cast_precision_loss)]

use super::font_system;
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache};

use super::{
    DIRECT_PATH_CUTOFF_PX, FAKE_ITALIC_SKEW_DEGREES, FontTable, GlyphCommand,
    baseline_to_alphabetic, build_cached_curved_glyph_commands, build_glyph_commands,
    chunk_oversized, direct_paint, extract_lines, italic_about, outline_glyph_commands, push_quad,
    rasterize_glyph, snap,
};
use crate::gpu::glyph_cache::{GlyphCache, GlyphKey, test_support::MockBackend};

const SANS: &[u8] = include_bytes!("../../../../../assets/fonts/BraiinsSans-Regular.otf");
const DECK_SANS: &[u8] = include_bytes!("../../../../../assets/fonts/BraiinsDeckSans-Regular.otf");

/// Fractional on both axes and in both directions,
/// so a rule that rounded x or truncated y would show.
const ORIGIN_X: f32 = 10.7;
const BASELINE_Y: f32 = 20.4;

/// Never consulted below the cutoff, which is where the quad path lives.
fn no_fonts() -> FontTable {
    FontTable { pairs: Vec::new() }
}

/// femtovg parity: `x.trunc()` but `y.round()` (`text.rs:492-499`).
/// A symmetric rule would drift text by up to a pixel
/// against the placement the direct path still takes from femtovg above 92 px.
#[test]
fn snap_is_trunc_x_round_y() {
    for ((x, y), expected) in [
        ((1.7_f32, 1.4_f32), (1.0_f32, 1.0_f32)),
        ((-1.7, -1.4), (-1.0, -1.0)),
        ((2.3, 2.5), (2.0, 3.0)),
    ] {
        assert_eq!(snap(x, y), expected, "snapping ({x}, {y})");
    }
}

/// Merging every quad of a page into one batch would reorder the line,
/// so a page seen again after another page opens a batch of its own.
#[test]
fn batches_split_on_page_change_only_contiguously() {
    let mut commands = Vec::new();
    for page in [0_usize, 0, 1, 0] {
        push_quad(&mut commands, page, femtovg::Quad::default());
    }

    let shape: Vec<(usize, usize)> = commands
        .iter()
        .map(|command| match command {
            GlyphCommand::Quads { page, quads } => (*page, quads.len()),
            GlyphCommand::Direct(_) => panic!("BUG: the quad fold emitted a direct run"),
        })
        .collect();
    assert_eq!(shape, vec![(0, 2), (1, 1), (0, 1)]);
}

#[test]
fn outline_expands_each_batch_without_repeating_page_commands() {
    let quad = femtovg::Quad {
        x0: 10.0,
        y0: 20.0,
        x1: 14.0,
        y1: 26.0,
        s0: 0.1,
        t0: 0.2,
        s1: 0.3,
        t1: 0.4,
    };
    let commands = vec![GlyphCommand::Quads {
        page: 7,
        quads: vec![quad],
    }];

    let outlined = outline_glyph_commands(&commands, 2);

    let [GlyphCommand::Quads { page, quads }] = outlined.as_slice() else {
        panic!("BUG: one cached batch must remain one outlined batch");
    };
    assert_eq!(*page, 7);
    assert_eq!(quads.len(), 16, "two rings must emit eight offsets each");
    assert_eq!((quads[0].x0, quads[0].y0), (11.0, 20.0));
    assert_eq!((quads[8].x0, quads[8].y0), (12.0, 20.0));
    assert_eq!(
        (quads[15].x0, quads[15].y0),
        (8.0, 22.0),
        "the final copy must be the second ring's bottom-left offset"
    );
    assert_eq!(
        (quads[0].s0, quads[0].t0, quads[0].s1, quads[0].t1),
        (0.1, 0.2, 0.3, 0.4)
    );
}

/// The skew's sign flips through the y-axis conjugation:
/// cosmic skews in font space (y-up), the canvas is y-down.
/// A wrong sign slants the text backwards, a wrong pivot shifts it sideways.
///
/// The oracle is swash's own skewed coverage.
/// 'W' is drawn entirely from straight segments, so its outline points
/// *are* its extremes — a curved glyph's control points would inflate
/// the bounds past the ink.
#[test]
fn italic_transform_matches_swash_skew() {
    let (ox, oy) = (12.0_f32, 34.0_f32);
    let italic = italic_about(ox, oy);
    let tan = FAKE_ITALIC_SKEW_DEGREES.to_radians().tan();
    for (x, y) in [(0.0_f32, 0.0_f32), (20.0, 10.0), (-5.0, 60.0)] {
        let (tx, ty) = italic.transform_point(x, y);
        assert!(
            (tx - (x - (y - oy) * tan)).abs() < 1e-3,
            "({x}, {y}) skewed to x {tx}"
        );
        assert!((ty - y).abs() < 1e-3, "({x}, {y}) moved to y {ty}");
    }

    let mut font_system = font_system();
    let mut swash = SwashCache::new();
    let upright_key = first_glyph_key(&mut font_system, "W", 92.0, false);
    let outline = swash
        .get_outline_commands_uncached(&mut font_system, upright_key)
        .expect("BUG: 'W' has no outline");
    let skewed_key = first_glyph_key(&mut font_system, "W", 92.0, true);
    let skewed = rasterize_glyph(&mut swash, &mut font_system, skewed_key)
        .expect("BUG: skewed 'W' does not rasterize");

    let about_origin = italic_about(0.0, 0.0);
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for point in outline_points(&outline) {
        // Font space is y-up about the baseline; the canvas is y-down.
        let (x, y) = about_origin.transform_point(point.0, -point.1);
        bounds = Some(bounds.map_or((x, x, y, y), |(x0, x1, y0, y1)| {
            (x0.min(x), x1.max(x), y0.min(y), y1.max(y))
        }));
    }
    let (min_x, max_x, min_y, max_y) = bounds.expect("BUG: 'W' outlines no points");

    let width = i32::try_from(skewed.width).expect("BUG: width outside i32");
    let height = i32::try_from(skewed.height).expect("BUG: height outside i32");
    for (transformed, rasterized, edge) in [
        (min_x, skewed.left, "left"),
        (max_x, skewed.left + width, "right"),
        (min_y, -skewed.top, "top"),
        (max_y, height - skewed.top, "bottom"),
    ] {
        assert!(
            (transformed - rasterized as f32).abs() <= 1.0,
            "{edge}: transformed to {transformed}, swash rasterized {rasterized}",
        );
    }
}

/// `set_transform` premultiplies, so the italic matrix alone composes
/// inside whatever transform curved text already applied.
/// Precomposing it with the current transform would apply that one twice.
#[test]
fn italic_delegation_composes_with_outer_transform() {
    // The transform stack lives on `Canvas`, above the renderer,
    // so the void renderer observes the same composition without GL.
    let mut canvas =
        femtovg::Canvas::new(femtovg::renderer::Void).expect("BUG: void canvas creation failed");
    canvas.translate(30.0, 40.0);
    canvas.rotate(0.3);
    let outer = canvas.transform();

    let (gx, gy) = (17.0_f32, 61.0_f32);
    let italic = italic_about(gx, gy);
    canvas.save();
    canvas.set_transform(&italic);
    let mid = canvas.transform();
    assert_eq!(
        mid,
        italic * outer,
        "the italic must apply before the outer"
    );

    let (px, py) = (25.0_f32, 40.0_f32);
    let (ix, iy) = italic.transform_point(px, py);
    let expected = outer.transform_point(ix, iy);
    let actual = mid.transform_point(px, py);
    assert!(
        (actual.0 - expected.0).abs() < 1e-3 && (actual.1 - expected.1).abs() < 1e-3,
        "point {actual:?} against italic-then-outer {expected:?}",
    );

    canvas.restore();
    assert_eq!(canvas.transform(), outer, "restore must undo the italic");
}

/// femtovg fills the unmodified outline, so a `FAKE_ITALIC` glyph batched
/// with its neighbours would render upright above the cutoff;
/// each one has to be submitted alone under its own skew.
/// Fonts break a run for a blunter reason:
/// `fill_glyph_run` takes one font for the whole run.
#[test]
fn oversized_runs_chunk_per_font_and_break_at_fake_italic() {
    // Fonts live in femtovg's text context, not on the canvas,
    // so real `FontId`s cost no GL context here.
    let fonts = femtovg::TextContext::default();
    let sans = fonts
        .add_font_mem(SANS)
        .expect("BUG: font registration failed");
    let deck = fonts
        .add_font_mem(DECK_SANS)
        .expect("BUG: font registration failed");

    let mut font_system = font_system();
    let font_table = FontTable {
        pairs: vec![
            (face_id(&font_system, "BraiinsSans"), sans),
            (face_id(&font_system, "BraiinsDeckSans-Regular"), deck),
        ],
    };
    let size = DIRECT_PATH_CUTOFF_PX + 1.0;
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(size, size));
    buffer.set_rich_text(
        &mut font_system,
        [
            ("AB", named(SANS_FAMILY, false)),
            ("CD", named(DECK_FAMILY, false)),
            ("E", named(SANS_FAMILY, true)),
            ("F", named(SANS_FAMILY, false)),
        ],
        &named(SANS_FAMILY, false),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, false);
    let lines = extract_lines(&buffer);
    let [line] = lines.as_slice() else {
        panic!("BUG: the fixture must shape to one line");
    };

    let runs = chunk_oversized(&line.glyphs, &font_table, ORIGIN_X, BASELINE_Y);
    let shape: Vec<(femtovg::FontId, bool, usize)> = runs
        .iter()
        .map(|run| (run.font_id, run.italic, run.glyphs.len()))
        .collect();
    assert_eq!(
        shape,
        vec![
            (sans, false, 2),
            (deck, false, 2),
            (sans, true, 1),
            (sans, false, 1),
        ],
    );

    let submitted: Vec<(f32, f32)> = runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .map(|glyph| (glyph.x, glyph.y))
        .collect();
    let expected: Vec<(f32, f32)> = line
        .glyphs
        .iter()
        .map(|glyph| snap(ORIGIN_X + glyph.x, BASELINE_Y + glyph.y))
        .collect();
    assert_eq!(submitted, expected, "delegated glyphs must be pre-snapped");

    let mut swash = SwashCache::new();
    let mut cache = GlyphCache::new();
    let commands = build_glyph_commands(
        &mut MockBackend::default(),
        &mut cache,
        &mut swash,
        &mut font_system,
        &font_table,
        &line.glyphs,
        ORIGIN_X,
        BASELINE_Y,
        size,
    );
    let delegated: Vec<(femtovg::FontId, bool, usize)> = commands
        .iter()
        .map(|command| match command {
            GlyphCommand::Direct(run) => (run.font_id, run.italic, run.glyphs.len()),
            GlyphCommand::Quads { .. } => {
                panic!("BUG: above the cutoff no glyph may reach the cache")
            }
        })
        .collect();
    assert_eq!(
        delegated, shape,
        "the draw path must submit the chunker's runs"
    );
}

/// `Paint` defaults to 16 px and femtovg picks the direct path
/// from the paint's size alone,
/// so a colour-only paint would quietly atlas a 93 px glyph.
#[test]
fn direct_paint_carries_the_nominal_size() {
    let paint = femtovg::Paint::color(femtovg::Color::white());
    let size = direct_paint(&paint, 93.0).font_size();
    assert!(
        (size - 93.0).abs() < f32::EPSILON,
        "the direct paint carries {size}, not the nominal 93",
    );
}

/// Curved text places one glyph per arc point, and the origin it submits
/// has to cancel the glyph's position within its line —
/// [`build_cached_curved_glyph_commands`] adds that position back.
/// Without the cancellation every glyph after the first
/// drifts by its own line offset, which a single-glyph fixture cannot see.
/// Asserted on the pen origin the quad reconstructs to,
/// not the quad's centre:
/// side bearings put the ink off centre for perfectly placed glyphs.
///
#[test]
fn curved_glyphs_center_their_advance_on_the_arc() {
    let mut font_system = font_system();
    let mut swash = SwashCache::new();
    let size = 24.0;
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(size, size));
    // Kerned pairs: the second glyph's line position is not the first
    // glyph's advance, so a leaked line position cannot cancel out.
    buffer.set_text(
        &mut font_system,
        "AVATAR",
        &named(SANS_FAMILY, false),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, false);
    let lines = extract_lines(&buffer);
    let [line] = lines.as_slice() else {
        panic!("BUG: the fixture must shape to one line");
    };
    assert!(
        line.glyphs.windows(2).any(|pair| {
            let [left, right] = pair else {
                unreachable!("BUG: windows(2) yields pairs")
            };
            (right.x - left.x - left.w).abs() > f32::EPSILON
        }),
        "the fixture must kern, or a leaked line position cancels out",
    );

    let mut backend = MockBackend::default();
    let mut cache = GlyphCache::new();
    let commands = build_cached_curved_glyph_commands(
        &mut backend,
        &mut cache,
        &mut swash,
        &mut font_system,
        &line.glyphs,
        BASELINE_Y,
    );
    assert_eq!(
        commands.len(),
        line.glyphs.len(),
        "each arc placement must retain its command slot"
    );
    for (glyph, command) in line.glyphs.iter().zip(commands) {
        let Some(GlyphCommand::Quads { quads, .. }) = command else {
            panic!("BUG: one glyph must submit one batch");
        };
        let [quad] = quads.as_slice() else {
            panic!("BUG: one glyph must submit one quad");
        };
        // Normalized, as the cache rasterized it: the raw key's subpixel
        // bins shift the placement by up to a pixel.
        let raster = rasterize_glyph(
            &mut swash,
            &mut font_system,
            GlyphKey::normalize(glyph.key).inner(),
        )
        .expect("BUG: a batched glyph has no coverage");
        let (expected_pen, _) = snap(-glyph.w / 2.0, BASELINE_Y);
        assert!(
            (quad.x0 - raster.left as f32 - expected_pen).abs() < f32::EPSILON,
            "glyph of advance {} drew from pen {}, not {expected_pen}",
            glyph.w,
            quad.x0 - raster.left as f32,
        );
    }
}

/// The cached quad has to land on the snapped origin,
/// with swash's baseline-relative `top` negated into the canvas's y-down
/// space. Getting that sign wrong hangs every glyph below its baseline.
#[test]
fn cached_quads_sit_at_the_snapped_origin() {
    let mut font_system = font_system();
    let mut swash = SwashCache::new();
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(24.0, 24.0));
    buffer.set_text(
        &mut font_system,
        "Hi",
        &named(SANS_FAMILY, false),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, false);
    let lines = extract_lines(&buffer);
    let [line] = lines.as_slice() else {
        panic!("BUG: the fixture must shape to one line");
    };

    let mut backend = MockBackend::default();
    let mut cache = GlyphCache::new();
    let commands = build_glyph_commands(
        &mut backend,
        &mut cache,
        &mut swash,
        &mut font_system,
        &no_fonts(),
        &line.glyphs,
        ORIGIN_X,
        BASELINE_Y,
        24.0,
    );

    let [GlyphCommand::Quads { quads, .. }] = commands.as_slice() else {
        panic!("BUG: one page must yield exactly one batch");
    };
    assert_eq!(quads.len(), line.glyphs.len());

    for (glyph, quad) in line.glyphs.iter().zip(quads) {
        let raster = rasterize_glyph(&mut swash, &mut font_system, glyph.key)
            .expect("BUG: a batched glyph has no coverage");
        let (gx, gy) = snap(ORIGIN_X + glyph.x, BASELINE_Y + glyph.y);
        assert_eq!(
            (quad.x0, quad.y0, quad.x1, quad.y1),
            (
                gx + raster.left as f32,
                gy - raster.top as f32,
                gx + raster.left as f32 + raster.width as f32,
                gy - raster.top as f32 + raster.height as f32,
            ),
        );
    }
}

// ── Boundary goldens ────────────────────────────────────────────

/// The seven faces `build_font_system` loads, in its order.
/// [`build_font_table`] pairs the two libraries positionally,
/// so a fallback glyph resolves only when the whole set is registered.
///
/// [`build_font_table`]: super::build_font_table
const EMBEDDED_FONTS: [&[u8]; 7] = [
    SANS,
    include_bytes!("../../../../../assets/fonts/BraiinsSans-SemiBold.otf"),
    include_bytes!("../../../../../assets/fonts/BraiinsSans-Bold.otf"),
    DECK_SANS,
    include_bytes!("../../../../../assets/fonts/BraiinsDeckSans-SemiBold.otf"),
    include_bytes!("../../../../../assets/fonts/BraiinsDeckSans-Bold.otf"),
    include_bytes!("../../../../../assets/fonts/NotoSans-Regular.ttf"),
];

/// Greek variant letters no Braiins face carries —
/// verified against the embedded cmaps, every one of them shapes to Noto.
/// Plain Greek would not do: the Braiins faces cover it,
/// and a string that never leaves the primary face
/// proves nothing about the fallback.
const GREEK_FALLBACK: &str = "ϖϑϰϱϵ";

/// The cosmic-to-femtovg font pairing `FemtoVgRenderer::new` builds,
/// without its canvas: fonts live in the text context, so real `FontId`s
/// cost no GL context here. The context is returned because the ids index
/// into it.
fn embedded_font_table(font_system: &FontSystem) -> (femtovg::TextContext, FontTable) {
    let fonts = femtovg::TextContext::default();
    let ids = EMBEDDED_FONTS.map(|data| {
        fonts
            .add_font_mem(data)
            .expect("BUG: font registration failed")
    });
    let table = super::build_font_table(font_system, &ids);
    (fonts, table)
}

/// The one visual line `text` shapes to through the embedded faces.
fn shape_line(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    italic: bool,
) -> super::LineGlyphs {
    let mut buffer = Buffer::new(font_system, Metrics::new(size, size));
    buffer.set_text(
        font_system,
        text,
        &named(SANS_FAMILY, italic),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    let mut lines = extract_lines(&buffer);
    assert_eq!(
        lines.len(),
        1,
        "the fixture {text:?} must shape to one line"
    );
    lines.pop().expect("BUG: a one-line fixture has a line")
}

/// Every quad one line submits, through a cache of its own so no raster
/// carries over between the variants under comparison.
fn quads_at(
    font_system: &mut FontSystem,
    swash: &mut SwashCache,
    glyphs: &[super::PositionedGlyphInfo],
    origin_x: f32,
    alphabetic_y: f32,
    size: f32,
) -> Vec<femtovg::Quad> {
    build_glyph_commands(
        &mut MockBackend::default(),
        &mut GlyphCache::new(),
        swash,
        font_system,
        &no_fonts(),
        glyphs,
        origin_x,
        alphabetic_y,
        size,
    )
    .into_iter()
    .flat_map(|command| match command {
        GlyphCommand::Quads { quads, .. } => quads,
        GlyphCommand::Direct(_) => panic!("BUG: below the cutoff nothing may delegate"),
    })
    .collect()
}

/// The coverage the cache stores for a glyph — keyed with the subpixel bins
/// normalized away, which is what makes one raster serve every origin.
fn normalized_raster(
    swash: &mut SwashCache,
    font_system: &mut FontSystem,
    glyph: &super::PositionedGlyphInfo,
) -> crate::gpu::glyph_cache::RasterGlyph {
    rasterize_glyph(swash, font_system, GlyphKey::normalize(glyph.key).inner())
        .expect("BUG: a cached glyph has no coverage")
}

/// A fractional origin may move a glyph only by the whole pixels it snaps
/// to: the quad stays on the integer grid, within a pixel of the exact
/// position, off a raster the origin never reaches. Sub-pixel drift is what
/// makes a string shimmer as an animation slides it across the screen.
#[test]
fn fractional_origins_snap_consistently() {
    let mut font_system = font_system();
    let mut swash = SwashCache::new();
    let size = 24.0;
    // No spaces: an empty box submits no quad, and this compares per glyph.
    let line = shape_line(&mut font_system, "Wij.", size, false);
    let rasters: Vec<_> = line
        .glyphs
        .iter()
        .map(|glyph| normalized_raster(&mut swash, &mut font_system, glyph))
        .collect();

    let mut reference: Option<Vec<femtovg::Quad>> = None;
    let mut snapped_apart = false;
    for (origin_x, baseline_y) in [
        (10.0_f32, 20.0_f32),
        (10.4, 20.4),
        (10.9, 20.9),
        (-10.6, -20.6),
    ] {
        let quads = quads_at(
            &mut font_system,
            &mut swash,
            &line.glyphs,
            origin_x,
            baseline_y,
            size,
        );
        assert_eq!(
            quads.len(),
            line.glyphs.len(),
            "every fixture glyph must cache a quad"
        );

        for ((glyph, raster), quad) in line.glyphs.iter().zip(&rasters).zip(&quads) {
            assert_eq!(
                (quad.x0.fract(), quad.y0.fract()),
                (0.0, 0.0),
                "quad ({}, {}) left the integer grid",
                quad.x0,
                quad.y0,
            );
            let exact_x = origin_x + glyph.x + raster.left as f32;
            let exact_y = baseline_y + glyph.y - raster.top as f32;
            assert!(
                (quad.x0 - exact_x).abs() < 1.0,
                "x snapped to {} from {exact_x}",
                quad.x0,
            );
            assert!(
                (quad.y0 - exact_y).abs() <= 0.5,
                "y snapped to {} from {exact_y}",
                quad.y0,
            );
        }

        match &reference {
            None => reference = Some(quads),
            Some(first) => {
                for (quad, base) in quads.iter().zip(first) {
                    assert_eq!(
                        (
                            quad.x1 - quad.x0,
                            quad.y1 - quad.y0,
                            quad.s0,
                            quad.t0,
                            quad.s1,
                            quad.t1
                        ),
                        (
                            base.x1 - base.x0,
                            base.y1 - base.y0,
                            base.s0,
                            base.t0,
                            base.s1,
                            base.t1
                        ),
                        "the origin reached the raster",
                    );
                    snapped_apart |= (quad.x0 - base.x0).abs() > 0.5;
                }
            }
        }
    }
    assert!(
        snapped_apart,
        "the origins must snap apart, or nothing is being compared"
    );
}

/// The four femtovg anchors must move a line by the offsets the layout's
/// own ascent and descent describe. Deriving them from `line_y − line_top`
/// instead folds in half the leading, which only a non-alphabetic anchor
/// ever shows.
#[test]
fn all_baselines_place_against_cosmic_metrics() {
    let mut font_system = font_system();
    let mut swash = SwashCache::new();
    let size = 24.0;
    let line = shape_line(&mut font_system, "Hxg", size, false);
    assert!(
        line.max_ascent > 1.0 && line.max_descent > 1.0,
        "the metrics must separate the four anchors"
    );
    let anchor_y = 40.0_f32;

    let mut quads_for = |font_system: &mut FontSystem, baseline| {
        let alphabetic_y =
            baseline_to_alphabetic(anchor_y, baseline, line.max_ascent, line.max_descent);
        quads_at(
            font_system,
            &mut swash,
            &line.glyphs,
            ORIGIN_X,
            alphabetic_y,
            size,
        )
    };

    let alphabetic = quads_for(&mut font_system, femtovg::Baseline::Alphabetic);
    for (baseline, expected) in [
        (femtovg::Baseline::Top, line.max_ascent),
        (femtovg::Baseline::Bottom, -line.max_descent),
        (
            femtovg::Baseline::Middle,
            (line.max_ascent - line.max_descent) / 2.0,
        ),
    ] {
        for (quad, base) in quads_for(&mut font_system, baseline)
            .iter()
            .zip(&alphabetic)
        {
            let moved = quad.y0 - base.y0;
            assert!(
                (moved - expected).abs() <= 1.0,
                "{baseline:?} moved the line by {moved}, not the metric-derived {expected}",
            );
        }
    }
}

/// Where the cache hands over to femtovg's direct path the pen has to keep
/// walking: a glyph at 93 px must sit where its 92 px twin sits, scaled.
/// The two sizes are separate layouts, so what is compared is each glyph's
/// displacement from the pen origin per pixel of nominal size.
#[test]
fn cutoff_is_continuous() {
    let mut font_system = font_system();
    let mut swash = SwashCache::new();
    let (_fonts, font_table) = embedded_font_table(&font_system);
    let cached_size = DIRECT_PATH_CUTOFF_PX;
    let direct_size = DIRECT_PATH_CUTOFF_PX + 1.0;

    for (text, italic) in [("AVAWij", false), ("AVAWij", true), (GREEK_FALLBACK, false)] {
        let cached = shape_line(&mut font_system, text, cached_size, italic);
        let direct = shape_line(&mut font_system, text, direct_size, italic);
        assert_eq!(
            cached.glyphs.len(),
            direct.glyphs.len(),
            "the two sizes must shape {text:?} to the same glyphs"
        );

        let quads = quads_at(
            &mut font_system,
            &mut swash,
            &cached.glyphs,
            ORIGIN_X,
            BASELINE_Y,
            cached_size,
        );
        assert_eq!(
            quads.len(),
            cached.glyphs.len(),
            "every fixture glyph must cache a quad"
        );
        let submitted: Vec<femtovg::PositionedGlyph> =
            chunk_oversized(&direct.glyphs, &font_table, ORIGIN_X, BASELINE_Y)
                .into_iter()
                .flat_map(|run| run.glyphs)
                .collect();
        assert_eq!(submitted.len(), direct.glyphs.len());

        for ((glyph, quad), placed) in cached.glyphs.iter().zip(&quads).zip(&submitted) {
            let raster = normalized_raster(&mut swash, &mut font_system, glyph);
            for (axis, below, above) in [
                (
                    "x",
                    quad.x0 - raster.left as f32 - ORIGIN_X,
                    placed.x - ORIGIN_X,
                ),
                (
                    "y",
                    quad.y0 + raster.top as f32 - BASELINE_Y,
                    placed.y - BASELINE_Y,
                ),
            ] {
                let jump = (below / cached_size - above / direct_size) * cached_size;
                assert!(
                    jump.abs() < 1.0,
                    "{text:?} italic={italic}: {axis} jumps {jump} px across the cutoff",
                );
            }
        }
    }
}

/// The delegated path must submit cosmic's positions, not femtovg's.
/// `gpu::text`'s header documents that the two shapers kern differently
/// but names no pair, so the pair is discovered here:
/// the first candidate the two disagree on at 93 px
/// is the only one whose placement can tell the shapers apart.
/// A font swap that made every candidate agree
/// fails the assertion rather than quietly testing nothing.
#[test]
fn delegated_runs_carry_cosmic_kerning() {
    let size = DIRECT_PATH_CUTOFF_PX + 1.0;
    let mut font_system = font_system();
    let (_, font_table) = embedded_font_table(&font_system);
    let divergent = super::divergent_kerning_pair(&mut font_system, size);

    let line = shape_line(&mut font_system, divergent, size, false);
    let submitted: Vec<(f32, f32)> =
        chunk_oversized(&line.glyphs, &font_table, ORIGIN_X, BASELINE_Y)
            .into_iter()
            .flat_map(|run| run.glyphs)
            .map(|glyph| (glyph.x, glyph.y))
            .collect();
    let expected: Vec<(f32, f32)> = line
        .glyphs
        .iter()
        .map(|glyph| snap(ORIGIN_X + glyph.x, BASELINE_Y + glyph.y))
        .collect();
    assert_eq!(
        submitted, expected,
        "{divergent:?} was delegated on femtovg's own advances",
    );
}

const SANS_FAMILY: &str = "Braiins Sans";
const DECK_FAMILY: &str = "Braiins Deck Sans";

fn named(family: &'static str, italic: bool) -> Attrs<'static> {
    let attrs = Attrs::new().family(Family::Name(family));
    if italic {
        attrs.style(Style::Italic)
    } else {
        attrs
    }
}

/// Every point an outline names, in canvas space (y-down).
fn outline_points(commands: &[cosmic_text::Command]) -> Vec<(f32, f32)> {
    commands
        .iter()
        .flat_map(|command| match *command {
            cosmic_text::Command::MoveTo(p) | cosmic_text::Command::LineTo(p) => vec![p],
            cosmic_text::Command::QuadTo(c, p) => vec![c, p],
            cosmic_text::Command::CurveTo(c0, c1, p) => vec![c0, c1, p],
            cosmic_text::Command::Close => Vec::new(),
        })
        .map(|point| (point.x, point.y))
        .collect()
}

fn face_id(font_system: &FontSystem, post_script_name: &str) -> cosmic_text::fontdb::ID {
    font_system
        .db()
        .faces()
        .find(|face| face.post_script_name == post_script_name)
        .expect("BUG: the embedded set has no such face")
        .id
}

/// The key of the first glyph a single-word line shapes to,
/// so the face, weight and flags are the ones cosmic really produces.
fn first_glyph_key(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    italic: bool,
) -> cosmic_text::CacheKey {
    let mut buffer = Buffer::new(font_system, Metrics::new(size, size));
    buffer.set_text(
        font_system,
        text,
        &named(SANS_FAMILY, italic),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    let lines = extract_lines(&buffer);
    lines
        .first()
        .and_then(|line| line.glyphs.first())
        .expect("BUG: the fixture shaped no glyph")
        .key
}
