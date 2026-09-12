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
/// Mirrors what `render_draw_inner` accumulates on the way down — an offset, a
/// uniform scale about the leaf's own box, and a rotation about the canvas
/// centre — except that each is a range rather than this frame's value.
#[derive(Clone, Copy)]
struct Transform {
    offset_x: (f32, f32),
    offset_y: (f32, f32),
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
        offset_x: (0.0, 0.0),
        offset_y: (0.0, 0.0),
        scale_max: 1.0,
        rotation: None,
        margin: 0.0,
    };

    /// Compose an additive range onto an existing one — the reachable set of a
    /// sum of independent ranges is the sum of their bounds.
    fn add_offset(mut self, dx: (f32, f32), dy: (f32, f32)) -> Self {
        self.offset_x = (self.offset_x.0 + dx.0, self.offset_x.1 + dx.1);
        self.offset_y = (self.offset_y.0 + dy.0, self.offset_y.1 + dy.1);
        self
    }

    /// Replace the accumulated offset, for a draw that positions its inner from
    /// the canvas rather than from where its parent put it.
    ///
    /// `Centered` and `Orbit` compute an absolute offset in the renderer
    /// (`components/draw.rs`, `new_offset_x`) and pass that down, dropping
    /// whatever they were handed. Adding here instead predicts a rect the paint
    /// never touches: the blit restores background somewhere else while the
    /// dynamic pass compounds alpha over the real draw.
    fn set_offset(mut self, dx: (f32, f32), dy: (f32, f32)) -> Self {
        self.offset_x = dx;
        self.offset_y = dy;
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
            scaled.x + self.offset_x.0,
            scaled.y + self.offset_y.0,
            scaled.w,
            scaled.h,
        );
        moved.union(Rect::new(
            scaled.x + self.offset_x.1,
            scaled.y + self.offset_y.1,
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
                transform.set_offset((dx, dx), (dy, dy)),
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
                transform.set_offset((dx, dx), (dy, dy)),
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
            AnimProperty::TranslateX => transform.add_offset((lo, hi), (0.0, 0.0)),
            AnimProperty::TranslateY => transform.add_offset((0.0, 0.0), (lo, hi)),
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
        .add_offset(
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
mod tests {
    use super::*;
    use crate::PrevDrawValues;
    use crate::tree::{HostAnimationDef, HostTransitionDef};
    use bmc_wasm_protocol::{
        ArcCap, ArcFill, ArcSegments, AutoFit, ColorSpace, Easing, Fill, LoopMode, TextStyle,
        colors::Color,
    };

    type Transitions = HashMap<TransitionStateKey, TransitionState>;

    fn canvas() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 480.0)
    }

    fn square(x: f32, y: f32, side: f32) -> DrawCommand {
        DrawCommand::Rect {
            x,
            y,
            w: side,
            h: side,
            fill: Fill::Solid(Color::from_rgb(1, 2, 3)),
        }
    }

    fn damage(draw: &DrawCommand) -> Option<Rect> {
        canvas_draw_damage(draw, canvas(), &HashMap::new(), 0).region()
    }

    #[test]
    fn a_still_draw_damages_its_box_plus_the_aa_fringe() {
        let rect = damage(&square(10.0, 20.0, 30.0)).expect("BUG: a plain rect is bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (9.0, 19.0, 32.0, 32.0),
            "no transform, no expansion"
        );
    }

    #[test]
    fn a_quarter_turn_sweeps_the_corner_arc_not_the_whole_disc() {
        // A corner square rotating a quarter turn about the canvas centre: the
        // disc would cover every side, a quarter turn only two.
        let draw = DrawCommand::Rotated {
            angle: FRAC_PI_2,
            inner: Box::new(square(0.0, 0.0, 100.0)),
        };
        let square_canvas = Rect::new(0.0, 0.0, 400.0, 400.0);
        let rect = canvas_draw_damage(&draw, square_canvas, &HashMap::new(), 0)
            .region()
            .expect("BUG: a rotation of a rect is bounded");
        assert!(rect.x >= 190.0, "left edge stays right of centre: {rect:?}");
        assert!(rect.y <= 10.0, "reaches the top edge: {rect:?}");
        assert!(
            rect.w * rect.h < square_canvas.w * square_canvas.h * 0.4,
            "a quarter turn must cost far less than the disc: {rect:?}"
        );
    }

    #[test]
    fn a_full_turn_covers_the_disc_through_the_pivot() {
        let mut draw = square(0.0, 0.0, 100.0);
        draw = DrawCommand::Modified {
            animations: vec![HostAnimationDef {
                property: AnimProperty::Rotate,
                from: 0.0,
                to: TAU,
                duration_ms: 1_000,
                delay_ms: 0,
                easing: Easing::Linear,
                loop_mode: LoopMode::Forever,
            }],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(draw),
        };
        let square_canvas = Rect::new(0.0, 0.0, 400.0, 400.0);
        let rect = canvas_draw_damage(&draw, square_canvas, &HashMap::new(), 0)
            .region()
            .expect("BUG: a rotate animation is bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (0.0, 0.0, 400.0, 400.0),
            "a full turn reaches every corner"
        );
    }

    /// `Centered` drops the offset it was handed, so an outer translate never
    /// reaches the paint — see [`Transform::set_offset`].
    #[test]
    fn an_outer_translate_does_not_move_a_centered_draw() {
        let centred = DrawCommand::Centered {
            inner: Box::new(square(0.0, 0.0, 30.0)),
        };
        let still = damage(&centred).expect("BUG: a centred square is bounded");

        let animated = DrawCommand::Modified {
            animations: vec![HostAnimationDef {
                property: AnimProperty::TranslateY,
                from: 20.0,
                to: 40.0,
                duration_ms: 1_000,
                delay_ms: 0,
                easing: Easing::Linear,
                loop_mode: LoopMode::Forever,
            }],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(centred),
        };
        let moved = damage(&animated).expect("BUG: the animated wrapper is bounded");

        assert_eq!(
            (moved.x, moved.y, moved.w, moved.h),
            (still.x, still.y, still.w, still.h),
            "the renderer discards the outer offset, so the bound must too"
        );
    }

    #[test]
    fn a_translate_animation_covers_both_ends() {
        let draw = DrawCommand::Modified {
            animations: vec![HostAnimationDef {
                property: AnimProperty::TranslateX,
                from: -50.0,
                to: 100.0,
                duration_ms: 1_000,
                delay_ms: 0,
                easing: Easing::Linear,
                loop_mode: LoopMode::Forever,
            }],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(square(200.0, 20.0, 30.0)),
        };
        let rect = damage(&draw).expect("BUG: a translate animation is bounded");
        assert_eq!(
            (rect.x, rect.w),
            (149.0, 182.0),
            "from the leftmost reach to the rightmost"
        );
    }

    fn scaling(from: f32, to: f32) -> HostAnimationDef {
        HostAnimationDef {
            property: AnimProperty::Scale,
            from,
            to,
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }
    }

    #[test]
    fn a_scale_animation_covers_its_largest_reach() {
        let draw = DrawCommand::Modified {
            animations: vec![scaling(1.0, 2.0)],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(square(100.0, 100.0, 20.0)),
        };
        let rect = damage(&draw).expect("BUG: a scale animation is bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (89.0, 89.0, 42.0, 42.0),
            "2x about the centre of a 20x20 at (100, 100)"
        );
    }

    /// Bounding by the larger alone strands the pixels between 2x and 4x, where
    /// the previous frame then stays on screen.
    #[test]
    fn two_scale_animations_compose_rather_than_taking_the_larger() {
        let draw = DrawCommand::Modified {
            animations: vec![scaling(1.0, 2.0), scaling(1.0, 2.0)],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(square(100.0, 100.0, 20.0)),
        };
        let rect = damage(&draw).expect("BUG: scale animations are bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (69.0, 69.0, 82.0, 82.0),
            "4x about the centre, not the 2x either one reaches alone"
        );
    }

    /// A factor under 1 keeps the draw inside its own box, so it must not pull
    /// the bound in below what the enlarging factor beside it reaches.
    #[test]
    fn a_shrinking_scale_does_not_narrow_the_bound() {
        let draw = DrawCommand::Modified {
            animations: vec![scaling(1.0, 2.0), scaling(1.0, 0.5)],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(square(100.0, 100.0, 20.0)),
        };
        let rect = damage(&draw).expect("BUG: scale animations are bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (89.0, 89.0, 42.0, 42.0),
            "the 2x reach survives the shrink beside it"
        );
    }

    #[test]
    fn a_mirroring_scale_covers_its_reflected_extent() {
        let draw = DrawCommand::Modified {
            animations: vec![scaling(-2.0, 1.0)],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(square(100.0, 100.0, 20.0)),
        };
        let rect = damage(&draw).expect("BUG: scale animations are bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (89.0, 89.0, 42.0, 42.0),
            "-2x mirrors across the same extent 2x reaches"
        );
    }

    /// An elastic scale passes its upper endpoint by 37%, and the eased value
    /// is what the renderer multiplies the box by. Bounding at the endpoint
    /// leaves the overshoot ring unrepainted on damage-tracked frames.
    #[test]
    fn an_overshooting_easing_widens_the_bound_past_its_endpoints() {
        let mut anim = scaling(1.0, 2.0);
        anim.easing = Easing::EaseOutElastic;
        let draw = DrawCommand::Modified {
            animations: vec![anim],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(square(100.0, 100.0, 20.0)),
        };
        let rect = damage(&draw).expect("BUG: scale animations are bounded");

        // The eased value is the *scale*, not the box:
        // 1 → 2 at 1.374 of the way reaches 2.374x.
        // Bounding at the 2x endpoint gives 40px, which this must reject.
        let (_, eased) = crate::animation::easing_extremes(Easing::EaseOutElastic);
        let reached = 20.0 * (1.0 + eased);
        assert!(
            rect.w >= reached && rect.h >= reached,
            "a {}x scale on a 20px box needs {reached}px of damage, got {rect:?}",
            1.0 + eased
        );
        assert!(
            rect.x <= 110.0 - reached / 2.0 && rect.y <= 110.0 - reached / 2.0,
            "the widened box must stay centred on the draw, got {rect:?}"
        );
    }

    /// The renderer keeps the easing selected at the last retarget, so damage
    /// must follow stored state even if the tree's current definition changes.
    #[test]
    fn damage_uses_the_active_transition_easing_after_the_definition_changes() {
        let inner = square(200.0, 20.0, 30.0);
        let mut recorded = extract_draw_values(&inner);
        recorded.x -= 40.0;
        let mut transitions = HashMap::new();
        transitions.insert(
            (0_u16, 7_u32),
            TransitionState {
                from: recorded,
                target: extract_draw_values(&inner),
                duration_ms: 500,
                easing: Easing::EaseOutBack,
                elapsed_ms: 0,
                last_seen_frame: 0,
            },
        );
        let draw = DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(HostTransitionDef {
                id_hash: 7,
                duration_ms: 500,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(inner),
        };
        let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
            .region()
            .expect("BUG: a transition with recorded state is bounded");

        // Delta is -40 px, so the overshoot lands 40 × 0.101 past x=200.
        let (_, eased) = crate::animation::easing_extremes(Easing::EaseOutBack);
        let overshoot = 40.0 * (eased - 1.0);
        assert!(
            rect.x + rect.w >= 230.0 + overshoot,
            "the bound must reach {overshoot}px past the target's right edge, got {rect:?}"
        );
        assert!(
            rect.x <= 160.0,
            "and still cover where it came from, got {rect:?}"
        );
    }

    /// `render_draw_inner`'s `AutofitText` arm scales its box off `(x, y)`
    /// rather than about the centre every other leaf uses, so a centred bound
    /// stops half the growth short and strands the right and bottom edges.
    #[test]
    fn a_scaled_autofit_box_is_bounded_from_its_own_corner() {
        let inner = DrawCommand::AutofitText {
            x: 100.0,
            y: 100.0,
            box_width: 100.0,
            box_height: 20.0,
            mode: AutoFit::Shrink,
            min_size: 8,
            max_size: 40,
            text: "wide".to_owned(),
            style: TextStyle::default(),
        };
        let draw = DrawCommand::Modified {
            animations: vec![scaling(1.0, 2.0)],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(inner),
        };
        let rect = damage(&draw).expect("BUG: an autofit box is bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (99.0, 99.0, 202.0, 42.0),
            "2x about the corner of a 100x20 box at (100, 100)"
        );
    }

    /// The renderer scales the transition-interpolated stroke too — `ew =
    /// width * scale` — so the margin standing for that stroke has to scale
    /// with it or the thick end of the ring falls outside the repaint.
    #[test]
    fn a_scaled_arc_transition_scales_its_stroke_margin_too() {
        let inner = DrawCommand::Arc {
            cx: 200.0,
            cy: 200.0,
            radius: 50.0,
            start_angle: 0.0,
            end_angle: TAU,
            width: 10.0,
            fill: ArcFill::Solid(Color::from_rgb(255, 255, 255)),
            segments: ArcSegments::Continuous,
            cap: ArcCap::Butt,
        };
        let mut recorded = extract_draw_values(&inner);
        recorded.arc_width = 40.0;
        let mut transitions = HashMap::new();
        transitions.insert(
            (0_u16, 9_u32),
            TransitionState {
                from: recorded,
                target: extract_draw_values(&inner),
                duration_ms: 500,
                easing: Easing::Linear,
                elapsed_ms: 0,
                last_seen_frame: 0,
            },
        );
        let draw = DrawCommand::Modified {
            animations: vec![scaling(1.0, 2.0)],
            transition: Some(HostTransitionDef {
                id_hash: 9,
                duration_ms: 500,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(inner),
        };
        let square_canvas = Rect::new(0.0, 0.0, 640.0, 640.0);
        let rect = canvas_draw_damage(&draw, square_canvas, &transitions, 0)
            .region()
            .expect("BUG: an arc transition is bounded");

        // Half-extent: (radius + recorded stroke / 2) x 2 = (50 + 20) x 2.
        assert!(
            rect.x <= 60.0 && rect.x + rect.w >= 340.0,
            "the 2x ring at its thickest reaches 140px from the centre, got {rect:?}"
        );
    }

    #[test]
    fn a_transition_covers_the_span_between_its_recorded_ends() {
        let inner = square(200.0, 20.0, 30.0);
        let mut recorded = extract_draw_values(&inner);
        recorded.x -= 40.0;
        let mut transitions = HashMap::new();
        transitions.insert(
            (0_u16, 7_u32),
            TransitionState {
                from: recorded,
                target: extract_draw_values(&inner),
                duration_ms: 500,
                easing: Easing::Linear,
                elapsed_ms: 0,
                last_seen_frame: 0,
            },
        );
        let draw = DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(HostTransitionDef {
                id_hash: 7,
                duration_ms: 500,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(inner),
        };
        let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
            .region()
            .expect("BUG: a transition with recorded state is bounded");
        assert_eq!(
            (rect.x, rect.w),
            (159.0, 72.0),
            "covers where it came from as well as where it is going"
        );
    }

    /// Wrap `inner` in a transition whose recorded start is `from`.
    fn transitioning(inner: DrawCommand, from: PrevDrawValues) -> (DrawCommand, Transitions) {
        let mut transitions = HashMap::new();
        transitions.insert(
            (0_u16, 7_u32),
            TransitionState {
                from,
                target: extract_draw_values(&inner),
                duration_ms: 500,
                easing: Easing::Linear,
                elapsed_ms: 0,
                last_seen_frame: 0,
            },
        );
        let draw = DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(HostTransitionDef {
                id_hash: 7,
                duration_ms: 500,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(inner),
        };
        (draw, transitions)
    }

    #[test]
    fn a_thinning_arc_stroke_keeps_the_reach_of_its_thicker_end() {
        let inner = DrawCommand::Arc {
            cx: 200.0,
            cy: 200.0,
            radius: 50.0,
            start_angle: 0.0,
            end_angle: 1.0,
            width: 4.0,
            fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
            segments: ArcSegments::Continuous,
            cap: ArcCap::Round,
        };
        let mut from = extract_draw_values(&inner);
        from.arc_width = 24.0;
        let (draw, transitions) = transitioning(inner, from);
        let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
            .region()
            .expect("BUG: an arc transition is bounded");
        assert_eq!(
            (rect.x, rect.w),
            (137.0, 126.0),
            "the widest stroke in the span sets the reach"
        );
    }

    #[test]
    fn a_sphere_transition_falls_back_to_the_whole_canvas() {
        let inner = DrawCommand::Sphere {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 100.0,
            bitmap_id: None,
            atmosphere: false,
            center_lat: 0.0,
            center_lon: 0.0,
            zoom: 3.0,
            light_lat: 0.0,
            light_lon: 0.0,
        };
        let from = extract_draw_values(&inner);
        let (draw, transitions) = transitioning(inner, from);
        assert!(
            matches!(
                canvas_draw_damage(&draw, canvas(), &transitions, 0),
                DrawDamage::WholeCanvas
            ),
            "a 3D scale and position project outside the declared box"
        );
    }

    #[test]
    fn an_unrecorded_transition_falls_back_to_the_whole_canvas() {
        let draw = DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(HostTransitionDef {
                id_hash: 7,
                duration_ms: 500,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(square(200.0, 20.0, 30.0)),
        };
        assert!(
            damage(&draw).is_none(),
            "with no recorded ends the span is unknown"
        );
    }

    #[test]
    fn a_shadow_and_unmeasured_text_fall_back_to_the_whole_canvas() {
        assert!(
            damage(&DrawCommand::Shadow {
                dx: 0.0,
                dy: 0.0,
                blur: 6.0,
                color: Color::from_rgb(0, 0, 0),
                inner: Box::new(square(10.0, 10.0, 20.0)),
            })
            .is_none(),
            "a shadow composites a canvas-sized layer"
        );
        assert!(
            damage(&DrawCommand::Text {
                x: 0.0,
                y: 0.0,
                text: "x".to_owned(),
                style: TextStyle::default(),
            })
            .is_none(),
            "text extents need shaping"
        );
    }

    #[test]
    fn damage_is_clipped_to_the_canvas() {
        let rect = damage(&square(-100.0, -100.0, 150.0)).expect("BUG: a plain rect is bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (0.0, 0.0, 51.0, 51.0),
            "the walk scissors each canvas to its own bounds"
        );
    }

    #[test]
    fn a_draw_entirely_outside_the_canvas_damages_nothing() {
        assert!(matches!(
            canvas_draw_damage(&square(-100.0, -100.0, 10.0), canvas(), &HashMap::new(), 0),
            DrawDamage::Nothing
        ));
    }

    #[test]
    fn a_clock_hand_costs_a_quarter_of_a_deck_screen() {
        // The shape the module exists for: a clock's second hand, easing 6° a
        // second about its own centre.
        let side = 338.0;
        let pivot_offset = side / 2.0;
        let inner = square(
            1280.0 / 2.0 - pivot_offset,
            480.0 / 2.0 - pivot_offset,
            side,
        );
        let rotated = DrawCommand::Rotated {
            angle: 0.0,
            inner: Box::new(inner),
        };
        let mut recorded = extract_draw_values(&rotated);
        recorded.rotation = -6.0_f32.to_radians();
        let mut transitions = HashMap::new();
        transitions.insert(
            (0_u16, 1_u32),
            TransitionState {
                from: recorded,
                target: extract_draw_values(&rotated),
                duration_ms: 200,
                easing: Easing::Linear,
                elapsed_ms: 0,
                last_seen_frame: 0,
            },
        );
        let draw = DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(HostTransitionDef {
                id_hash: 1,
                duration_ms: 200,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(rotated),
        };
        let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
            .region()
            .expect("BUG: a hand is bounded");
        let share = rect.w * rect.h / (1280.0 * 480.0);
        assert!(
            (0.20..0.30).contains(&share),
            "a hand should damage roughly a quarter of the screen, got {share} from {rect:?}"
        );
    }
}
