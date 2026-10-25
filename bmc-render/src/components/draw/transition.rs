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

//! Draw-value transitions: what a draw's previous values were, and the frames between.

use crate::PrevDrawValues;
use crate::animation::interpolate_color;
use crate::tree::Draw;
use bmc_ui::ColorSpace;

use glam::{Quat, Vec3};

/// Extract the static values from a draw command's innermost content for transition tracking.
#[expect(clippy::too_many_lines)]
pub(crate) fn extract_draw_values(draw: &Draw) -> PrevDrawValues {
    match draw {
        Draw::Bitmap { x, y, w, h, .. } | Draw::NinePatch { x, y, w, h, .. } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            ..Default::default()
        },
        Draw::Qr { x, y, size, .. } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *size,
            h: *size,
            ..Default::default()
        },
        Draw::Sphere {
            x,
            y,
            w,
            h,
            center_lat,
            center_lon,
            zoom,
            light_lat,
            light_lon,
            ..
        } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            center_lat: *center_lat,
            center_lon: *center_lon,
            zoom: *zoom,
            light_lat: *light_lat,
            light_lon: *light_lon,
            ..Default::default()
        },
        Draw::Mesh {
            x, y, w, h, args, ..
        } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            orientation: Quat::from_xyzw(
                args.transform.quat[0],
                args.transform.quat[1],
                args.transform.quat[2],
                args.transform.quat[3],
            ),
            fov: args.transform.fov,
            distance: args.transform.distance,
            mesh_scale: args.transform.scale,
            position: Vec3::new(
                args.transform.position[0],
                args.transform.position[1],
                args.transform.position[2],
            ),
            light_pitch: args.lighting.pitch,
            light_yaw: args.lighting.yaw,
            ambient: args.lighting.ambient,
            specular: args.lighting.specular,
            ..Default::default()
        },
        Draw::Rect { x, y, w, h, fill } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            color: fill.primary_color(),
            ..Default::default()
        },
        Draw::Svg {
            x, y, w, h, color, ..
        } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            color: *color,
            ..Default::default()
        },
        Draw::Circle {
            cx, cy, r, fill, ..
        } => PrevDrawValues {
            x: *cx,
            y: *cy,
            w: *r,
            color: fill.primary_color(),
            ..Default::default()
        },
        Draw::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill,
            ..
        } => PrevDrawValues {
            x: *cx,
            y: *cy,
            w: *radius,
            color: fill.primary_color(),
            arc_start_angle: *start_angle,
            arc_sweep: *end_angle - *start_angle,
            arc_width: *width,
            ..Default::default()
        },
        Draw::Orbit {
            radius,
            angle,
            inner,
        } => {
            let mut vals = extract_draw_values(inner);
            vals.angle = *angle;
            vals.radius = *radius;
            vals
        }
        Draw::Rotated { angle, inner } => {
            let mut vals = extract_draw_values(inner);
            vals.rotation = *angle;
            vals
        }
        Draw::Centered { inner } | Draw::Modified { inner, .. } | Draw::Shadow { inner, .. } => {
            extract_draw_values(inner)
        }
        Draw::Path { paint, .. } => PrevDrawValues {
            color: paint.primary_color(),
            ..Default::default()
        },
        Draw::Text { x, y, style, .. } => PrevDrawValues {
            x: *x,
            y: *y,
            color: style.color,
            ..Default::default()
        },
        Draw::CurvedText {
            cx,
            cy,
            radius,
            angle,
            style,
            ..
        } => PrevDrawValues {
            x: *cx,
            y: *cy,
            radius: *radius,
            angle: *angle,
            color: style.color,
            ..Default::default()
        },
        Draw::AutofitText {
            x,
            y,
            box_width,
            box_height,
            ..
        } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *box_width,
            h: *box_height,
            ..Default::default()
        },
    }
}

/// Whether a sweep closes into a ring,
/// allowing for the rounding of a sweep summed up to `TAU`.
pub(crate) fn is_full_turn(start: f32, end: f32) -> bool {
    (end - start).abs() >= std::f32::consts::TAU - 1e-4
}

/// Shortest-path delta for angle interpolation (wraps around TAU).
pub(crate) fn shortest_angle_delta(from: f32, to: f32) -> f32 {
    let delta = to - from;
    let wrapped = normalize_angle(delta);
    if wrapped.to_bits() == (-std::f32::consts::PI).to_bits() && delta.is_sign_positive() {
        std::f32::consts::PI
    } else {
        wrapped
    }
}

pub(super) fn shortest_angle_delta_deg(from: f32, to: f32) -> f32 {
    let delta = to - from;
    let wrapped = normalize_angle_deg(delta);
    if wrapped.to_bits() == (-180.0_f32).to_bits() && delta.is_sign_positive() {
        180.0
    } else {
        wrapped
    }
}

fn normalize_angle(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

pub(super) fn normalize_angle_deg(angle: f32) -> f32 {
    (angle + 180.0).rem_euclid(360.0) - 180.0
}

fn interpolate_angle(from: f32, target: f32, t: f32) -> f32 {
    target - shortest_angle_delta(from, target) * (1.0 - t)
}

fn interpolate_angle_deg(from: f32, target: f32, t: f32) -> f32 {
    target - shortest_angle_delta_deg(from, target) * (1.0 - t)
}

/// Linearly interpolate between two sets of draw values.
pub(super) fn interpolate_draw_values(
    a: &PrevDrawValues,
    b: &PrevDrawValues,
    t: f32,
    color_space: ColorSpace,
) -> PrevDrawValues {
    PrevDrawValues {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        w: a.w + (b.w - a.w) * t,
        h: a.h + (b.h - a.h) * t,
        color: if a.color == b.color {
            a.color
        } else {
            interpolate_color(a.color, b.color, t, color_space)
        },
        angle: interpolate_angle(a.angle, b.angle, t),
        radius: a.radius + (b.radius - a.radius) * t,
        rotation: interpolate_angle(a.rotation, b.rotation, t),
        arc_start_angle: interpolate_angle(a.arc_start_angle, b.arc_start_angle, t),
        arc_sweep: a.arc_sweep + (b.arc_sweep - a.arc_sweep) * t,
        arc_width: a.arc_width + (b.arc_width - a.arc_width) * t,
        center_lat: a.center_lat + (b.center_lat - a.center_lat) * t,
        center_lon: interpolate_angle_deg(a.center_lon, b.center_lon, t),
        zoom: a.zoom + (b.zoom - a.zoom) * t,
        light_lat: a.light_lat + (b.light_lat - a.light_lat) * t,
        light_lon: interpolate_angle_deg(a.light_lon, b.light_lon, t),
        // Mesh fields — slerp for quaternion, linear for the rest
        orientation: slerp_quat(a.orientation, b.orientation, t),
        fov: a.fov + (b.fov - a.fov) * t,
        distance: a.distance + (b.distance - a.distance) * t,
        mesh_scale: a.mesh_scale + (b.mesh_scale - a.mesh_scale) * t,
        position: a.position.lerp(b.position, t),
        light_pitch: a.light_pitch + (b.light_pitch - a.light_pitch) * t,
        light_yaw: a.light_yaw + (b.light_yaw - a.light_yaw) * t,
        ambient: a.ambient + (b.ambient - a.ambient) * t,
        specular: a.specular + (b.specular - a.specular) * t,
    }
}

/// Spherical linear interpolation for quaternions.
///
/// Delegates to `glam::Quat::slerp`, which handles short-path selection
/// and is SIMD-accelerated when available. Pricier than nlerp (one `acos`
/// plus two `sin` per call) but the math is correct on the unit
/// hypersphere regardless of the angle between `a` and `b`.
fn slerp_quat(a: Quat, b: Quat, t: f32) -> Quat {
    a.slerp(b, t)
}
