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

//! Pure geometry for the draw calls: scissor boxes, arc spans, gradient endpoints and paths.

use crate::gpu::text::to_femtovg_color;
use crate::interaction::Rect;
use bmc_ui::colors::Color;
use bmc_ui::{ArcFill, ArcSegments, Fill};
use femtovg::{Paint, Path};

/// Logical size scaled to device pixels, saturating rather than wrapping.
/// The `glScissor` box covering `rect`, in physical pixels.
///
/// Taffy hands out fractional bounds, so both edges round outwards: an
/// inscribed box leaves the fractional edge column unwritten, and on a
/// `FrameClear::Keep` frame that column keeps the previous frame's pixels — a
/// one-pixel trail behind anything moving right or down. Covering a pixel too
/// many only costs the pixel.
///
/// GL's scissor origin is bottom-left, the tree's is top-left.
#[expect(
    clippy::cast_possible_truncation,
    reason = "physical pixel coordinates, bounded by the render target and clamped to it"
)]
pub(super) fn scissor_box(
    rect: Rect,
    dpi: f32,
    width_px: i32,
    height_px: i32,
) -> (i32, i32, i32, i32) {
    let left = ((rect.x * dpi).floor() as i32).clamp(0, width_px);
    let right = (((rect.x + rect.w) * dpi).ceil() as i32).clamp(left, width_px);
    let top = ((rect.y * dpi).floor() as i32).clamp(0, height_px);
    let bottom = (((rect.y + rect.h) * dpi).ceil() as i32).clamp(top, height_px);
    (left, height_px - bottom, right - left, bottom - top)
}

/// Build a femtovg [`Paint`] for `fill`. `lin_bbox` is the bounding box used
/// for linear endpoint projection; `radial` is `(cx, cy, radius)` for the
/// radial gradient (a circle passes its own radius, not the bbox diagonal).
pub(super) fn paint_for_fill(
    fill: &Fill,
    lin_bbox: (f32, f32, f32, f32),
    radial: (f32, f32, f32),
) -> Paint {
    match fill {
        Fill::Solid(c) => Paint::color(to_femtovg_color(c.to_u32())),
        Fill::Linear { angle, start, end } => {
            let (bx, by, bw, bh) = lin_bbox;
            let ((sx, sy), (ex, ey)) = linear_endpoints(bx, by, bw, bh, *angle);
            Paint::linear_gradient(
                sx,
                sy,
                ex,
                ey,
                to_femtovg_color(start.to_u32()),
                to_femtovg_color(end.to_u32()),
            )
        }
        Fill::Radial { inner, outer } => {
            let (cx, cy, radius) = radial;
            Paint::radial_gradient(
                cx,
                cy,
                0.0,
                radius,
                to_femtovg_color(inner.to_u32()),
                to_femtovg_color(outer.to_u32()),
            )
        }
    }
}

pub(super) const ARC_CHUNK_MAX: f32 = 0.4;

pub(super) const ARC_SEAM_EPS: f32 = 0.002;

// Explicit segments hold absolute angular positions; clipping them to the draw
// sweep lets the sweep reveal or hide whole/partial segments without moving
// them, so a sweep transition animates the arc's length in place. The gradient
// still maps over the draw sweep, so the colour at a revealed angle converges to
// its resting value as the sweep reaches the requested end.
pub(super) fn arc_spans(segments: &ArcSegments, start: f32, end: f32) -> Vec<(f32, f32)> {
    match segments {
        ArcSegments::Continuous => vec![(start, end)],
        ArcSegments::Explicit(spans) => {
            let (lo, hi) = (start.min(end), start.max(end));
            spans
                .iter()
                .filter_map(|&(s, e)| {
                    let clipped = (s.max(lo), e.min(hi));
                    (clipped.0 < clipped.1).then_some(clipped)
                })
                .collect()
        }
    }
}

pub(super) fn chunk_span(a0: f32, a1: f32, max: f32) -> Vec<(f32, f32)> {
    let span = a1 - a0;
    let max = max.abs();
    if span.abs() <= max || max <= f32::EPSILON {
        return vec![(a0, a1)];
    }
    let direction = span.signum();
    let step = max * direction;
    let mut chunks = Vec::new();
    let mut s = a0;
    while ((a1 - s) * direction) > max {
        let e = s + step;
        chunks.push((s, e));
        s = e;
    }
    chunks.push((s, a1));
    chunks
}

pub(super) fn sweep_fraction(theta: f32, start: f32, end: f32) -> f32 {
    let span = end - start;
    if span.abs() < f32::EPSILON {
        0.0
    } else {
        ((theta - start) / span).clamp(0.0, 1.0)
    }
}

pub(super) fn arc_color_at(fill: &ArcFill, t: f32) -> Color {
    match fill {
        ArcFill::Solid(c) => *c,
        ArcFill::Gradient { start, end } => start.mix(*end, t),
    }
}

pub(super) fn arc_to_femtovg_angle(angle: f32) -> f32 {
    angle - std::f32::consts::FRAC_PI_2
}

pub(super) fn arc_point(cx: f32, cy: f32, radius: f32, angle: f32) -> (f32, f32) {
    let femtovg_angle = arc_to_femtovg_angle(angle);
    (
        cx + radius * femtovg_angle.cos(),
        cy + radius * femtovg_angle.sin(),
    )
}

/// Compute the two linear-gradient endpoints for a bounding box `(x, y, w, h)`
/// at `angle` degrees (`0` = top→bottom, `90` = left→right, clockwise).
///
/// The gradient axis passes through the box centre; `start` sits at the
/// minimum projection of the corners onto the angle vector and `end` at the
/// maximum, so the gradient always spans the box (CSS-style).
#[expect(
    clippy::many_single_char_names,
    reason = "x/y/w/h/t are conventional bounding-box and projection names"
)]
pub(super) fn linear_endpoints(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    angle: f32,
) -> ((f32, f32), (f32, f32)) {
    let rad = angle.to_radians();
    let (dx, dy) = (rad.sin(), rad.cos());
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)];
    let mut t_min = f32::INFINITY;
    let mut t_max = f32::NEG_INFINITY;
    for (px, py) in corners {
        let t = (px - cx) * dx + (py - cy) * dy;
        t_min = t_min.min(t);
        t_max = t_max.max(t);
    }
    (
        (cx + t_min * dx, cy + t_min * dy),
        (cx + t_max * dx, cy + t_max * dy),
    )
}

/// Build a FemtoVG `Path` from a sequence of points.
///
/// - `smooth = false`: straight line segments (`move_to` + `line_to`).
/// - `smooth = true`: Catmull-Rom spline converted to cubic Bézier curves.
///   Each segment between `p[i]` and `p[i+1]` uses control points derived
///   from neighboring points, producing a smooth curve through all points.
pub(super) fn build_femtovg_path(points: &[(f32, f32)], closed: bool, smooth: bool) -> Path {
    let mut path = Path::new();

    if smooth && points.len() >= 2 {
        let n = points.len();
        path.move_to(points[0].0, points[0].1);

        for i in 0..n - 1 {
            let p0 = points[if i == 0 { 0 } else { i - 1 }];
            let p1 = points[i];
            let p2 = points[i + 1];
            let p3 = points[if i + 2 < n { i + 2 } else { n - 1 }];

            // Catmull-Rom → cubic Bézier control points (tension = 1.0)
            let cp1x = p1.0 + (p2.0 - p0.0) / 6.0;
            let cp1y = p1.1 + (p2.1 - p0.1) / 6.0;
            let cp2x = p2.0 - (p3.0 - p1.0) / 6.0;
            let cp2y = p2.1 - (p3.1 - p1.1) / 6.0;

            path.bezier_to(cp1x, cp1y, cp2x, cp2y, p2.0, p2.1);
        }
    } else {
        path.move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] {
            path.line_to(x, y);
        }
    }

    if closed {
        path.close();
    }
    path
}
