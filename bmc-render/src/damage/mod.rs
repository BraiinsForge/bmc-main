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
use crate::tree::DrawCommand;
use crate::{TransitionState, TransitionStateKey};
use bmc_wasm_protocol::{AnimProperty, Easing};

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
    /// Largest scale factor reachable, as the product of every factor that can
    /// enlarge the draw. Magnitudes below 1 shrink the box and so stay inside
    /// it, which is why they contribute nothing.
    scale_max: f32,
    /// `None` until something can rotate this draw, kept distinct from a zero
    /// range so a still draw skips the sweep: its polar round-trip costs a pixel
    /// of precision, which is slack worth paying only when something moves.
    rotation: Option<(f32, f32)>,
    /// Extra reach on every side, for a stroke that is thicker somewhere in the
    /// transition than in the tree the leaf box was read from.
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
        scale_max: 1.0,
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

    /// Compose a reachable scale factor onto the accumulated one.
    ///
    /// Multiplicative, because the renderer composes simultaneous scales that
    /// way: two 1 → 2 scales on one draw reach 4x, and taking the larger would
    /// leave everything past 2x unrepainted. A magnitude below 1 only shrinks
    /// the box, so it is clamped away rather than narrowing the bound.
    ///
    /// By magnitude, because the renderer multiplies the extent by the signed
    /// factor: -2 mirrors the draw across the ground 2 covers.
    fn compose_scale(mut self, factor: f32) -> Self {
        self.scale_max *= factor.abs().max(1.0);
        self
    }

    fn widen_by(mut self, margin: f32) -> Self {
        self.margin = self.margin.max(margin);
        self
    }

    /// Apply the accumulated ranges to a leaf box, in canvas-local coordinates.
    fn apply(self, leaf: Rect, canvas_w: f32, canvas_h: f32) -> Rect {
        self.apply_anchored(leaf, canvas_w, canvas_h, ScaleAnchor::Centre)
    }

    /// [`Self::apply`] for a leaf whose scaling grows it about `anchor`.
    fn apply_anchored(self, leaf: Rect, canvas_w: f32, canvas_h: f32, anchor: ScaleAnchor) -> Rect {
        let grow_w = leaf.w * (self.scale_max - 1.0);
        let grow_h = leaf.h * (self.scale_max - 1.0);
        // The margin is an unscaled stroke delta, and the renderer scales
        // the stroke along with the box (`ew = width * scale`),
        // so the reach it stands for grows too.
        let margin = self.margin * self.scale_max;
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

/// Slack on every side for femtovg's antialiasing fringe: `expand_fill`
/// displaces a ribbon about half a fringe width past the geometry `bounded`
/// returns. Whether flooring absorbs it depends on where the edge lands, so
/// without this a moving draw sheds its fringe from the damage set on roughly
/// every other frame, and a cached frame never paints it back — a trailing line.
const FRINGE_SLACK: f32 = 1.0;

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
) -> DrawDamage {
    let Some(local) = bounded(draw, Transform::IDENTITY, canvas, transitions, canvas_index) else {
        return DrawDamage::WholeCanvas;
    };
    // Fail open. `f32::min`/`max` ignore NaN, so a NaN sweep leaves `+INF`
    // against `-INF` and inverts the comparison below: the draw would report
    // *nothing* while still painting. The deserializer rejects these, so this
    // is the second line, and over-damaging is the harmless direction.
    if !local.x.is_finite() || !local.y.is_finite() || !local.w.is_finite() || !local.h.is_finite()
    {
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

fn bounded(
    draw: &DrawCommand,
    transform: Transform,
    canvas: Rect,
    transitions: &HashMap<TransitionStateKey, TransitionState>,
    canvas_index: u16,
) -> Option<Rect> {
    match draw {
        DrawCommand::Rotated { angle, inner } => bounded(
            inner,
            transform.add_rotation((*angle, *angle)),
            canvas,
            transitions,
            canvas_index,
        ),

        DrawCommand::Centered { inner } => {
            let (iw, ih) = get_draw_bounds(inner);
            let dx = (canvas.w - iw) / 2.0;
            let dy = (canvas.h - ih) / 2.0;
            bounded(
                inner,
                transform.set_layout(dx, dy),
                canvas,
                transitions,
                canvas_index,
            )
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
            bounded(
                inner,
                transform.set_layout(dx, dy),
                canvas,
                transitions,
                canvas_index,
            )
        }

        DrawCommand::Modified {
            animations,
            transition,
            inner,
            ..
        } => {
            let transform = modified_transform(
                transform,
                animations,
                transition.as_ref(),
                inner,
                transitions,
                canvas_index,
            )?;
            bounded(inner, transform, canvas, transitions, canvas_index)
        }

        // The shadow renders its inner draw into a canvas-sized layer and
        // composites the whole of it, so its reach is the canvas either way.
        DrawCommand::Shadow { .. } => None,

        DrawCommand::Rect { x, y, w, h, .. }
        | DrawCommand::Svg { x, y, w, h, .. }
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
        // Both ends of an arc transition stay inside the full ring: an
        // interpolated sweep only shortens it, and a moving start angle rotates
        // within it.
        DrawCommand::Arc {
            cx,
            cy,
            radius,
            width,
            ..
        } => {
            let reach = radius + width / 2.0;
            Some(transform.apply(
                Rect::new(cx - reach, cy - reach, 2.0 * reach, 2.0 * reach),
                canvas.w,
                canvas.h,
            ))
        }
        // Text extents depend on shaping the host has not done at this point,
        // and a stroked path straddles its points by half a width this level
        // cannot read out of the paint.
        DrawCommand::Text { .. } | DrawCommand::CurvedText { .. } | DrawCommand::Path { .. } => {
            None
        }
    }
}

/// The transform range a `Modified` wrapper adds, or `None` when one of its
/// payloads moves the draw in a way this module cannot bound.
fn modified_transform(
    transform: Transform,
    animations: &[crate::tree::HostAnimationDef],
    transition: Option<&crate::tree::HostTransitionDef>,
    inner: &DrawCommand,
    transitions: &HashMap<TransitionStateKey, TransitionState>,
    canvas_index: u16,
) -> Option<Transform> {
    let mut transform = transform;
    for anim in animations {
        let (lo, hi) = animated_range(anim);
        transform = match anim.property {
            AnimProperty::Rotate => transform.add_rotation((lo, hi)),
            AnimProperty::TranslateX => transform.add_translate((lo, hi), (0.0, 0.0)),
            AnimProperty::TranslateY => transform.add_translate((0.0, 0.0), (lo, hi)),
            // The reachable ends are ordered, not ranked by reach.
            // A -2 → 1 scale mirrors past twice the box on its way,
            // so the bound follows the widest magnitude, not the upper end.
            AnimProperty::Scale => transform.compose_scale(lo.abs().max(hi.abs())),
            // An orbit angle sweeps the inner draw around a radius this level
            // cannot see.
            AnimProperty::OrbitAngle => return None,
            AnimProperty::Alpha | AnimProperty::Color => transform,
        };
    }
    let Some(def) = transition else {
        return Some(transform);
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
    if target.w > 0.0 {
        // Width interpolates the same way, so the reachable scale applies
        // the delta's factor about the target rather than the raw ratio.
        let ratio_delta = from.w / target.w - 1.0;
        let (lo, hi) = (1.0 + ratio_delta * near, 1.0 + ratio_delta * far);
        transform = transform.compose_scale(lo.abs().max(hi.abs()));
    }
    Some(transform)
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
