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

//! Screen region one dynamic canvas draw can occupy.
//!
//! Damage tracking repaints only the rectangles the walk reports, and a canvas
//! is a single layout leaf — so reporting the node's rect repaints a whole
//! viewport-sized canvas to move one animated draw. This module bounds the
//! single draw instead.
//!
//! The bound must hold for **every** frame the rectangle is used in, not just
//! the one it was computed from: damage is consumed a frame late and spans the
//! export buffer rotation. So it is taken over the whole *range* each transform
//! can reach while the guest is idle — an animation's `from..to`, a
//! transition's recorded `from..target` — rather than this frame's value. Those
//! ranges only change on a guest frame, which repaints in full.
//!
//! Every bound over-approximates: over-covering costs pixels, under-covering
//! leaves trails of stale content. Anything unboundable — a drop shadow, an
//! orbit whose angle moves, unmeasured text extents, a transition with no
//! recorded state — returns `None` and falls back to the whole canvas.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::components::draw::{extract_draw_values, get_draw_bounds};
use crate::interaction::Rect;
use crate::renderer::SvgContent;
use crate::tree::DrawCommand;
use crate::{TransitionState, TransitionStateKey};
use bmc_wasm_protocol::{AnimProperty, Easing, SvgId};

/// The accumulated transform range between the canvas and a leaf draw.
///
/// Mirrors what `render_draw_inner` accumulates on the way down —
/// a canvas-relative placement and a translate range, a uniform scale about
/// the leaf's own box, and a rotation about the canvas centre —
/// except that each moving part is a range rather than this frame's value.
#[derive(Clone, Copy)]
struct Transform {
    translate_x: (f32, f32),
    translate_y: (f32, f32),
    /// Where an enclosing `Centered`/`Orbit` placed this draw — see
    /// [`Transform::set_layout`] for why it is not folded into the ranges above.
    layout_x: f32,
    layout_y: f32,
    /// Every factor reachable, signed, as the product of every scale's range.
    /// A shrink keeps the draw inside its box,
    /// but pulls a region painted within it towards the box centre.
    scale_range: (f32, f32),
    /// `None` until something can rotate this draw, kept distinct from a zero
    /// range so a still draw skips the sweep: its polar round-trip costs a pixel
    /// of precision, which is slack worth paying only when something moves.
    rotation: Option<(f32, f32)>,
    /// Extra reach on every side, for a stroke the leaf box leaves out:
    /// an icon's, or an arc's that is thicker somewhere in its transition
    /// than in the tree the leaf box was read from.
    margin: f32,
}

/// Which corner of a leaf box stays put as scale grows it.
///
/// Most leaves scale about their centre, as `render_draw_inner` does with
/// `sx = x + (w - w * scale) / 2`. [`DrawCommand::AutofitText`] does not:
/// it scales its box straight off `(x, y)`, and a centred bound then
/// leaves the far edges out.
#[derive(Clone, Copy)]
enum ScaleAnchor {
    Centre,
    TopLeft,
}

impl Transform {
    const IDENTITY: Self = Self {
        translate_x: (0.0, 0.0),
        translate_y: (0.0, 0.0),
        layout_x: 0.0,
        layout_y: 0.0,
        scale_range: (1.0, 1.0),
        rotation: None,
        margin: 0.0,
    };

    /// Compose an additive range onto an existing one — the reachable set of a
    /// sum of independent ranges is the sum of their bounds.
    fn add_translate(mut self, dx: (f32, f32), dy: (f32, f32)) -> Self {
        self.translate_x = (self.translate_x.0 + dx.0, self.translate_x.1 + dx.1);
        self.translate_y = (self.translate_y.0 + dy.0, self.translate_y.1 + dy.1);
        self
    }

    /// Replace the placement, for a draw positioned against the canvas rather
    /// than from where its parent put it.
    ///
    /// The renderer's `DrawOffset` (`components/draw/mod.rs`) keeps layout and
    /// translate apart, and the bound has to agree: replacing the translate
    /// range here would predict a rect the paint never touches, and adding to
    /// the layout would double a placement the renderer applies once.
    fn set_layout(mut self, dx: f32, dy: f32) -> Self {
        self.layout_x = dx;
        self.layout_y = dy;
        self
    }

    fn add_rotation(mut self, range: (f32, f32)) -> Self {
        // An all-zero range is no rotation at all, and adding it would send a
        // still draw through the sweep for nothing.
        if range.0.abs() + range.1.abs() == 0.0 {
            return self;
        }
        let (lo, hi) = self.rotation.unwrap_or((0.0, 0.0));
        self.rotation = Some((lo + range.0, hi + range.1));
        self
    }

    /// Compose the reachable scale factors `lo..=hi` onto the accumulated ones.
    ///
    /// Multiplicative, because the renderer composes simultaneous scales that
    /// way: two 1 → 2 scales on one draw reach 4x, and taking the larger would
    /// leave everything past 2x unrepainted. Independent ranges multiply to
    /// the range of their ends' products, sign included: the renderer
    /// multiplies the extent by the signed factor, so -2 mirrors the draw
    /// across the ground 2 covers.
    fn compose_scale(mut self, (lo, hi): (f32, f32)) -> Self {
        let (a, b) = self.scale_range;
        let products = [a * lo, a * hi, b * lo, b * hi];
        self.scale_range = (
            products.into_iter().fold(f32::INFINITY, f32::min),
            products.into_iter().fold(f32::NEG_INFINITY, f32::max),
        );
        self
    }

    /// The most any reachable factor grows the draw by.
    /// A shrink keeps it inside its box, so nothing below 1 counts.
    fn scale_reach(self) -> f32 {
        let (lo, hi) = self.scale_range;
        lo.abs().max(hi.abs()).max(1.0)
    }

    fn widen_by(mut self, margin: f32) -> Self {
        self.margin = self.margin.max(margin);
        self
    }

    /// Apply the accumulated ranges to a leaf box, in canvas-local coordinates.
    fn apply(self, leaf: Rect, canvas_w: f32, canvas_h: f32) -> Rect {
        self.apply_anchored(leaf, canvas_w, canvas_h, ScaleAnchor::Centre)
    }

    /// [`Self::apply`] for a leaf that paints only `painted`, a region inside it.
    ///
    /// The renderer scales the leaf about its centre,
    /// which moves `painted` rather than growing it in place.
    /// Under a factor that cannot turn negative,
    /// each point only slides along its ray from the centre,
    /// so the region at both ends of the range covers every frame between.
    /// One that can turn negative flips it across the centre,
    /// so the region is mirrored there to cover every factor in `-1..=1`,
    /// and `apply` scales that out to the rest.
    fn apply_within(self, leaf: Rect, painted: Rect, canvas_w: f32, canvas_h: f32) -> Rect {
        let (lo, hi) = self.scale_range;
        if (lo - 1.0).abs() + (hi - 1.0).abs() == 0.0 {
            return self.apply(painted, canvas_w, canvas_h);
        }
        let (cx, cy) = (leaf.x + leaf.w / 2.0, leaf.y + leaf.h / 2.0);
        if lo >= 0.0 {
            let about_centre = |factor: f32| {
                Rect::new(
                    cx + (painted.x - cx) * factor,
                    cy + (painted.y - cy) * factor,
                    painted.w * factor,
                    painted.h * factor,
                )
            };
            let mut reached = about_centre(lo);
            reached.union(about_centre(hi));
            // The scale is spent on `reached`; only the stroke it widens is left.
            let placed = Self {
                scale_range: (1.0, 1.0),
                margin: self.margin * self.scale_reach(),
                ..self
            };
            return placed.apply(reached, canvas_w, canvas_h);
        }
        let half_w = (painted.x - cx)
            .abs()
            .max((painted.x + painted.w - cx).abs());
        let half_h = (painted.y - cy)
            .abs()
            .max((painted.y + painted.h - cy).abs());
        let mirrored = Rect::new(cx - half_w, cy - half_h, 2.0 * half_w, 2.0 * half_h);
        self.apply(mirrored, canvas_w, canvas_h)
    }

    /// [`Self::apply`] for a leaf whose scaling grows it about `anchor`.
    fn apply_anchored(self, leaf: Rect, canvas_w: f32, canvas_h: f32, anchor: ScaleAnchor) -> Rect {
        let reach = self.scale_reach();
        let grow_w = leaf.w * (reach - 1.0);
        let grow_h = leaf.h * (reach - 1.0);
        // The margin is an unscaled stroke delta, and the renderer scales
        // the stroke along with the box (`ew = width * scale`),
        // so the reach it stands for grows too.
        let margin = self.margin * reach;
        let scaled = match anchor {
            ScaleAnchor::Centre => Rect::new(
                leaf.x - grow_w / 2.0 - margin,
                leaf.y - grow_h / 2.0 - margin,
                leaf.w + grow_w + 2.0 * margin,
                leaf.h + grow_h + 2.0 * margin,
            ),
            ScaleAnchor::TopLeft => Rect::new(
                leaf.x - margin,
                leaf.y - margin,
                leaf.w + grow_w + 2.0 * margin,
                leaf.h + grow_h + 2.0 * margin,
            ),
        };
        let mut moved = Rect::new(
            scaled.x + self.layout_x + self.translate_x.0,
            scaled.y + self.layout_y + self.translate_y.0,
            scaled.w,
            scaled.h,
        );
        moved.union(Rect::new(
            scaled.x + self.layout_x + self.translate_x.1,
            scaled.y + self.layout_y + self.translate_y.1,
            scaled.w,
            scaled.h,
        ));
        let Some((from, to)) = self.rotation else {
            return moved;
        };
        swept_bounds(moved, (canvas_w / 2.0, canvas_h / 2.0), from, to)
    }
}

/// Region a draw sweeps while rotating about `pivot` over `[from, to]`.
///
/// A rotating convex box is extreme in any direction at one of its corners, so
/// the swept region's bounds are the union of the four corners' circular arcs.
/// A range spanning a full turn falls out of the same arithmetic as the disc
/// through the pivot — no special case.
fn swept_bounds(rect: Rect, pivot: (f32, f32), from: f32, to: f32) -> Rect {
    let (start, end) = (from.min(to), from.max(to));
    let corners = [
        (rect.x, rect.y),
        (rect.x + rect.w, rect.y),
        (rect.x, rect.y + rect.h),
        (rect.x + rect.w, rect.y + rect.h),
    ];
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (px, py) in corners {
        let (dx, dy) = (px - pivot.0, py - pivot.1);
        let radius = dx.hypot(dy);
        let phi = dy.atan2(dx);
        let (cos_lo, cos_hi) = cos_range(phi + start, phi + end);
        let (sin_lo, sin_hi) = sin_range(phi + start, phi + end);
        min_x = min_x.min(pivot.0 + radius * cos_lo);
        max_x = max_x.max(pivot.0 + radius * cos_hi);
        min_y = min_y.min(pivot.1 + radius * sin_lo);
        max_y = max_y.max(pivot.1 + radius * sin_hi);
    }
    Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

/// Whether some `target + 2πk` lies in `[start, start + span]`.
fn spans_angle(start: f32, span: f32, target: f32) -> bool {
    (target - start).rem_euclid(TAU) <= span
}

fn cos_range(start: f32, end: f32) -> (f32, f32) {
    let span = end - start;
    let hi = if spans_angle(start, span, 0.0) {
        1.0
    } else {
        start.cos().max(end.cos())
    };
    let lo = if spans_angle(start, span, PI) {
        -1.0
    } else {
        start.cos().min(end.cos())
    };
    (lo, hi)
}

fn sin_range(start: f32, end: f32) -> (f32, f32) {
    let span = end - start;
    let hi = if spans_angle(start, span, FRAC_PI_2) {
        1.0
    } else {
        start.sin().max(end.sin())
    };
    let lo = if spans_angle(start, span, -FRAC_PI_2) {
        -1.0
    } else {
        start.sin().min(end.sin())
    };
    (lo, hi)
}

/// Box around an arc stroke of `half_width`
/// whose centre line runs over `angles`, ascending,
/// measured clockwise from 12 o'clock as the renderer does.
///
/// Padding the centre line's bounds by the half width on every side
/// also covers a round cap, which is a disc of that radius on the line's end.
fn sector_bounds(centre: (f32, f32), radius: f32, angles: (f32, f32), half_width: f32) -> Rect {
    let (sin_lo, sin_hi) = sin_range(angles.0, angles.1);
    let (cos_lo, cos_hi) = cos_range(angles.0, angles.1);
    let (left, right) = (centre.0 + radius * sin_lo, centre.0 + radius * sin_hi);
    let (top, bottom) = (centre.1 - radius * cos_hi, centre.1 - radius * cos_lo);
    Rect::new(
        left - half_width,
        top - half_width,
        right - left + 2.0 * half_width,
        bottom - top + 2.0 * half_width,
    )
}

/// The angles an arc's stroke can cover, in radians clockwise from 12 o'clock.
#[derive(Clone, Copy)]
enum ArcSpan {
    /// Between `start` and `end`, with `start <= end`.
    Sector { start: f32, end: f32 },
    /// Anywhere on the ring.
    FullRing,
}

impl ArcSpan {
    /// What a stroke from `start` to `end` covers.
    ///
    /// A negative sweep covers the ring too: femtovg 0.20 `Path::arc` adds full
    /// turns until a hole-wound sweep is positive, so it paints the long way round.
    fn between(start: f32, end: f32) -> Self {
        if end < start || end - start >= TAU {
            Self::FullRing
        } else {
            Self::Sector { start, end }
        }
    }
}

/// What a transition on an arc, its direct child, moves the arc through.
///
/// The renderer replaces the arc's interpolated geometry outright
/// rather than composing it like the transforms in [`Transform`].
#[derive(Clone, Copy)]
struct ArcTransition {
    span: ArcSpan,
    /// Every radius the eased transition reaches, ascending.
    radius: (f32, f32),
}

/// Where a [`DrawCommand::Arc`] strokes, with its angles in radians
/// clockwise from 12 o'clock.
#[derive(Clone, Copy)]
struct ArcGeometry {
    centre: (f32, f32),
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    width: f32,
}

impl ArcGeometry {
    fn of(draw: &DrawCommand) -> Option<Self> {
        let DrawCommand::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            ..
        } = *draw
        else {
            return None;
        };
        Some(Self {
            centre: (cx, cy),
            radius,
            start_angle,
            end_angle,
            width,
        })
    }
}

/// Canvas-local box of an arc: the sector it or its transition can sweep,
/// painted within its ring.
///
/// A `Scale` animation scales an arc's radius and stroke about the ring's centre,
/// as it scales an icon about its box's, so the sector goes through the same
/// [`Transform::apply_within`]. A transition moves the radius alone,
/// so the stroke keeps its width at every radius in between.
fn arc_bounds(
    arc: ArcGeometry,
    transition: Option<ArcTransition>,
    transform: &Transform,
    canvas: Rect,
) -> Rect {
    let (span, (least, most)) = transition.map_or_else(
        || {
            (
                ArcSpan::between(arc.start_angle, arc.end_angle),
                (arc.radius, arc.radius),
            )
        },
        |transition| (transition.span, transition.radius),
    );
    let half_width = arc.width / 2.0;
    // femtovg draws a negative radius mirrored through the centre.
    let reach = least.abs().max(most) + half_width;
    let (cx, cy) = arc.centre;
    let ring = Rect::new(cx - reach, cy - reach, 2.0 * reach, 2.0 * reach);
    let painted = match span {
        // Each point of the centre line moves along its ray as the radius changes,
        // so the sectors at both ends of the range cover every radius between.
        ArcSpan::Sector { start, end } if least >= 0.0 => {
            let mut painted = sector_bounds(arc.centre, least, (start, end), half_width);
            painted.union(sector_bounds(arc.centre, most, (start, end), half_width));
            painted
        }
        ArcSpan::Sector { .. } | ArcSpan::FullRing => ring,
    };
    transform.apply_within(ring, painted, canvas.w, canvas.h)
}

/// Slack on every side for femtovg's antialiasing fringe: `expand_fill`
/// displaces a ribbon about half a fringe width past the geometry `bounded`
/// returns. Whether flooring absorbs it depends on where the edge lands, so
/// without this a moving draw sheds its fringe from the damage set on roughly
/// every other frame, and a cached frame never paints it back — a trailing line.
const FRINGE_SLACK: f32 = 1.0;

/// How far from the canvas origin, in logical pixels, f32 still resolves an eighth of a pixel: 2²⁰.
const PRECISE_EXTENT: f32 = 1_048_576.0;

/// Upper bound on femtovg's fringe width in logical pixels.
///
/// It sets `1 / dpi` physical pixels, so `1 / dpi²` logical:
/// within one while the dpi scale is at least 1,
/// which every `FemtoVgRenderer` frame debug-asserts on the way in.
const FEMTOVG_FRINGE: f32 = 1.0;

/// The scale femtovg's `PathCache::calculate_joins` caps a join's extrusion at,
/// so no join, stroked or filled, reaches past `√600` widths.
const FEMTOVG_EXTRUSION_SCALE_CAP: f32 = 600.0;

/// How many fringe widths past its join `PathCache::expand_fill` puts the outer
/// edge of a fill's fringe: a whole fringe beyond the half it insets the fill by,
/// on whichever side the path's winding puts it.
const FEMTOVG_FILL_FRINGE_OFFSET: f32 = 1.5;

/// What one dynamic canvas draw asks a repaint to cover.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DrawDamage {
    /// Screen-space region the repaint must cover.
    Region(Rect),
    /// The draw is scissored away entirely, so it paints nothing.
    Nothing,
    /// The draw's reach cannot be bounded; repaint the whole canvas.
    WholeCanvas,
}

impl DrawDamage {
    /// The bounded region, if there is one.
    #[cfg(test)]
    fn region(self) -> Option<Rect> {
        match self {
            Self::Region(rect) => Some(rect),
            Self::Nothing | Self::WholeCanvas => None,
        }
    }
}

/// Screen-space region a repaint must cover for one dynamic canvas draw.
///
/// `canvas` is the canvas node's screen rect; the returned rect is clipped to
/// it, since the walk scissors every canvas to its own bounds. Edges are rounded
/// outward to whole pixels — the GL scissor truncates, so a fractional edge
/// would drop the pixel row the draw actually touches.
#[must_use]
pub(crate) fn canvas_draw_damage(
    draw: &DrawCommand,
    canvas: Rect,
    transitions: &HashMap<TransitionStateKey, TransitionState>,
    canvas_index: u16,
    svg_content: &dyn Fn(SvgId) -> Option<SvgContent>,
) -> DrawDamage {
    let scope = Scope {
        canvas,
        transitions,
        canvas_index,
        svg_content,
    };
    let Some(local) = bounded(draw, Transform::IDENTITY, &scope) else {
        return DrawDamage::WholeCanvas;
    };
    // Fail open. `f32::min`/`max` ignore NaN, so a NaN sweep leaves `+INF`
    // against `-INF` and inverts the comparison below: the draw would report
    // *nothing* while still painting. The deserializer rejects these, so this
    // is the second line, and over-damaging is the harmless direction.
    // Past `PRECISE_EXTENT` the sums below lose sub-pixel precision and can cancel,
    // collapsing the rect of an icon whose points reach far off its box.
    let precise = |v: f32| v.abs() <= PRECISE_EXTENT;
    if !(precise(local.x) && precise(local.y) && precise(local.w) && precise(local.h)) {
        return DrawDamage::WholeCanvas;
    }
    let x = (canvas.x + local.x - FRINGE_SLACK).floor().max(canvas.x);
    let y = (canvas.y + local.y - FRINGE_SLACK).floor().max(canvas.y);
    let right = (canvas.x + local.x + local.w + FRINGE_SLACK)
        .ceil()
        .min(canvas.x + canvas.w);
    let bottom = (canvas.y + local.y + local.h + FRINGE_SLACK)
        .ceil()
        .min(canvas.y + canvas.h);
    // Clipped away is not unboundable: reporting it as such would damage the
    // whole canvas every frame for a draw that paints nothing.
    if right > x && bottom > y {
        DrawDamage::Region(Rect::new(x, y, right - x, bottom - y))
    } else {
        DrawDamage::Nothing
    }
}

/// What every level of [`bounded`] reads but none changes.
struct Scope<'a> {
    canvas: Rect,
    transitions: &'a HashMap<TransitionStateKey, TransitionState>,
    canvas_index: u16,
    /// [`crate::renderer::RenderAssets::svg_content`].
    svg_content: &'a dyn Fn(SvgId) -> Option<SvgContent>,
}

fn bounded(draw: &DrawCommand, transform: Transform, scope: &Scope<'_>) -> Option<Rect> {
    let canvas = scope.canvas;
    match draw {
        DrawCommand::Rotated { angle, inner } => {
            bounded(inner, transform.add_rotation((*angle, *angle)), scope)
        }

        DrawCommand::Centered { inner } => {
            let (iw, ih) = get_draw_bounds(inner);
            let dx = (canvas.w - iw) / 2.0;
            let dy = (canvas.h - ih) / 2.0;
            bounded(inner, transform.set_layout(dx, dy), scope)
        }

        // A moving orbit angle would trace an arc of `radius` around the canvas
        // centre; a fixed one is just an offset. `Modified` refuses the moving
        // case, so reaching here means the angle is fixed.
        DrawCommand::Orbit {
            radius,
            angle,
            inner,
        } => {
            let (iw, ih) = get_draw_bounds(inner);
            let dx = canvas.w / 2.0 + radius * angle.cos() - iw / 2.0;
            let dy = canvas.h / 2.0 + radius * angle.sin() - ih / 2.0;
            bounded(inner, transform.set_layout(dx, dy), scope)
        }

        DrawCommand::Modified {
            animations,
            transition,
            inner,
            ..
        } => {
            let modification = modified_transform(
                transform,
                animations,
                transition.as_ref(),
                inner,
                scope.transitions,
                scope.canvas_index,
            )?;
            match modification {
                Modification::Composed(transform) => bounded(inner, transform, scope),
                Modification::ArcTransition {
                    transform,
                    arc,
                    transition,
                } => Some(arc_bounds(arc, Some(transition), &transform, canvas)),
            }
        }

        // The shadow renders its inner draw into a canvas-sized layer and
        // composites the whole of it, so its reach is the canvas either way.
        DrawCommand::Shadow { .. } => None,

        DrawCommand::Svg {
            x,
            y,
            w,
            h,
            icon_id,
            ..
        } => Some(svg_bound(
            Rect::new(*x, *y, *w, *h),
            *icon_id,
            transform,
            scope,
        )),
        DrawCommand::Rect { x, y, w, h, .. }
        | DrawCommand::Bitmap { x, y, w, h, .. }
        | DrawCommand::Sphere { x, y, w, h, .. }
        | DrawCommand::Mesh { x, y, w, h, .. }
        | DrawCommand::NinePatch { x, y, w, h, .. } => {
            Some(transform.apply(Rect::new(*x, *y, *w, *h), canvas.w, canvas.h))
        }
        DrawCommand::AutofitText {
            x,
            y,
            box_width,
            box_height,
            ..
        } => Some(transform.apply_anchored(
            Rect::new(*x, *y, *box_width, *box_height),
            canvas.w,
            canvas.h,
            ScaleAnchor::TopLeft,
        )),
        DrawCommand::Qr { x, y, size, .. } => {
            Some(transform.apply(Rect::new(*x, *y, *size, *size), canvas.w, canvas.h))
        }
        DrawCommand::Circle { cx, cy, r, .. } => Some(transform.apply(
            Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r),
            canvas.w,
            canvas.h,
        )),
        DrawCommand::Arc { .. } => {
            ArcGeometry::of(draw).map(|arc| arc_bounds(arc, None, &transform, canvas))
        }
        // Text extents depend on shaping the host has not done at this point,
        // and a stroked path straddles its points by half a width this level
        // cannot read out of the paint.
        DrawCommand::Text { .. } | DrawCommand::CurvedText { .. } | DrawCommand::Path { .. } => {
            None
        }
    }
}

/// Bound an icon drawn into `leaf` by what its paths paint.
///
/// usvg folds a letterboxed viewBox into the paths,
/// so an icon may paint a sliver of its box: a clock hand is 4 units wide in 398.
fn svg_bound(leaf: Rect, icon_id: Option<SvgId>, transform: Transform, scope: &Scope<'_>) -> Rect {
    let (canvas_w, canvas_h) = (scope.canvas.w, scope.canvas.h);
    let Some(content) = icon_id.and_then(scope.svg_content) else {
        return transform.apply(leaf, canvas_w, canvas_h);
    };
    let (sx, sy) = (leaf.w / content.viewbox_w, leaf.h / content.viewbox_h);
    let painted = Rect::new(
        leaf.x + content.bounds.x * sx,
        leaf.y + content.bounds.y * sy,
        content.bounds.w * sx,
        content.bounds.h * sy,
    );
    // Any join may extrude as far as femtovg's cap lets it.
    let cap = FEMTOVG_EXTRUSION_SCALE_CAP.sqrt();
    let stroke_reach = content.stroke.map_or(0.0, |stroke| {
        // femtovg strokes in screen space: one width for both axes,
        // scaled by the mean of its matrix's row norms,
        // floored at the fringe and widened by half of it before the joins extrude.
        // A rotation by θ mixes `sx` and `sy` into rows whose norms average
        // at most `|cos θ| + |sin θ|` times theirs, the factor by which
        // the rotated margin's bounding box already outreaches the margin.
        let half_width = stroke.half_width * (sx + sy) / 2.0;
        (half_width.max(FEMTOVG_FRINGE / 2.0) + FEMTOVG_FRINGE / 2.0) * cap
    });
    let fill_reach = if content.fills {
        FEMTOVG_FILL_FRINGE_OFFSET * FEMTOVG_FRINGE * cap
    } else {
        0.0
    };
    transform
        .widen_by(stroke_reach.max(fill_reach))
        .apply_within(leaf, painted, canvas_w, canvas_h)
}

/// What a `Modified` wrapper does to the draw inside it.
enum Modification {
    /// Moves it by a range the renderer composes, as [`Transform`] accumulates.
    Composed(Transform),
    /// Its transition replaces `arc`, the direct child, with interpolated geometry
    /// on top of `transform`.
    ArcTransition {
        transform: Transform,
        arc: ArcGeometry,
        transition: ArcTransition,
    },
}

/// What a `Modified` wrapper does to its draw, or `None` when one of its
/// payloads moves the draw in a way this module cannot bound.
fn modified_transform(
    transform: Transform,
    animations: &[crate::tree::HostAnimationDef],
    transition: Option<&crate::tree::HostTransitionDef>,
    inner: &DrawCommand,
    transitions: &HashMap<TransitionStateKey, TransitionState>,
    canvas_index: u16,
) -> Option<Modification> {
    let mut transform = transform;
    for anim in animations {
        let (lo, hi) = animated_range(anim);
        transform = match anim.property {
            AnimProperty::Rotate => transform.add_rotation((lo, hi)),
            AnimProperty::TranslateX => transform.add_translate((lo, hi), (0.0, 0.0)),
            AnimProperty::TranslateY => transform.add_translate((0.0, 0.0), (lo, hi)),
            AnimProperty::Scale => transform.compose_scale((lo, hi)),
            // An orbit angle sweeps the inner draw around a radius this level
            // cannot see.
            AnimProperty::OrbitAngle => return None,
            AnimProperty::Alpha | AnimProperty::Color => transform,
        };
    }
    let Some(def) = transition else {
        return Some(Modification::Composed(transform));
    };
    // The renderer applies the interpolation as a delta against the tree's
    // current values: `target + delta * (1 - eased)`.
    let state = transitions.get(&(canvas_index, def.id_hash))?;
    let (from, target) = (state.from, extract_draw_values(inner));
    if (from.angle - target.angle) != 0.0 {
        return None;
    }
    // A mesh or sphere transition interpolates a 3D scale and position whose
    // projection this level cannot predict, so it may well paint outside the
    // box the draw declares.
    if matches!(inner, DrawCommand::Mesh { .. } | DrawCommand::Sphere { .. }) {
        return None;
    }
    let (near, far) = transition_factors(state.easing);
    let delta_range = |delta: f32| {
        let (a, b) = (delta * near, delta * far);
        (a.min(b), a.max(b))
    };
    // An arc's leaf box is read from the tree's stroke width; a transition
    // starting from a thicker one — or overshooting a thinner one — is wider
    // than that box in between.
    let arc_delta = from.arc_width - target.arc_width;
    let (_, widest) = delta_range(arc_delta);
    transform = transform.widen_by(widest.max(0.0) / 2.0);
    transform = transform
        .add_translate(
            delta_range(from.x - target.x),
            delta_range(from.y - target.y),
        )
        // The renderer wraps the rotation delta to the shortest path, so a
        // transition past half a turn travels the way round the raw
        // difference does not: bounding the raw one leaves every pose in
        // between outside the damage rect.
        .add_rotation(delta_range(-crate::components::draw::shortest_angle_delta(
            from.rotation,
            target.rotation,
        )));
    let direct_arc = ArcGeometry::of(inner);
    if target.w > 0.0 && direct_arc.is_none() {
        // Width interpolates the same way, so the reachable scale applies
        // the delta's factor about the target rather than the raw ratio.
        let ratio_delta = from.w / target.w - 1.0;
        transform = transform.compose_scale((1.0 + ratio_delta * near, 1.0 + ratio_delta * far));
    }
    // The renderer interpolates an arc's radius, start and sweep
    // only when the arc is the transition's direct child.
    // While the sweep stays non-negative it paints between the two ends.
    // The start takes the shortest way round, like the rotation above.
    let Some(arc) = direct_arc else {
        return Some(Modification::Composed(transform));
    };
    let (radius_lo, radius_hi) = delta_range(from.w - target.w);
    let (sweep_lo, _) = delta_range(from.arc_sweep - target.arc_sweep);
    let span = if target.arc_sweep + sweep_lo < 0.0 {
        ArcSpan::FullRing
    } else {
        let start_delta = -crate::components::draw::shortest_angle_delta(
            from.arc_start_angle,
            target.arc_start_angle,
        );
        let target_end = target.arc_start_angle + target.arc_sweep;
        let (start_lo, start_hi) = delta_range(start_delta);
        let (end_lo, end_hi) = delta_range(start_delta + from.arc_sweep - target.arc_sweep);
        ArcSpan::between(
            (target.arc_start_angle + start_lo).min(target_end + end_lo),
            (target.arc_start_angle + start_hi).max(target_end + end_hi),
        )
    };
    let radius = (target.w + radius_lo, target.w + radius_hi);
    Some(Modification::ArcTransition {
        transform,
        arc,
        transition: ArcTransition { span, radius },
    })
}

/// The factors the renderer can multiply a transition's delta by.
///
/// It interpolates `target + delta * (1 - eased)`. An easing confined
/// to `0.0..=1.0` reaches no further than the recorded start; the Back
/// and Elastic families leave that interval, putting the value *past
/// the target*, on the opposite side from where it started.
fn transition_factors(easing: Easing) -> (f32, f32) {
    let (min_t, max_t) = crate::animation::easing_extremes(easing);
    (1.0 - max_t, 1.0 - min_t)
}

/// Every value `anim` reaches, in ascending order.
///
/// Wider than its endpoints: the Back and Elastic easings leave
/// `0.0..=1.0` deliberately, so the eased lerp passes the endpoint it is
/// heading for — an elastic 1 → 2 scale reaches 2.37. Bounding by the
/// endpoints alone leaves that overshoot unrepainted.
fn animated_range(anim: &crate::tree::HostAnimationDef) -> (f32, f32) {
    let (min_t, max_t) = crate::animation::easing_extremes(anim.easing);
    let span = anim.to - anim.from;
    let (a, b) = (anim.from + span * min_t, anim.from + span * max_t);
    (a.min(b), a.max(b))
}

#[cfg(test)]
mod tests;
