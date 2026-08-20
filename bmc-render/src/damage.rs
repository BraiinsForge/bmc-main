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
//! Damage tracking repaints only what changed, and its granularity is whatever
//! rectangles the walk reports. A canvas is a single layout leaf, so reporting
//! the node's rect damages the whole canvas for one animated draw — an analog
//! clock whose canvas fills the viewport reported 100% and repainted the
//! surface to move three hands.
//!
//! This module bounds a single draw instead. The bound must hold for **every**
//! frame the rectangle is used in, not just the one it was computed from:
//! damage is consumed a frame late (it has to be known before the static layer
//! is composited, but is discovered during the walk) and spans the export
//! buffer rotation. So the bound is taken over the whole *range* each transform
//! can reach while the guest is idle — an animation's `from..to`, a
//! transition's recorded `from..target` — rather than the value this frame
//! happens to hold. Those ranges only change on a guest frame, and a guest
//! frame repaints in full.
//!
//! Every bound over-approximates: over-covering costs pixels, under-covering
//! leaves trails of stale content, and `BMC_DAMAGE_MAX_PCT` already abandons
//! tracking when the total grows past the point of paying for itself. Anything
//! this module cannot bound at all — a drop shadow (composited as a
//! canvas-sized layer), an orbit whose angle moves, unmeasurable text extents,
//! a transition with no recorded state yet — returns `None`, and the caller
//! falls back to the whole canvas.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::components::draw::{extract_draw_values, get_draw_bounds};
use crate::interaction::Rect;
use crate::tree::DrawCommand;
use crate::{TransitionState, TransitionStateKey};
use bmc_wasm_protocol::AnimProperty;

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
    /// enlarge the draw. Values below 1 shrink the box and so stay inside it,
    /// which is why they contribute nothing.
    scale_max: f32,
    /// `None` until something can rotate this draw. Kept distinct from a zero
    /// range so a draw that never rotates skips the sweep entirely — the polar
    /// round-trip inside it costs a pixel of precision, which is fine as slack
    /// on a rotating draw and pure waste on a still one.
    rotation: Option<(f32, f32)>,
    /// Extra reach on every side, for a stroke that is thicker somewhere in the
    /// transition than in the tree the leaf box was read from.
    margin: f32,
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
    /// way (`acc_scale *= value` in `components/draw.rs`): two 1 → 2 scales on
    /// one draw reach 4x, and a bound that took the larger of them would leave
    /// everything past 2x unrepainted on the way back in. A factor below 1 only
    /// shrinks the box, so it is clamped away rather than shrinking the bound.
    fn compose_scale(mut self, factor: f32) -> Self {
        self.scale_max *= factor.max(1.0);
        self
    }

    fn widen_by(mut self, margin: f32) -> Self {
        self.margin = self.margin.max(margin);
        self
    }

    /// Apply the accumulated ranges to a leaf box, in canvas-local coordinates.
    fn apply(self, leaf: Rect, canvas_w: f32, canvas_h: f32) -> Rect {
        // Scale is centred on the leaf's own box, matching the renderer's
        // `sx = x + (w - w * scale) / 2`.
        let grow_w = leaf.w * (self.scale_max - 1.0) / 2.0;
        let grow_h = leaf.h * (self.scale_max - 1.0) / 2.0;
        let grow_w = grow_w + self.margin;
        let grow_h = grow_h + self.margin;
        let scaled = Rect::new(
            leaf.x - grow_w,
            leaf.y - grow_h,
            leaf.w + 2.0 * grow_w,
            leaf.h + 2.0 * grow_h,
        );
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

/// Screen-space region a repaint must cover for one dynamic canvas draw, or
/// `None` when the draw's reach cannot be bounded and the caller must fall back
/// to the whole canvas.
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
) -> Option<Rect> {
    let local = bounded(draw, Transform::IDENTITY, canvas, transitions, canvas_index)?;
    let x = (canvas.x + local.x).floor().max(canvas.x);
    let y = (canvas.y + local.y).floor().max(canvas.y);
    let right = (canvas.x + local.x + local.w)
        .ceil()
        .min(canvas.x + canvas.w);
    let bottom = (canvas.y + local.y + local.h)
        .ceil()
        .min(canvas.y + canvas.h);
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
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
                transform.add_offset((dx, dx), (dy, dy)),
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
                transform.add_offset((dx, dx), (dy, dy)),
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
        } => Some(transform.apply(
            Rect::new(*x, *y, *box_width, *box_height),
            canvas.w,
            canvas.h,
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
        let (lo, hi) = (anim.from.min(anim.to), anim.from.max(anim.to));
        transform = match anim.property {
            AnimProperty::Rotate => transform.add_rotation((lo, hi)),
            AnimProperty::TranslateX => transform.add_offset((lo, hi), (0.0, 0.0)),
            AnimProperty::TranslateY => transform.add_offset((0.0, 0.0), (lo, hi)),
            AnimProperty::Scale => transform.compose_scale(hi),
            // An orbit angle sweeps the inner draw around a radius this level
            // cannot see.
            AnimProperty::OrbitAngle => return None,
            AnimProperty::Alpha | AnimProperty::Color => transform,
        };
    }
    let Some(def) = transition else {
        return Some(transform);
    };
    // The interpolation runs from the recorded `from` to the tree's current
    // values, and the renderer applies each as a delta against those current
    // values — so the reachable range is between zero and the recorded
    // difference.
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
    // An arc's leaf box is read from the tree's stroke width; a transition
    // starting from a thicker one is wider than that box in between.
    transform = transform.widen_by((from.arc_width - target.arc_width).max(0.0) / 2.0);
    transform = transform
        .add_offset(
            signed_range(from.x - target.x),
            signed_range(from.y - target.y),
        )
        .add_rotation(signed_range(from.rotation - target.rotation));
    if target.w > 0.0 {
        transform = transform.compose_scale(from.w / target.w);
    }
    Some(transform)
}

/// The range between zero and `delta`, in ascending order.
fn signed_range(delta: f32) -> (f32, f32) {
    (delta.min(0.0), delta.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrevDrawValues;
    use crate::tree::{HostAnimationDef, HostTransitionDef};
    use bmc_wasm_protocol::{
        ArcCap, ArcFill, ArcSegments, ColorSpace, Easing, Fill, LoopMode, TextStyle, colors::Color,
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
        canvas_draw_damage(draw, canvas(), &HashMap::new(), 0)
    }

    #[test]
    fn a_still_draw_damages_only_its_own_box() {
        let rect = damage(&square(10.0, 20.0, 30.0)).expect("BUG: a plain rect is bounded");
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (10.0, 20.0, 30.0, 30.0),
            "no transform, no expansion"
        );
    }

    #[test]
    fn a_quarter_turn_sweeps_the_corner_arc_not_the_whole_disc() {
        // A 100 px square in the top-left corner of a square canvas, rotating a
        // quarter turn about the canvas centre. Its far corner is at the pivot's
        // distance and sweeps to the canvas's top-right; the disc would cover
        // every side, a quarter turn only two.
        let draw = DrawCommand::Rotated {
            angle: FRAC_PI_2,
            inner: Box::new(square(0.0, 0.0, 100.0)),
        };
        let square_canvas = Rect::new(0.0, 0.0, 400.0, 400.0);
        let rect = canvas_draw_damage(&draw, square_canvas, &HashMap::new(), 0)
            .expect("BUG: a rotation of a rect is bounded");
        // Corners sit at radius 200·√2 ≈ 283 and 100·√2 ≈ 141 from the centre;
        // a quarter turn about (200, 200) maps the box onto the top-right.
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
            .expect("BUG: a rotate animation is bounded");
        // Radius to the far corner is 200·√2 ≈ 283, so the disc overruns the
        // canvas on every side and clips to all of it.
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (0.0, 0.0, 400.0, 400.0),
            "a full turn reaches every corner"
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
            (150.0, 180.0),
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
            (90.0, 90.0, 40.0, 40.0),
            "2x about the centre of a 20x20 at (100, 100)"
        );
    }

    /// `Draw::animate` takes several, and the renderer multiplies them
    /// together. Bounding by the larger alone leaves the pixels between 2x and
    /// 4x unrepainted while the draw contracts, so the previous frame stays on
    /// screen out there.
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
            (70.0, 70.0, 80.0, 80.0),
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
            (90.0, 90.0, 40.0, 40.0),
            "the 2x reach survives the shrink beside it"
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
            .expect("BUG: a transition with recorded state is bounded");
        assert_eq!(
            (rect.x, rect.w),
            (160.0, 70.0),
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
            .expect("BUG: an arc transition is bounded");
        // Ring reaches radius + width/2 = 52 at the tree's 4 px stroke, and 62
        // at the 24 px stroke it is easing down from.
        assert_eq!(
            (rect.x, rect.w),
            (138.0, 124.0),
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
            canvas_draw_damage(&draw, canvas(), &transitions, 0).is_none(),
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
            (0.0, 0.0, 50.0, 50.0),
            "the walk scissors each canvas to its own bounds"
        );
    }

    #[test]
    fn a_draw_entirely_outside_the_canvas_damages_nothing() {
        assert!(damage(&square(-100.0, -100.0, 10.0)).is_none());
    }

    #[test]
    fn a_clock_hand_costs_a_quarter_of_a_deck_screen() {
        // The shape the module exists for: the analog clock's second hand — a
        // 338 px box whose pivot is its own centre, placed at the centre of a
        // 1280×480 canvas, easing 6° per second between two recorded angles.
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
        let rect =
            canvas_draw_damage(&draw, canvas(), &transitions, 0).expect("BUG: a hand is bounded");
        let share = rect.w * rect.h / (1280.0 * 480.0);
        assert!(
            (0.20..0.30).contains(&share),
            "a hand should damage roughly a quarter of the screen, got {share} from {rect:?}"
        );
    }
}
