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

//! Glyph rasterisation and the commands that draw glyph runs, cached or direct.

#![expect(clippy::cast_precision_loss)]

use super::shaping::PositionedGlyphInfo;
use crate::gpu::glyph_cache::{
    GlyphCache, GlyphLookup, GlyphQuad, PageBackend, PageCreateFailed, PageFaultKind, RasterGlyph,
};
use crate::gpu::renderer::FACES;
use bmc_ui::TextStyle;
use cosmic_text::FontSystem;
use femtovg::{Canvas, FontId, Paint, renderer::OpenGl};
use rgb::FromSlice as _;

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

/// Rasterize one glyph into 8-bit alpha coverage.
///
/// Deliberately `get_image_uncached`:
/// cosmic-text's `get_image` memoizes into an unbounded map,
/// the very growth this cache exists to replace.
/// `None` covers everything with no coverage to upload — a missing font,
/// a colour or subpixel bitmap, an empty box such as a space.
pub(crate) fn rasterize_glyph(
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    key: cosmic_text::CacheKey,
) -> Option<RasterGlyph> {
    let image = swash.get_image_uncached(font_system, key)?;
    if image.content != cosmic_text::SwashContent::Mask {
        return None;
    }

    // The single u32 → usize boundary: swash sizes glyphs in u32, the cache
    // and everything under it in usize.
    let width = usize::try_from(image.placement.width).expect("BUG: swash dimension exceeds usize");
    let height =
        usize::try_from(image.placement.height).expect("BUG: swash dimension exceeds usize");
    if width * height == 0 {
        return None;
    }
    assert_eq!(
        image.data.len(),
        width * height,
        "BUG: swash mask is not a tightly packed bitmap of its placement"
    );

    Some(RasterGlyph {
        width,
        height,
        left: image.placement.left,
        top: image.placement.top,
        coverage: image.data,
    })
}

/// femtovg's own direct-path check (`lib.rs:1455`).
/// Above it femtovg path-renders per frame instead of atlasing,
/// so the cache owns exactly the range femtovg would have atlased.
pub(crate) const DIRECT_PATH_CUTOFF_PX: f32 = 92.0;

/// The skew cosmic-text synthesizes italic with (`swash.rs:68`);
/// no shipped face has a true italic.
pub(super) const FAKE_ITALIC_SKEW_DEGREES: f32 = 14.0;

/// cosmic-text's per-glyph face choice,
/// translated into femtovg's handle for the same font binary.
pub(crate) struct FontTable {
    pub(super) pairs: Vec<(cosmic_text::fontdb::ID, FontId)>,
}

impl FontTable {
    pub(crate) fn femtovg_font(&self, id: cosmic_text::fontdb::ID) -> FontId {
        self.pairs
            .iter()
            .find_map(|&(face, font)| (face == id).then_some(font))
            .expect("BUG: a glyph came from a face outside the loaded font set")
    }
}

/// Pair each cosmic-text face, positionally,
/// with the femtovg font registered from the same mapping.
///
/// femtovg's [`FontId`] is opaque, so no pair can be *queried* for correctness —
/// it holds by construction, both libraries being fed the same seven mappings
/// in the same order. The check against [`FACES`] is unconditional:
/// `faces()` iterates fontdb's private slot map, and a release build whose order
/// had silently diverged would map glyphs to the wrong fonts forever after.
/// Seven faces once at init cost nothing.
pub(crate) fn build_font_table(
    font_system: &FontSystem,
    femtovg_ids: &[FontId; FACES.len()],
) -> FontTable {
    let faces: Vec<_> = font_system.db().faces().collect();
    assert_eq!(faces.len(), FACES.len(), "BUG: font table order diverged");
    for (face, expected) in faces.iter().zip(&FACES) {
        assert_eq!(
            (face.post_script_name.as_str(), face.weight.0),
            (expected.post_script_name, expected.weight),
            "BUG: font table order diverged"
        );
    }

    FontTable {
        pairs: faces
            .iter()
            .map(|face| face.id)
            .zip(femtovg_ids.iter().copied())
            .collect(),
    }
}

/// Snap a glyph origin the way femtovg does (`text.rs:492-499`):
/// x truncated, y rounded.
/// Applied once in layout space, before any canvas transform,
/// so cached quads and delegated runs land on the same coordinates.
pub(crate) fn snap(x: f32, y: f32) -> (f32, f32) {
    (x.trunc(), y.round())
}

/// cosmic-text's synthetic italic, in canvas space,
/// about a glyph's baseline origin.
/// cosmic skews `x' = x + y·tan(14°)` in font space (y-up);
/// the canvas is y-down, so the conjugation flips the sign
/// to `x' = x − (y − oy)·tan(14°)`.
pub(super) fn italic_about(ox: f32, oy: f32) -> femtovg::Transform2D {
    let mut transform = femtovg::Transform2D::translation(-ox, -oy);
    transform.skew_x(-FAKE_ITALIC_SKEW_DEGREES.to_radians());
    transform.translate(ox, oy);
    transform
}

/// One `fill_glyph_run` submission: contiguous glyphs sharing a font,
/// or a lone `FAKE_ITALIC` glyph the caller draws under its own skew.
#[derive(Debug)]
pub(crate) struct OversizedRun {
    pub font_id: FontId,
    pub italic: bool,
    pub glyphs: Vec<femtovg::PositionedGlyph>,
}

/// What one line's glyphs submit to, in draw order.
pub(crate) enum GlyphCommand<P> {
    Quads { page: P, quads: Vec<femtovg::Quad> },
    Direct(OversizedRun),
}

/// Split an oversized line into the runs femtovg can take:
/// one per font, and one per `FAKE_ITALIC` glyph, which femtovg's direct path
/// would otherwise fill from the unmodified outline and render upright.
pub(crate) fn chunk_oversized(
    glyphs: &[PositionedGlyphInfo],
    font_table: &FontTable,
    origin_x: f32,
    alphabetic_y: f32,
) -> Vec<OversizedRun> {
    let mut runs: Vec<OversizedRun> = Vec::new();
    for glyph in glyphs {
        let (x, y) = snap(origin_x + glyph.x, alphabetic_y + glyph.y);
        let font_id = font_table.femtovg_font(glyph.font_id);
        let italic = glyph
            .flags
            .contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC);
        let positioned = femtovg::PositionedGlyph {
            x,
            y,
            glyph_id: glyph.glyph_id,
        };

        match runs.last_mut() {
            Some(run) if !italic && !run.italic && run.font_id == font_id => {
                run.glyphs.push(positioned);
            }
            Some(_) | None => runs.push(OversizedRun {
                font_id,
                italic,
                glyphs: vec![positioned],
            }),
        }
    }
    runs
}

/// Extend the batch in flight while its page holds; a page change closes it.
/// Grouping every quad of a page together instead would reorder the line,
/// letting a later glyph draw under an earlier one.
pub(super) fn push_quad<P: Copy + Eq>(
    commands: &mut Vec<GlyphCommand<P>>,
    page: P,
    quad: femtovg::Quad,
) {
    match commands.last_mut() {
        Some(GlyphCommand::Quads { page: open, quads }) if *open == page => quads.push(quad),
        Some(GlyphCommand::Quads { .. } | GlyphCommand::Direct(_)) | None => {
            commands.push(GlyphCommand::Quads {
                page,
                quads: vec![quad],
            });
        }
    }
}

/// swash reports `top` as the coverage's height above the baseline,
/// so the canvas-space top edge is its negation.
fn glyph_quad<P>(cached: &GlyphQuad<P>, gx: f32, gy: f32) -> femtovg::Quad {
    let x0 = gx + cached.placement.left as f32;
    let y0 = gy - cached.placement.top as f32;
    femtovg::Quad {
        x0,
        y0,
        x1: x0 + cached.placement.width as f32,
        y1: y0 + cached.placement.height as f32,
        s0: cached.u0,
        t0: cached.v0,
        s1: cached.u1,
        t1: cached.v1,
    }
}

fn cached_glyph_quad<P>(
    backend: &mut impl PageBackend<PageId = P>,
    cache: &mut GlyphCache<P>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    glyph: &PositionedGlyphInfo,
    origin_x: f32,
    alphabetic_y: f32,
) -> Option<(P, femtovg::Quad)>
where
    P: Copy + Eq + core::fmt::Debug,
{
    let (gx, gy) = snap(origin_x + glyph.x, alphabetic_y + glyph.y);
    let lookup = cache.get_or_insert(backend, glyph.key, |key| {
        rasterize_glyph(swash, font_system, key.inner())
    });
    match lookup {
        GlyphLookup::Resident(cached) => Some((cached.page, glyph_quad(&cached, gx, gy))),
        GlyphLookup::Missing | GlyphLookup::Oversized | GlyphLookup::Dropped => None,
    }
}

/// Build what one line submits, without submitting it.
///
/// Split out from [`draw_line_glyphs`] because femtovg's submission calls
/// return nothing: every placement assertion — snapping, baselines, the cutoff —
/// is made against this output, which `draw_line_glyphs` submits exactly.
#[expect(clippy::too_many_arguments, reason = "one line's full draw state")]
pub(crate) fn build_glyph_commands<P>(
    backend: &mut impl PageBackend<PageId = P>,
    cache: &mut GlyphCache<P>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    font_table: &FontTable,
    glyphs: &[PositionedGlyphInfo],
    origin_x: f32,
    alphabetic_y: f32,
    font_size: f32,
) -> Vec<GlyphCommand<P>>
where
    P: Copy + Eq + core::fmt::Debug,
{
    if font_size > DIRECT_PATH_CUTOFF_PX {
        return chunk_oversized(glyphs, font_table, origin_x, alphabetic_y)
            .into_iter()
            .map(GlyphCommand::Direct)
            .collect();
    }

    let mut commands = Vec::new();
    for glyph in glyphs {
        if let Some((page, quad)) = cached_glyph_quad(
            backend,
            cache,
            swash,
            font_system,
            glyph,
            origin_x,
            alphabetic_y,
        ) {
            push_quad(&mut commands, page, quad);
        }
    }
    commands
}

/// Prepare one cached command per curved glyph before its individual transform.
/// Missing glyphs retain their slot so commands stay aligned with arc placements.
pub(crate) fn build_cached_curved_glyph_commands<P>(
    backend: &mut impl PageBackend<PageId = P>,
    cache: &mut GlyphCache<P>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    glyphs: &[PositionedGlyphInfo],
    alphabetic_y: f32,
) -> Vec<Option<GlyphCommand<P>>>
where
    P: Copy + Eq + core::fmt::Debug,
{
    glyphs
        .iter()
        .map(|glyph| {
            cached_glyph_quad(
                backend,
                cache,
                swash,
                font_system,
                glyph,
                curved_glyph_origin_x(glyph),
                alphabetic_y,
            )
            .map(|(page, quad)| GlyphCommand::Quads {
                page,
                quads: vec![quad],
            })
        })
        .collect()
}

/// Expand cached batches for a solid-colour outline.
/// All copies share one paint, so grouping offsets within each page batch
/// preserves the source-over result without allocating one vector per offset.
pub(crate) fn outline_glyph_commands<P: Copy>(
    commands: &[GlyphCommand<P>],
    rings: u32,
) -> Vec<GlyphCommand<P>> {
    assert!(rings > 0, "BUG: an outline must contain at least one ring");
    let repeats = usize::try_from(rings)
        .expect("BUG: outline ring count exceeds usize")
        .checked_mul(8)
        .expect("BUG: outline repeat count overflows usize");
    let mut outlined = Vec::with_capacity(commands.len());

    for command in commands {
        let GlyphCommand::Quads { page, quads } = command else {
            panic!("BUG: direct-path commands cannot be expanded as cached outline quads");
        };
        let capacity = quads
            .len()
            .checked_mul(repeats)
            .expect("BUG: outline quad count overflows usize");
        let mut repeated = Vec::with_capacity(capacity);
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
                repeated.extend(quads.iter().map(|quad| femtovg::Quad {
                    x0: quad.x0 + dx,
                    y0: quad.y0 + dy,
                    x1: quad.x1 + dx,
                    y1: quad.y1 + dy,
                    ..*quad
                }));
            }
        }
        outlined.push(GlyphCommand::Quads {
            page: *page,
            quads: repeated,
        });
    }

    outlined
}

/// The paint femtovg's direct path needs.
/// The incoming paint carries colour only, and `Paint` defaults to 16 px
/// (`paint.rs:281-288`) — femtovg chooses path rendering over its own atlas
/// by the paint's font size alone (`lib.rs:1455`), so the colour-only paint
/// would populate that atlas instead.
pub(super) fn direct_paint(paint: &Paint, font_size: f32) -> Paint {
    let mut direct = paint.clone();
    direct.set_font_size(font_size);
    direct
}

/// Draw one line's glyphs, cached below the cutoff and delegated above it.
///
/// Takes a slice rather than a [`super::LineGlyphs`] so callers can submit a subgroup
/// without building a temporary vector on every warm frame.
#[expect(clippy::too_many_arguments, reason = "one line's full draw state")]
pub(crate) fn draw_line_glyphs(
    canvas: &mut Canvas<OpenGl>,
    cache: &mut GlyphCache<femtovg::ImageId>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    font_table: &FontTable,
    glyphs: &[PositionedGlyphInfo],
    origin_x: f32,
    alphabetic_y: f32,
    paint: &Paint,
    font_size: f32,
) {
    let commands = build_glyph_commands(
        &mut FemtovgPages { canvas },
        cache,
        swash,
        font_system,
        font_table,
        glyphs,
        origin_x,
        alphabetic_y,
        font_size,
    );

    submit_glyph_commands(canvas, cache, commands, paint, font_size);
}

/// Submit prepared commands in one cached batch or as direct-path runs.
pub(crate) fn submit_glyph_commands<I>(
    canvas: &mut Canvas<OpenGl>,
    #[cfg_attr(not(feature = "atlas-inspect"), expect(unused_variables))] cache: &mut GlyphCache<
        femtovg::ImageId,
    >,
    commands: I,
    paint: &Paint,
    font_size: f32,
) where
    I: IntoIterator<Item = GlyphCommand<femtovg::ImageId>>,
    I::IntoIter: ExactSizeIterator,
{
    let commands = commands.into_iter();
    if font_size > DIRECT_PATH_CUTOFF_PX {
        let mut oversized_paint = None;
        for command in commands {
            match command {
                GlyphCommand::Quads { .. } => {
                    panic!("BUG: direct-path text produced a cached-quad command");
                }
                GlyphCommand::Direct(run) => {
                    let paint =
                        oversized_paint.get_or_insert_with(|| direct_paint(paint, font_size));
                    if run.italic {
                        debug_assert_eq!(
                            run.glyphs.len(),
                            1,
                            "BUG: an italic run's skew pivots on its one glyph"
                        );
                        let pivot = run
                            .glyphs
                            .first()
                            .expect("BUG: an oversized run carries no glyph");
                        let italic = italic_about(pivot.x, pivot.y);
                        canvas.save();
                        canvas.set_transform(&italic);
                        direct_path::fill_oversized_run(canvas, run.font_id, run.glyphs, paint);
                        canvas.restore();
                    } else {
                        direct_path::fill_oversized_run(canvas, run.font_id, run.glyphs, paint);
                    }
                }
            }
        }
        return;
    }

    let mut alpha_glyphs = Vec::with_capacity(commands.len());
    for command in commands {
        let GlyphCommand::Quads { page, quads } = command else {
            panic!("BUG: atlas-range text produced a direct-path command");
        };
        #[cfg(feature = "atlas-inspect")]
        cache.record_drawn_page(page);
        alpha_glyphs.push(femtovg::DrawCommand {
            image_id: page,
            quads,
        });
    }
    if !alpha_glyphs.is_empty() {
        canvas.draw_glyph_commands(
            femtovg::GlyphDrawCommands {
                alpha_glyphs,
                color_glyphs: Vec::new(),
            },
            paint,
        );
    }
}

/// Origin that centers one glyph's advance on the arc point it is drawn at.
///
/// The `− x` cancels the glyph's position within its line, which
/// [`draw_line_glyphs`] adds back; without it every glyph after the first
/// is displaced by that position on top of its arc placement.
pub(crate) fn curved_glyph_origin_x(glyph: &PositionedGlyphInfo) -> f32 {
    -glyph.w / 2.0 - glyph.x
}

mod direct_path {
    use femtovg::{Canvas, FontId, Paint, PositionedGlyph, renderer::OpenGl};

    /// The one femtovg direct-path entry point. Everything
    /// > 92 px nominal routes here; femtovg path-renders it per frame.
    pub(super) fn fill_oversized_run(
        canvas: &mut Canvas<OpenGl>,
        font_id: FontId,
        glyphs: impl IntoIterator<Item = PositionedGlyph>,
        paint: &Paint,
    ) {
        debug_assert!(
            paint.font_size() > 92.0,
            "BUG: direct path for atlas-range size"
        );
        #[expect(
            clippy::disallowed_methods,
            reason = "sole >92px direct-path delegation"
        )]
        let _ = canvas.fill_glyph_run(font_id, glyphs, paint);
    }
}

/// The atlas pages, backed by femtovg images.
/// Dimensions stay `usize` end to end: femtovg's image calls take `usize`,
/// and so does [`PageBackend`].
pub(crate) struct FemtovgPages<'a> {
    pub(crate) canvas: &'a mut Canvas<OpenGl>,
}

impl PageBackend for FemtovgPages<'_> {
    type PageId = femtovg::ImageId;

    fn create_page(&mut self, size_px: usize) -> Result<Self::PageId, PageCreateFailed> {
        self.canvas
            .create_image_empty(
                size_px,
                size_px,
                femtovg::PixelFormat::Gray8,
                femtovg::ImageFlags::NEAREST,
            )
            .map_err(|_| PageCreateFailed)
    }

    fn upload(
        &mut self,
        page: Self::PageId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[u8],
    ) -> Result<(), PageFaultKind> {
        // `as_gray` is a slice cast over a transparent wrapper,
        // so the padded buffer reaches the driver without a copy.
        let coverage = imgref::ImgRef::new(pixels.as_gray(), width, height);
        self.canvas
            .update_image(page, femtovg::ImageSource::Gray(coverage), x, y)
            .map_err(|error| upload_fault(&error))
    }
}

/// A rect that leaves the page, a format mismatch or an unknown image
/// are all bugs in what we asked for, and quarantining the page is what stops
/// the cache repeating them; anything else is the driver having a bad frame.
fn upload_fault(error: &femtovg::ErrorKind) -> PageFaultKind {
    if matches!(
        error,
        femtovg::ErrorKind::ImageUpdateOutOfBounds
            | femtovg::ErrorKind::ImageUpdateWithDifferentFormat
            | femtovg::ErrorKind::ImageIdNotFound
    ) {
        PageFaultKind::Invariant
    } else {
        PageFaultKind::Transient
    }
}

/// Draw underline / strikethrough for a text segment.
/// `baseline_y` is the text baseline (from cosmic-text `line_y`).
pub(super) fn draw_decorations_for_segment(
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
