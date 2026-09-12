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

//! Canvas draw command rendering.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::cast_possible_wrap,
    reason = "canvas rendering casts freely between f32 coordinates and small, \
              bounded integer indices (pixel positions, QR module counts)"
)]
#![allow(clippy::wildcard_imports)]

use glam::{Quat, Vec3};

use bmc_wasm_protocol::*;

use crate::animation::{apply_easing, compute_animation_value, interpolate_color};
use crate::gpu::mesh::{MeshDrawArgs, MeshLighting, MeshTransform};
use crate::renderer::{RenderTarget, Renderer};
use crate::tree::{AnimationContext, DrawCommand, HostAnimationDef};
use crate::{AnimationState, PrevDrawValues, TransitionState};

/// Interpolated mesh parameters for transition override.
/// Wraps the same `MeshDrawArgs` shape rendered later,
/// so override application is a struct copy
/// instead of a 21-field shuffle.
#[derive(Debug, Clone, Copy)]
struct MeshOverride {
    args: MeshDrawArgs,
}

#[derive(Debug, Clone)]
struct ArcOverride {
    cx: f32,
    cy: f32,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    width: f32,
    segments: ArcSegments,
}

/// Apply the canvas-level `color_override` (forces a solid recolour) and
/// `alpha` opacity to a shape's `fill`.
fn effective_fill(fill: &Fill, color_override: Option<Color>, alpha: f32) -> Fill {
    let base = match color_override {
        Some(c) => Fill::Solid(c),
        None => *fill,
    };
    if alpha < 1.0 {
        base.scale_alpha(alpha)
    } else {
        base
    }
}

fn effective_arc_fill(fill: &ArcFill, color_override: Option<Color>, alpha: f32) -> ArcFill {
    // A colour override recolours a solid arc, but a gradient can't be expressed
    // as one colour — the transition interpolates a gradient's primary colour, so
    // applying the override here would flatten the gradient mid-animation. Leave
    // gradients to render as-is.
    let base = match (color_override, fill) {
        (Some(c), ArcFill::Solid(_)) => ArcFill::Solid(c),
        _ => *fill,
    };
    if alpha < 1.0 {
        base.scale_alpha(alpha)
    } else {
        base
    }
}

/// Resolve the stroke colour for a polyline: `color_override` wins, else the
/// path's stroke colour, then scaled by `alpha`.
fn stroke_color(color: Color, color_override: Option<Color>, alpha: f32) -> Color {
    let base = color_override.unwrap_or(color);
    if alpha < 1.0 {
        base.scale_alpha(alpha)
    } else {
        base
    }
}

/// Most runs one path can usefully produce.
/// A path needing more than this at its period is far longer
/// than anything on screen, and walking it would tie up
/// the render thread building runs nobody sees.
const MAX_DASH_RUNS: f32 = 10_000.0;

/// Total arc length of a polyline, or `None` if the points don't describe
/// one that can be measured — non-finite coordinates, or a length past `f32`.
fn path_length(points: &[(f32, f32)]) -> Option<f32> {
    let total: f32 = points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    total.is_finite().then_some(total)
}

/// Painted runs of a `(on, off)` dash pattern, walked by arc length.
/// Re-validated here because the caller scales the pattern,
/// which can shrink a sound one below the walkable minimum.
/// Anything rejected draws as one solid run.
fn dash_runs(points: &[(f32, f32)], on: f32, off: f32) -> Vec<Vec<(f32, f32)>> {
    let Some(Dash { on, off }) = Dash::new(on, off) else {
        return vec![points.to_vec()];
    };
    // The walk cannot spin, but it can still grind: a segment far longer
    // than its period costs one run per period before the guard below stops it.
    // Decide that up front rather than after millions of allocations.
    let unwalkable = path_length(points).is_none_or(|len| len / (on + off) > MAX_DASH_RUNS);
    if unwalkable {
        return vec![points.to_vec()];
    }
    let mut runs: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut painting = true;
    // distance left before the on/off band flips
    let mut budget = on;
    let mut run: Vec<(f32, f32)> = vec![points[0]];
    for pair in points.windows(2) {
        let (ax, ay) = pair[0];
        let (bx, by) = pair[1];
        let (dx, dy) = (bx - ax, by - ay);
        let len = dx.hypot(dy);
        if len <= 0.0 {
            continue;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mut walked = 0.0_f32;
        // `>=` so a band ending exactly on the vertex flips here rather than
        // carrying to the next segment, which would leave `budget` at zero.
        // Exiting on a strict `<` also keeps `budget` positive below, so no
        // later segment can inherit a phase that never flips again.
        while len - walked >= budget {
            // Points come from the guest too: at coordinates where the ULP
            // exceeds `budget`, the walk stops advancing and would spin here.
            let next = walked + budget;
            if next <= walked {
                break;
            }
            walked = next;
            let point = (ax + ux * walked, ay + uy * walked);
            if painting {
                run.push(point);
                if run.len() >= 2 {
                    runs.push(core::mem::take(&mut run));
                } else {
                    run.clear();
                }
            } else {
                run = vec![point];
            }
            painting = !painting;
            budget = if painting { on } else { off };
        }
        budget -= len - walked;
        // A flip landing on the vertex already recorded it as the run's start.
        if painting && run.last() != Some(&(bx, by)) {
            run.push((bx, by));
        }
    }
    if painting && run.len() >= 2 {
        runs.push(run);
    }
    runs
}

pub(crate) fn render_draw_command(
    renderer: &mut RenderTarget<'_, '_, '_>,
    draw: &DrawCommand,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    anim_ctx: &mut AnimationContext<'_>,
) {
    render_draw_inner(
        renderer, draw, cx, cy, cw, ch, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, None, anim_ctx,
    );
}

/// Get the bounds (width, height) of a draw command
pub(crate) fn get_draw_bounds(draw: &DrawCommand) -> (f32, f32) {
    match draw {
        DrawCommand::Rect { w, h, .. }
        | DrawCommand::Svg { w, h, .. }
        | DrawCommand::Bitmap { w, h, .. }
        | DrawCommand::Sphere { w, h, .. }
        | DrawCommand::Mesh { w, h, .. }
        | DrawCommand::NinePatch { w, h, .. } => (*w, *h),
        DrawCommand::Circle { r, .. } => (*r * 2.0, *r * 2.0),
        DrawCommand::Qr { size, .. } => (*size, *size),
        DrawCommand::Arc { radius, width, .. } => {
            let d = 2.0 * radius + width;
            (d, d)
        }
        DrawCommand::Centered { inner }
        | DrawCommand::Rotated { inner, .. }
        | DrawCommand::Modified { inner, .. }
        | DrawCommand::Shadow { inner, .. }
        | DrawCommand::Orbit { inner, .. } => get_draw_bounds(inner),
        DrawCommand::Text { .. } | DrawCommand::CurvedText { .. } => (0.0, 0.0),
        DrawCommand::AutofitText {
            box_width,
            box_height,
            ..
        } => (*box_width, *box_height),
        DrawCommand::Path { points, .. } => {
            if points.is_empty() {
                (0.0, 0.0)
            } else {
                let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
                let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
                for &(x, y) in points {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
                (max_x - min_x, max_y - min_y)
            }
        }
    }
}

/// QR error correction: Medium (~15% recoverable) leaves enough slack to scan a
/// code photographed off a screen, without the denser modules a higher level
/// would force into the same footprint. A fixed policy — not a widget knob.
const QR_ECC: qrcodegen::QrCodeEcc = qrcodegen::QrCodeEcc::Medium;

/// Encode `text` and rasterise it into an `es`×`es` square at `(ox, oy)`, quiet
/// zone included. Dark modules merge into per-row runs, and every module edge is
/// snapped to a whole pixel so neighbours abut without anti-aliased seams. Text
/// too long to encode draws nothing.
#[expect(clippy::too_many_arguments)]
fn render_qr(
    renderer: &mut dyn Renderer,
    ox: f32,
    oy: f32,
    es: f32,
    dark: Color,
    light: Color,
    quiet_zone: u8,
    text: &str,
) {
    let Ok(code) = qrcodegen::QrCode::encode_text(text, QR_ECC) else {
        return;
    };
    let n = code.size() as usize;
    let qz = usize::from(quiet_zone);
    let span = n + 2 * qz;
    let unit = es / span as f32;
    let edge = |i: usize| (unit * i as f32).round();
    let module = |row: usize, col: usize| code.get_module(col as i32, row as i32);

    if light.to_u32() != TRANSPARENT.to_u32() {
        renderer.fill_rect(ox, oy, es, es, light);
    }
    for row in 0..n {
        let top = oy + edge(qz + row);
        let height = oy + edge(qz + row + 1) - top;
        let mut col = 0;
        while col < n {
            if !module(row, col) {
                col += 1;
                continue;
            }
            let start = col;
            while col < n && module(row, col) {
                col += 1;
            }
            let left = ox + edge(qz + start);
            let width = ox + edge(qz + col) - left;
            renderer.fill_rect(left, top, width, height, dark);
        }
    }
}

/// Render a draw command with accumulated transforms and animation modifiers.
#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_draw_inner(
    renderer: &mut RenderTarget<'_, '_, '_>,
    draw: &DrawCommand,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    offset_x: f32,
    offset_y: f32,
    rotation: f32,
    scale: f32,
    alpha: f32,
    orbit_angle_offset: f32,
    color_override: Option<Color>,
    anim_ctx: &mut AnimationContext<'_>,
) {
    match draw {
        DrawCommand::Rect { x, y, w, h, fill } => {
            let ew = *w * scale;
            let eh = *h * scale;
            // Center-anchored scaling: offset by half the size difference
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            let paint = effective_fill(fill, color_override, alpha);
            if rotation == 0.0 {
                renderer.fill_rect_paint(rx, ry, ew, eh, &paint);
            } else {
                // Rotate around canvas center (like CSS transform-origin: center)
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.fill_rect_paint(rx - pivot_x, ry - pivot_y, ew, eh, &paint);
                renderer.restore();
            }
        }
        DrawCommand::Svg {
            x,
            y,
            w,
            h,
            color,
            icon_id,
            anti_alias,
            fills,
        } => {
            let Some(icon_id) = *icon_id else { return };
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            let base_color = color_override.unwrap_or(*color);
            let final_color = if alpha < 1.0 {
                base_color.scale_alpha(alpha)
            } else {
                base_color
            };
            if rotation == 0.0 {
                renderer.draw_svg(rx, ry, ew, eh, final_color, icon_id, *anti_alias, fills);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_svg(
                    rx - pivot_x,
                    ry - pivot_y,
                    ew,
                    eh,
                    final_color,
                    icon_id,
                    *anti_alias,
                    fills,
                );
                renderer.restore();
            }
        }
        DrawCommand::Bitmap {
            x,
            y,
            w,
            h,
            bitmap_id,
        } => {
            let Some(bitmap_id) = *bitmap_id else { return };
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            if rotation == 0.0 {
                renderer.draw_bitmap(rx, ry, ew, eh, bitmap_id);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_bitmap(rx - pivot_x, ry - pivot_y, ew, eh, bitmap_id);
                renderer.restore();
            }
        }
        DrawCommand::Qr {
            x,
            y,
            size,
            dark,
            light,
            quiet_zone,
            text,
        } => {
            let es = *size * scale;
            let sx = *x + offset_x + (*size - es) / 2.0;
            let sy = *y + offset_y + (*size - es) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            let (dark, light) = if alpha < 1.0 {
                (dark.scale_alpha(alpha), light.scale_alpha(alpha))
            } else {
                (*dark, *light)
            };
            if rotation == 0.0 {
                render_qr(&mut *renderer, rx, ry, es, dark, light, *quiet_zone, text);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                render_qr(
                    &mut *renderer,
                    rx - pivot_x,
                    ry - pivot_y,
                    es,
                    dark,
                    light,
                    *quiet_zone,
                    text,
                );
                renderer.restore();
            }
        }
        DrawCommand::NinePatch {
            x,
            y,
            w,
            h,
            bitmap_id,
            left,
            top,
            right,
            bottom,
        } => {
            let Some(bitmap_id) = *bitmap_id else { return };
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            if rotation == 0.0 {
                renderer.draw_nine_patch(rx, ry, ew, eh, bitmap_id, *left, *top, *right, *bottom);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_nine_patch(
                    rx - pivot_x,
                    ry - pivot_y,
                    ew,
                    eh,
                    bitmap_id,
                    *left,
                    *top,
                    *right,
                    *bottom,
                );
                renderer.restore();
            }
        }
        DrawCommand::Circle {
            cx: circle_cx,
            cy: circle_cy,
            r,
            fill,
        } => {
            let er = *r * scale;
            let scx = *circle_cx + offset_x;
            let scy = *circle_cy + offset_y;
            let paint = effective_fill(fill, color_override, alpha);
            renderer.fill_circle_paint(cx + scx, cy + scy, er, &paint);
        }
        DrawCommand::Arc {
            cx: arc_cx,
            cy: arc_cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill,
            segments,
            cap,
        } => {
            let er = *radius * scale;
            let ew = *width * scale;
            let scx = cx + *arc_cx + offset_x;
            let scy = cy + *arc_cy + offset_y;
            let eff = effective_arc_fill(fill, color_override, alpha);
            if rotation == 0.0 {
                renderer.stroke_arc(
                    scx,
                    scy,
                    er,
                    *start_angle,
                    *end_angle,
                    ew,
                    &eff,
                    segments,
                    *cap,
                );
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.stroke_arc(
                    scx - pivot_x,
                    scy - pivot_y,
                    er,
                    *start_angle,
                    *end_angle,
                    ew,
                    &eff,
                    segments,
                    *cap,
                );
                renderer.restore();
            }
        }
        DrawCommand::Centered { inner } => {
            let (iw, ih) = get_draw_bounds(inner);
            let new_offset_x = (cw - iw) / 2.0;
            let new_offset_y = (ch - ih) / 2.0;
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                new_offset_x,
                new_offset_y,
                rotation,
                scale,
                alpha,
                orbit_angle_offset,
                color_override,
                anim_ctx,
            );
        }
        DrawCommand::Orbit {
            radius,
            angle,
            inner,
        } => {
            let effective_angle = *angle + orbit_angle_offset;
            let center_offset_x = cw / 2.0;
            let center_offset_y = ch / 2.0;
            let (iw, ih) = get_draw_bounds(inner);
            let new_offset_x = center_offset_x + radius * effective_angle.cos() - iw / 2.0;
            let new_offset_y = center_offset_y + radius * effective_angle.sin() - ih / 2.0;
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                new_offset_x,
                new_offset_y,
                rotation,
                scale,
                alpha,
                0.0, // orbit_angle_offset consumed
                color_override,
                anim_ctx,
            );
        }
        DrawCommand::Rotated { angle, inner } => {
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                offset_x,
                offset_y,
                rotation + angle,
                scale,
                alpha,
                orbit_angle_offset,
                color_override,
                anim_ctx,
            );
        }
        DrawCommand::Shadow {
            dx,
            dy,
            blur,
            color,
            inner,
        } => {
            // FBO is canvas-sized; the closure draws at FBO-local (0, 0)
            // and `drop_shadow` composites at the canvas origin (cx, cy).
            let fbo_w = cw.ceil().max(1.0) as u32;
            let fbo_h = ch.ceil().max(1.0) as u32;
            renderer.drop_shadow(cx, cy, fbo_w, fbo_h, *dx, *dy, *blur, *color, &mut |r| {
                render_draw_inner(
                    r,
                    inner,
                    0.0,
                    0.0,
                    cw,
                    ch,
                    offset_x,
                    offset_y,
                    rotation,
                    scale,
                    alpha,
                    orbit_angle_offset,
                    color_override,
                    anim_ctx,
                );
            });
        }
        DrawCommand::Modified {
            animations,
            transition,
            color_space,
            inner,
        } => {
            let mut acc_rotation = rotation;
            let mut acc_scale = scale;
            let mut acc_alpha = alpha;
            let mut acc_offset_x = offset_x;
            let mut acc_offset_y = offset_y;
            let mut acc_orbit_angle = orbit_angle_offset;
            let mut acc_color: Option<Color> = color_override;
            let mut sphere_override: Option<(f32, f32, f32, f32, f32)> = None;
            let mut mesh_override: Option<MeshOverride> = None;
            let mut arc_override: Option<ArcOverride> = None;

            // Process animations
            for anim_def in animations {
                let key = animation_key(anim_def, anim_ctx.draw_counter);
                let state =
                    anim_ctx
                        .animation_states
                        .entry(key)
                        .or_insert_with(|| AnimationState {
                            elapsed_ms: 0,
                            last_seen_frame: anim_ctx.frame_counter,
                        });
                state.last_seen_frame = anim_ctx.frame_counter;

                let (value, active) = compute_animation_value(anim_def, state, anim_ctx.delta_ms);
                if active {
                    anim_ctx.has_active = true;
                }

                match anim_def.property {
                    AnimProperty::Rotate => acc_rotation += value,
                    AnimProperty::Scale => acc_scale *= value,
                    AnimProperty::Alpha => acc_alpha *= value,
                    AnimProperty::TranslateX => acc_offset_x += value,
                    AnimProperty::TranslateY => acc_offset_y += value,
                    AnimProperty::OrbitAngle => acc_orbit_angle += value,
                    AnimProperty::Color => {
                        let from_color = Color::from_raw(f32::to_bits(anim_def.from));
                        let to_color = Color::from_raw(f32::to_bits(anim_def.to));
                        // value is the raw lerped f32, recompute t for color
                        let range = anim_def.to - anim_def.from;
                        let t = if range.abs() > f32::EPSILON {
                            (value - anim_def.from) / range
                        } else {
                            0.0
                        };
                        acc_color = Some(interpolate_color(from_color, to_color, t, *color_space));
                    }
                }
            }

            // Process transition.
            //
            // The key is `(canvas_index, id_hash)` so transition state
            // follows the widget-supplied id across tree-shape changes
            // — an optional sibling appearing or disappearing no
            // longer reshuffles state into the wrong draws.
            if let Some(trans_def) = transition {
                let current_values = extract_draw_values(inner);
                let key = (anim_ctx.canvas_index, trans_def.id_hash);
                let state = anim_ctx.transition_states.entry(key).or_insert_with(|| {
                    TransitionState {
                        from: current_values,
                        target: current_values,
                        duration_ms: trans_def.duration_ms,
                        easing: trans_def.easing,
                        elapsed_ms: trans_def.duration_ms, // start finished
                        last_seen_frame: anim_ctx.frame_counter,
                    }
                });
                state.last_seen_frame = anim_ctx.frame_counter;

                // Charged to the transition that was running, since `delta_ms`
                // elapsed before any new target appeared. Advancing before the
                // retarget below anchors `from` where that transition actually
                // reached, while the new one still starts at zero — so a
                // slow-rendering widget finishes the old transition instead of
                // skipping the new one.
                //
                // Advancing only on unchanged-target frames strands `from`
                // instead: a guest submitting a new value every frame
                // re-anchors at `t = 0` forever, and the draw holds
                // its first-ever value for the length of the interaction.
                state.elapsed_ms = state.elapsed_ms.saturating_add(anim_ctx.delta_ms);
                if trans_def.duration_ms == 0 {
                    state.from = current_values;
                    state.target = current_values;
                    state.duration_ms = 0;
                    state.easing = trans_def.easing;
                    state.elapsed_ms = 0;
                } else if state.target != current_values {
                    // D3-style: interpolate from current interpolated position
                    let t = if state.duration_ms == 0 {
                        1.0
                    } else {
                        (state.elapsed_ms as f32 / state.duration_ms as f32).min(1.0)
                    };
                    let eased_t = apply_easing(state.easing, t);
                    state.from =
                        interpolate_draw_values(&state.from, &state.target, eased_t, *color_space);
                    state.target = current_values;
                    state.duration_ms = trans_def.duration_ms;
                    state.easing = trans_def.easing;
                    state.elapsed_ms = 0;
                }

                if state.elapsed_ms < state.duration_ms {
                    anim_ctx.has_active = true;
                    let t = state.elapsed_ms as f32 / state.duration_ms as f32;
                    let eased_t = apply_easing(state.easing, t);
                    let interp =
                        interpolate_draw_values(&state.from, &state.target, eased_t, *color_space);
                    if let DrawCommand::Arc { segments, .. } = inner.as_ref() {
                        // Segments stay at their absolute positions; the
                        // interpolated sweep clips them in the renderer, so the
                        // arc's length animates in place rather than remapping.
                        arc_override = Some(ArcOverride {
                            cx: interp.x,
                            cy: interp.y,
                            radius: interp.w,
                            start_angle: interp.arc_start_angle,
                            end_angle: interp.arc_start_angle + interp.arc_sweep,
                            width: interp.arc_width,
                            segments: segments.clone(),
                        });
                    } else {
                        acc_offset_x += interp.x - current_values.x;
                        acc_offset_y += interp.y - current_values.y;
                        acc_scale *= if current_values.w > 0.0 {
                            interp.w / current_values.w
                        } else {
                            1.0
                        };
                        acc_orbit_angle += interp.angle - current_values.angle;
                        acc_rotation += interp.rotation - current_values.rotation;
                    }
                    if interp.color != current_values.color {
                        acc_color = Some(interp.color);
                    }
                    if matches!(inner.as_ref(), DrawCommand::Sphere { .. }) {
                        sphere_override = Some((
                            interp.center_lat,
                            interp.center_lon,
                            interp.zoom,
                            interp.light_lat,
                            interp.light_lon,
                        ));
                    }
                    if let DrawCommand::Mesh { args, .. } = inner.as_ref() {
                        mesh_override = Some(MeshOverride {
                            args: MeshDrawArgs {
                                transform: MeshTransform {
                                    fov: interp.fov,
                                    distance: interp.distance,
                                    quat: [
                                        interp.orientation.x,
                                        interp.orientation.y,
                                        interp.orientation.z,
                                        interp.orientation.w,
                                    ],
                                    position: [
                                        interp.position.x,
                                        interp.position.y,
                                        interp.position.z,
                                    ],
                                    scale: interp.mesh_scale,
                                },
                                lighting: MeshLighting {
                                    pitch: interp.light_pitch,
                                    yaw: interp.light_yaw,
                                    ambient: interp.ambient,
                                    specular: interp.specular,
                                },
                                // Highlight is not interpolated; carry the
                                // current draw's value through.
                                highlight: args.highlight,
                            },
                        });
                    }
                }
            }

            anim_ctx.draw_counter += 1;

            if let (
                Some((center_lat, center_lon, zoom, light_lat, light_lon)),
                DrawCommand::Sphere {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                    atmosphere,
                    ..
                },
            ) = (sphere_override, inner.as_ref())
            {
                let overridden = DrawCommand::Sphere {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                    bitmap_id: *bitmap_id,
                    atmosphere: *atmosphere,
                    center_lat,
                    center_lon,
                    zoom,
                    light_lat,
                    light_lon,
                };
                render_draw_inner(
                    renderer,
                    &overridden,
                    cx,
                    cy,
                    cw,
                    ch,
                    acc_offset_x,
                    acc_offset_y,
                    acc_rotation,
                    acc_scale,
                    acc_alpha,
                    acc_orbit_angle,
                    acc_color,
                    anim_ctx,
                );
            } else if let (
                Some(mo),
                DrawCommand::Mesh {
                    x,
                    y,
                    w,
                    h,
                    mesh_id,
                    ..
                },
            ) = (mesh_override, inner.as_ref())
            {
                let overridden = DrawCommand::Mesh {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                    mesh_id: *mesh_id,
                    args: mo.args,
                };
                render_draw_inner(
                    renderer,
                    &overridden,
                    cx,
                    cy,
                    cw,
                    ch,
                    acc_offset_x,
                    acc_offset_y,
                    acc_rotation,
                    acc_scale,
                    acc_alpha,
                    acc_orbit_angle,
                    acc_color,
                    anim_ctx,
                );
            } else if let (Some(ao), DrawCommand::Arc { fill, cap, .. }) =
                (arc_override, inner.as_ref())
            {
                let overridden = DrawCommand::Arc {
                    cx: ao.cx,
                    cy: ao.cy,
                    radius: ao.radius,
                    start_angle: ao.start_angle,
                    end_angle: ao.end_angle,
                    width: ao.width,
                    fill: *fill,
                    segments: ao.segments,
                    cap: *cap,
                };
                render_draw_inner(
                    renderer,
                    &overridden,
                    cx,
                    cy,
                    cw,
                    ch,
                    acc_offset_x,
                    acc_offset_y,
                    acc_rotation,
                    acc_scale,
                    acc_alpha,
                    acc_orbit_angle,
                    acc_color,
                    anim_ctx,
                );
            } else {
                render_draw_inner(
                    renderer,
                    inner,
                    cx,
                    cy,
                    cw,
                    ch,
                    acc_offset_x,
                    acc_offset_y,
                    acc_rotation,
                    acc_scale,
                    acc_alpha,
                    acc_orbit_angle,
                    acc_color,
                    anim_ctx,
                );
            }
        }
        DrawCommand::Path {
            points,
            paint,
            closed,
            smooth,
        } => {
            if points.len() < 2 {
                return;
            }
            // Transform points: apply canvas offset + accumulated offset + scale
            let transformed: Vec<(f32, f32)> = points
                .iter()
                .map(|&(px, py)| (cx + (px + offset_x) * scale, cy + (py + offset_y) * scale))
                .collect();

            let pivoted: Vec<(f32, f32)>;
            let pts = if rotation == 0.0 {
                &transformed
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                pivoted = transformed
                    .iter()
                    .map(|&(px, py)| (px - pivot_x, py - pivot_y))
                    .collect();
                &pivoted
            };

            match paint {
                PathPaint::Fill(fill) => {
                    renderer.fill_path_paint(
                        pts,
                        &effective_fill(fill, color_override, alpha),
                        *smooth,
                    );
                }
                PathPaint::Stroke { color, width, dash } => {
                    let paint_color = stroke_color(*color, color_override, alpha);
                    if let Some(d) = dash {
                        // scale the pattern like the width
                        for run in dash_runs(pts, d.on * scale, d.off * scale) {
                            renderer.stroke_path(&run, *width * scale, paint_color, false, false);
                        }
                    } else {
                        renderer.stroke_path(pts, *width * scale, paint_color, *closed, *smooth);
                    }
                }
            }

            if rotation != 0.0 {
                renderer.restore();
            }
        }
        DrawCommand::Text { x, y, text, style } => {
            let rx = cx + *x + offset_x;
            let ry = cy + *y + offset_y;
            let mut render_style = *style;
            render_style.size = (style.size as f32 * scale) as u32;
            let base_color = color_override.unwrap_or(style.color);
            render_style.color = if alpha < 1.0 {
                base_color.scale_alpha(alpha)
            } else {
                base_color
            };
            if rotation == 0.0 {
                renderer.draw_canvas_text(text, rx, ry, &render_style);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_canvas_text(text, rx - pivot_x, ry - pivot_y, &render_style);
                renderer.restore();
            }
        }
        DrawCommand::CurvedText {
            cx: local_cx,
            cy: local_cy,
            radius,
            angle,
            anchor,
            facing,
            text,
            style,
        } => {
            let rx = cx + *local_cx + offset_x;
            let ry = cy + *local_cy + offset_y;
            let mut render_style = *style;
            render_style.size = (style.size as f32 * scale) as u32;
            let base_color = color_override.unwrap_or(style.color);
            render_style.color = if alpha < 1.0 {
                base_color.scale_alpha(alpha)
            } else {
                base_color
            };
            let scaled_radius = *radius * scale;

            if rotation == 0.0 {
                renderer.draw_curved_text(
                    rx,
                    ry,
                    scaled_radius,
                    *angle,
                    *anchor,
                    *facing,
                    text,
                    &render_style,
                );
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_curved_text(
                    rx - pivot_x,
                    ry - pivot_y,
                    scaled_radius,
                    *angle,
                    *anchor,
                    *facing,
                    text,
                    &render_style,
                );
                renderer.restore();
            }
        }
        DrawCommand::AutofitText {
            x,
            y,
            box_width,
            box_height,
            mode,
            min_size,
            max_size,
            text,
            style,
        } => {
            let rx = cx + *x + offset_x;
            let ry = cy + *y + offset_y;
            let bw = *box_width * scale;
            let bh = *box_height * scale;
            let mut render_style = *style;
            // Scale the font size and search bounds with the box, exactly as the
            // `Text`/`CurvedText` arms scale `render_style.size`. Otherwise the box
            // lives in scaled (device) units while the size cap and bounds stay in
            // unscaled units, and autofit text under a non-unit scale renders at a
            // different size than a plain scaled `Text` beside it.
            render_style.size = (style.size as f32 * scale) as u32;
            let min_size = (f32::from(*min_size) * scale) as u16;
            let max_size = (f32::from(*max_size) * scale) as u16;
            let base_color = color_override.unwrap_or(style.color);
            render_style.color = if alpha < 1.0 {
                base_color.scale_alpha(alpha)
            } else {
                base_color
            };
            if rotation == 0.0 {
                renderer.draw_autofit_text(
                    rx,
                    ry,
                    bw,
                    bh,
                    text,
                    &render_style,
                    *mode,
                    min_size,
                    max_size,
                );
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_autofit_text(
                    rx - pivot_x,
                    ry - pivot_y,
                    bw,
                    bh,
                    text,
                    &render_style,
                    *mode,
                    min_size,
                    max_size,
                );
                renderer.restore();
            }
        }
        DrawCommand::Sphere {
            x,
            y,
            w,
            h,
            bitmap_id,
            atmosphere,
            center_lat,
            center_lon,
            zoom,
            light_lat,
            light_lon,
        } => {
            let Some(bitmap_id) = *bitmap_id else { return };
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            if rotation == 0.0 {
                renderer.draw_sphere(
                    rx,
                    ry,
                    ew,
                    eh,
                    bitmap_id,
                    *center_lat,
                    *center_lon,
                    *zoom,
                    *light_lat,
                    *light_lon,
                    *atmosphere,
                );
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_sphere(
                    rx - pivot_x,
                    ry - pivot_y,
                    ew,
                    eh,
                    bitmap_id,
                    *center_lat,
                    *center_lon,
                    *zoom,
                    *light_lat,
                    *light_lon,
                    *atmosphere,
                );
                renderer.restore();
            }
        }
        DrawCommand::Mesh {
            x,
            y,
            w,
            h,
            mesh_id,
            args,
        } => {
            let Some(mesh_id) = *mesh_id else { return };
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            let slot = anim_ctx.mesh_slot_counter;
            anim_ctx.mesh_slot_counter = slot.saturating_add(1);
            if rotation == 0.0 {
                renderer.draw_mesh(rx, ry, ew, eh, slot, mesh_id, *args);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_mesh(rx - pivot_x, ry - pivot_y, ew, eh, slot, mesh_id, *args);
                renderer.restore();
            }
        }
    }
}

/// Compute a content-based hash key for an animation definition + draw counter salt.
fn animation_key(def: &HostAnimationDef, draw_counter: u32) -> u64 {
    // Simple FNV-like hash of the animation definition bytes
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    h ^= def.property as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.from.to_bits() as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.to.to_bits() as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.duration_ms as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.delay_ms as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.easing as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.loop_mode as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^ draw_counter as u64
}

/// Extract the static values from a draw command's innermost content for transition tracking.
#[expect(clippy::too_many_lines)]
pub(crate) fn extract_draw_values(draw: &DrawCommand) -> PrevDrawValues {
    match draw {
        DrawCommand::Bitmap { x, y, w, h, .. } | DrawCommand::NinePatch { x, y, w, h, .. } => {
            PrevDrawValues {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                ..Default::default()
            }
        }
        DrawCommand::Qr { x, y, size, .. } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *size,
            h: *size,
            ..Default::default()
        },
        DrawCommand::Sphere {
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
        DrawCommand::Mesh {
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
        DrawCommand::Rect { x, y, w, h, fill } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            color: fill.primary_color(),
            ..Default::default()
        },
        DrawCommand::Svg {
            x, y, w, h, color, ..
        } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            color: *color,
            ..Default::default()
        },
        DrawCommand::Circle {
            cx, cy, r, fill, ..
        } => PrevDrawValues {
            x: *cx,
            y: *cy,
            w: *r,
            color: fill.primary_color(),
            ..Default::default()
        },
        DrawCommand::Arc {
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
        DrawCommand::Orbit {
            radius,
            angle,
            inner,
        } => {
            let mut vals = extract_draw_values(inner);
            vals.angle = *angle;
            vals.radius = *radius;
            vals
        }
        DrawCommand::Rotated { angle, inner } => {
            let mut vals = extract_draw_values(inner);
            vals.rotation = *angle;
            vals
        }
        DrawCommand::Centered { inner }
        | DrawCommand::Modified { inner, .. }
        | DrawCommand::Shadow { inner, .. } => extract_draw_values(inner),
        DrawCommand::Path { paint, .. } => PrevDrawValues {
            color: paint.primary_color(),
            ..Default::default()
        },
        DrawCommand::Text { x, y, style, .. } => PrevDrawValues {
            x: *x,
            y: *y,
            color: style.color,
            ..Default::default()
        },
        DrawCommand::CurvedText {
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
        DrawCommand::AutofitText {
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

fn shortest_angle_delta_deg(from: f32, to: f32) -> f32 {
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

fn normalize_angle_deg(angle: f32) -> f32 {
    (angle + 180.0).rem_euclid(360.0) - 180.0
}

fn interpolate_angle(from: f32, target: f32, t: f32) -> f32 {
    target - shortest_angle_delta(from, target) * (1.0 - t)
}

fn interpolate_angle_deg(from: f32, target: f32, t: f32) -> f32 {
    target - shortest_angle_delta_deg(from, target) * (1.0 - t)
}

/// Linearly interpolate between two sets of draw values.
fn interpolate_draw_values(
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use super::*;
    use crate::TransitionStateKey;
    use crate::renderer::RendererAssetResolver;
    use crate::tree::{AutoFit, SpanData};

    #[test]
    fn dash_runs_splits_a_horizontal_line_into_even_dashes() {
        // 100px line, 10 on / 10 off → dashes at 0-10, 20-30, 40-50, 60-70, 80-90.
        let runs = dash_runs(&[(0.0, 0.0), (100.0, 0.0)], 10.0, 10.0);
        assert_eq!(runs.len(), 5);
        assert_eq!(runs[0], vec![(0.0, 0.0), (10.0, 0.0)]);
        assert_eq!(runs[1], vec![(20.0, 0.0), (30.0, 0.0)]);
        assert_eq!(runs[4], vec![(80.0, 0.0), (90.0, 0.0)]);
    }

    #[test]
    fn dash_runs_returns_a_solid_run_for_a_degenerate_pattern() {
        let pts = [(0.0, 0.0), (10.0, 5.0)];
        assert_eq!(dash_runs(&pts, 0.0, 4.0), vec![pts.to_vec()]);
        // A negative gap once walked the arc backwards forever; non-finite
        // params slip past the sum guard too. Both must fold to one solid run.
        assert_eq!(dash_runs(&pts, 10.0, -5.0), vec![pts.to_vec()]);
        assert_eq!(dash_runs(&pts, f32::NAN, 4.0), vec![pts.to_vec()]);
        assert_eq!(dash_runs(&pts, 10.0, f32::INFINITY), vec![pts.to_vec()]);
        // A zero gap left the walk budget at 0, so it advanced by nothing
        // for as long as the segment lasted; it reads as solid anyway.
        assert_eq!(dash_runs(&pts, 5.0, 0.0), vec![pts.to_vec()]);
        // Sub-pixel runs stop moving an f32 walk long before they draw.
        assert_eq!(dash_runs(&pts, 1e-9, 1e-9), vec![pts.to_vec()]);
    }

    #[test]
    fn dash_runs_draws_a_path_too_long_to_dash_as_one_solid_run() {
        // Points are guest data. The walk cannot spin here, but at an 8 px period
        // it would take millions of steps to reach the magnitude where the step
        // stops registering — bounded, and a stall all the same.
        let pts = [(0.0, 0.0), (1e30, 0.0)];
        assert_eq!(dash_runs(&pts, 4.0, 4.0), vec![pts.to_vec()]);
    }

    #[test]
    fn dash_runs_draws_an_unmeasurable_path_as_one_solid_run() {
        // Compared by shape, not value: a run carrying NaN is never `==` itself.
        for far in [f32::NAN, f32::INFINITY] {
            let pts = [(0.0, 0.0), (far, 0.0)];
            let runs = dash_runs(&pts, 4.0, 4.0);
            assert_eq!(runs.len(), 1, "x = {far}: expected one solid run");
            assert_eq!(runs[0].len(), pts.len(), "x = {far}: run kept every point");
        }
    }

    #[test]
    fn dash_runs_flips_where_a_band_boundary_lands_on_a_vertex() {
        // Integer points with an integer dash put the band's end exactly on the
        // vertex at x=2. The flip has to be emitted there, or `budget` leaves
        // the segment at zero and every later segment inherits the wedge.
        let pts = [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (4.0, 0.0)];
        assert_eq!(
            dash_runs(&pts, 2.0, 2.0),
            vec![vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]],
            "dash 0–2 then a gap to the end"
        );
    }

    #[test]
    fn dash_runs_keeps_painting_past_a_vertex_hit_in_the_gap() {
        // The same wedge, landing while the pattern is off.
        // Everything past it went undrawn, `painting` never flipping back.
        let pts = [(0.0, 0.0), (3.0, 0.0), (4.0, 0.0), (8.0, 0.0)];
        assert_eq!(
            dash_runs(&pts, 2.0, 2.0),
            vec![vec![(0.0, 0.0), (2.0, 0.0)], vec![(4.0, 0.0), (6.0, 0.0)],],
            "dash 0–2, gap 2–4, dash 4–6, gap 6–8"
        );
    }

    #[test]
    fn dash_runs_spans_a_corner() {
        // Two 10px legs (horizontal then vertical),
        // dash 15 on / 5 off: the first dash must cross
        // the corner, carrying into the second leg.
        let runs = dash_runs(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 15.0, 5.0);
        assert_eq!(runs[0], vec![(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
    }

    #[test]
    fn qr_renders_a_code_that_scans_back_to_its_text() {
        // Decoded by an independent reader (rqrr), not our own code — so a
        // packing or transpose bug can't quietly roundtrip through itself.
        let text = "https://deck.local/setup?x=42";
        let size = 500.0;
        let mut rr = RecordingRenderer::default();
        render_qr(&mut rr, 0.0, 0.0, size, BLACK, WHITE, 4, text);

        // Replay the recorded rects onto a white canvas.
        let dim = size as u32;
        let mut img = image::GrayImage::from_pixel(dim, dim, image::Luma([255]));
        for &(x, y, w, h, color) in &rr.rects {
            let luma = if color.to_u32() == BLACK.to_u32() {
                0
            } else {
                255
            };
            let x0 = x.round().max(0.0) as u32;
            let y0 = y.round().max(0.0) as u32;
            let x1 = ((x + w).round() as u32).min(dim);
            let y1 = ((y + h).round() as u32).min(dim);
            for py in y0..y1 {
                for px in x0..x1 {
                    img.put_pixel(px, py, image::Luma([luma]));
                }
            }
        }

        let mut prepared = rqrr::PreparedImage::prepare(img);
        let grids = prepared.detect_grids();
        assert_eq!(grids.len(), 1, "exactly one QR grid must be detected");
        let (_meta, decoded) = grids[0]
            .decode()
            .expect("BUG: the rendered grid must decode");
        assert_eq!(decoded, text, "scanned text must equal the encoded input");
    }

    #[derive(Debug)]
    enum RenderEvent {
        Save,
        Restore,
        Translate(f32, f32),
        Rotate(f32),
        Arc {
            cx: f32,
            cy: f32,
            radius: f32,
            start_angle: f32,
            end_angle: f32,
            width: f32,
            fill: ArcFill,
            segments: ArcSegments,
            cap: ArcCap,
        },
        Sphere {
            center_lon: f32,
        },
        CurvedText {
            cx: f32,
            cy: f32,
            radius: f32,
            angle: f32,
            anchor: ArcAnchor,
            facing: ArcTextFacing,
            text: String,
            style: TextStyle,
        },
        AutofitText {
            x: f32,
            y: f32,
            box_width: f32,
            box_height: f32,
            text: String,
            size: u32,
            mode: AutoFit,
            min_size: u16,
            max_size: u16,
        },
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Asset {
        Svg(SvgId),
        Bitmap(BitmapId),
        Mesh(MeshId),
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AssetEvent {
        Resolve(Asset),
        Draw(Asset),
    }

    #[derive(Default)]
    struct RecordingRenderer {
        events: Vec<RenderEvent>,
        rects: Vec<(f32, f32, f32, f32, Color)>,
        asset_events: Rc<RefCell<Vec<AssetEvent>>>,
    }

    impl Renderer for RecordingRenderer {
        fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
            self.rects.push((x, y, w, h, color));
        }

        fn fill_rounded_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _radius: f32,
            _color: Color,
        ) {
        }

        fn stroke_rounded_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _radius: f32,
            _border_width: f32,
            _color: Color,
        ) {
        }

        fn fill_circle(&mut self, _cx: f32, _cy: f32, _r: f32, _color: Color) {}

        fn fill_rect_paint(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _fill: &Fill) {}

        fn fill_circle_paint(&mut self, _cx: f32, _cy: f32, _r: f32, _fill: &Fill) {}

        fn stroke_arc(
            &mut self,
            cx: f32,
            cy: f32,
            radius: f32,
            start_angle: f32,
            end_angle: f32,
            width: f32,
            fill: &ArcFill,
            segments: &ArcSegments,
            cap: ArcCap,
        ) {
            self.events.push(RenderEvent::Arc {
                cx,
                cy,
                radius,
                start_angle,
                end_angle,
                width,
                fill: *fill,
                segments: segments.clone(),
                cap,
            });
        }

        fn stroke_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _border_width: f32,
            _color: Color,
        ) {
        }

        fn draw_line(
            &mut self,
            _x1: f32,
            _y1: f32,
            _x2: f32,
            _y2: f32,
            _width: f32,
            _color: Color,
        ) {
        }

        fn save(&mut self) {
            self.events.push(RenderEvent::Save);
        }

        fn restore(&mut self) {
            self.events.push(RenderEvent::Restore);
        }

        fn translate(&mut self, x: f32, y: f32) {
            self.events.push(RenderEvent::Translate(x, y));
        }

        fn rotate(&mut self, angle_radians: f32) {
            self.events.push(RenderEvent::Rotate(angle_radians));
        }

        fn push_scissor(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}

        fn pop_scissor(&mut self) {}

        fn draw_text(&mut self, _text: &str, _x: f32, _y: f32, _size: f32, _color: Color) {}

        fn measure_text(&mut self, _text: &str, _size: f32) -> f32 {
            0.0
        }

        fn measure_paragraph(
            &mut self,
            _style: &TextStyle,
            _spans: &[SpanData],
            _max_width: Option<f32>,
        ) -> (f32, f32) {
            (0.0, 0.0)
        }

        fn draw_paragraph(
            &mut self,
            _style: &TextStyle,
            _spans: &[SpanData],
            _x: f32,
            _y: f32,
            _max_width: f32,
        ) {
        }

        fn draw_paragraph_clipped(
            &mut self,
            _style: &TextStyle,
            _spans: &[SpanData],
            _x: f32,
            _y: f32,
            _max_width: f32,
            _clip_top: f32,
            _clip_bottom: f32,
        ) {
        }

        fn register_svg(&mut self, _tag: &str, _data: &[u8]) -> Option<SvgId> {
            None
        }

        fn draw_svg(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _color: Color,
            icon_id: SvgId,
            _anti_alias: bool,
            _fills: &[(String, Color)],
        ) {
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Draw(Asset::Svg(icon_id)));
        }

        fn register_bitmap(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
            None
        }

        fn register_bitmap_nearest(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
            None
        }

        fn register_bitmap_rgba(
            &mut self,
            _tag: &str,
            _rgba: &[u8],
            _width: u32,
            _height: u32,
        ) -> Option<BitmapId> {
            None
        }

        fn register_bitmap_rgba_nearest(
            &mut self,
            _tag: &str,
            _rgba: &[u8],
            _width: u32,
            _height: u32,
        ) -> Option<BitmapId> {
            None
        }

        fn draw_bitmap(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, bitmap_id: BitmapId) {
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Draw(Asset::Bitmap(bitmap_id)));
        }

        fn draw_nine_patch(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            bitmap_id: BitmapId,
            _left: u16,
            _top: u16,
            _right: u16,
            _bottom: u16,
        ) {
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Draw(Asset::Bitmap(bitmap_id)));
        }

        fn register_mesh(&mut self, _tag: &str, _data: &[u8]) -> Option<MeshId> {
            None
        }

        fn draw_mesh(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _slot_index: u8,
            mesh_id: MeshId,
            _args: MeshDrawArgs,
        ) {
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Draw(Asset::Mesh(mesh_id)));
        }

        fn draw_sphere(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            bitmap_id: BitmapId,
            _center_lat: f32,
            center_lon: f32,
            _zoom: f32,
            _light_lat: f32,
            _light_lon: f32,
            _atmosphere: bool,
        ) {
            self.events.push(RenderEvent::Sphere { center_lon });
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Draw(Asset::Bitmap(bitmap_id)));
        }

        fn draw_canvas_text(&mut self, _text: &str, _x: f32, _y: f32, _style: &TextStyle) {}

        fn draw_curved_text(
            &mut self,
            cx: f32,
            cy: f32,
            radius: f32,
            angle: f32,
            anchor: ArcAnchor,
            facing: ArcTextFacing,
            text: &str,
            style: &TextStyle,
        ) {
            self.events.push(RenderEvent::CurvedText {
                cx,
                cy,
                radius,
                angle,
                anchor,
                facing,
                text: text.to_owned(),
                style: *style,
            });
        }

        fn draw_autofit_text(
            &mut self,
            x: f32,
            y: f32,
            box_width: f32,
            box_height: f32,
            text: &str,
            style: &TextStyle,
            mode: AutoFit,
            min_size: u16,
            max_size: u16,
        ) {
            self.events.push(RenderEvent::AutofitText {
                x,
                y,
                box_width,
                box_height,
                text: text.to_string(),
                size: style.size,
                mode,
                min_size,
                max_size,
            });
        }

        fn stroke_path(
            &mut self,
            _points: &[(f32, f32)],
            _stroke_width: f32,
            _color: Color,
            _closed: bool,
            _smooth: bool,
        ) {
        }

        fn fill_path_paint(&mut self, _points: &[(f32, f32)], _fill: &Fill, _smooth: bool) {}

        fn drop_shadow(
            &mut self,
            _cx: f32,
            _cy: f32,
            _fbo_w: u32,
            _fbo_h: u32,
            _dx: f32,
            _dy: f32,
            _blur: f32,
            _color: Color,
            inner: &mut dyn FnMut(&mut dyn Renderer),
        ) {
            inner(self);
        }

        fn begin_frame(&mut self, _width: u32, _height: u32, _dpi_scale: f32) {}

        fn flush(&mut self) {}

        fn width(&self) -> f32 {
            0.0
        }

        fn height(&self) -> f32 {
            0.0
        }

        fn evict_prefix(&mut self, _prefix: &str) -> usize {
            0
        }

        fn bitmap_resident_bytes(&self) -> u64 {
            0
        }
    }

    struct RecordingAssetResolver {
        asset_events: Rc<RefCell<Vec<AssetEvent>>>,
    }

    impl RendererAssetResolver for RecordingAssetResolver {
        fn resolve_svg(&mut self, _renderer: &mut dyn Renderer, id: SvgId) -> bool {
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Resolve(Asset::Svg(id)));
            true
        }

        fn resolve_bitmap(&mut self, _renderer: &mut dyn Renderer, id: BitmapId) -> bool {
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Resolve(Asset::Bitmap(id)));
            true
        }

        fn resolve_mesh(&mut self, _renderer: &mut dyn Renderer, id: MeshId) -> bool {
            self.asset_events
                .borrow_mut()
                .push(AssetEvent::Resolve(Asset::Mesh(id)));
            true
        }
    }

    fn test_mesh_draw_args() -> MeshDrawArgs {
        MeshDrawArgs {
            transform: MeshTransform {
                fov: 0.0,
                distance: 0.0,
                quat: [0.0; 4],
                position: [0.0; 3],
                scale: 1.0,
            },
            lighting: MeshLighting {
                pitch: f32::NAN,
                yaw: 0.0,
                ambient: 0.0,
                specular: 0.0,
            },
            highlight: crate::gpu::mesh::MeshHighlight {
                u_min: f32::NAN,
                v_min: 0.0,
                u_max: 0.0,
                v_max: 0.0,
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
        }
    }

    #[test]
    fn render_target_resolves_asset_ids_at_each_draw_including_shadow_content() {
        let svg = SvgId::from_wire(1).expect("BUG: fixture SVG ID must be non-zero");
        let bitmap = BitmapId::from_wire(2).expect("BUG: fixture bitmap ID must be non-zero");
        let mesh = MeshId::from_wire(3).expect("BUG: fixture mesh ID must be non-zero");
        let mut renderer = RecordingRenderer::default();
        let mut resolver = RecordingAssetResolver {
            asset_events: Rc::clone(&renderer.asset_events),
        };
        {
            let resolver = RefCell::new(&mut resolver as &mut dyn RendererAssetResolver);
            let mut target = RenderTarget::new(&mut renderer, Some(&resolver));
            target.drop_shadow(
                0.0,
                0.0,
                1,
                1,
                0.0,
                0.0,
                0.0,
                Color::default(),
                &mut |target| {
                    target.draw_svg(0.0, 0.0, 1.0, 1.0, WHITE, svg, false, &[]);
                },
            );
            target.draw_bitmap(0.0, 0.0, 1.0, 1.0, bitmap);
            target.draw_nine_patch(0.0, 0.0, 1.0, 1.0, bitmap, 0, 0, 0, 0);
            target.draw_sphere(
                0.0,
                0.0,
                1.0,
                1.0,
                bitmap,
                0.0,
                0.0,
                1.0,
                f32::NAN,
                0.0,
                false,
            );
            target.draw_mesh(0.0, 0.0, 1.0, 1.0, 0, mesh, test_mesh_draw_args());
        }

        let expected = [
            AssetEvent::Resolve(Asset::Svg(svg)),
            AssetEvent::Draw(Asset::Svg(svg)),
            AssetEvent::Resolve(Asset::Bitmap(bitmap)),
            AssetEvent::Draw(Asset::Bitmap(bitmap)),
            AssetEvent::Resolve(Asset::Bitmap(bitmap)),
            AssetEvent::Draw(Asset::Bitmap(bitmap)),
            AssetEvent::Resolve(Asset::Bitmap(bitmap)),
            AssetEvent::Draw(Asset::Bitmap(bitmap)),
            AssetEvent::Resolve(Asset::Mesh(mesh)),
            AssetEvent::Draw(Asset::Mesh(mesh)),
        ];
        assert_eq!(*renderer.asset_events.borrow(), expected);
    }

    fn animation_context<'a>(
        animation_states: &'a mut HashMap<u64, AnimationState>,
        transition_states: &'a mut HashMap<TransitionStateKey, TransitionState>,
    ) -> AnimationContext<'a> {
        AnimationContext {
            emit: crate::tree::EmitMode::default(),
            animation_states,
            transition_states,
            delta_ms: 0,
            frame_counter: 0,
            draw_counter: 0,
            canvas_index: 0,
            draw_in_canvas: 0,
            mesh_slot_counter: 0,
            has_active: false,
            now_unix_secs: 0,
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn render_draw_inner_for_test(
        renderer: &mut dyn Renderer,
        draw: &DrawCommand,
        cx: f32,
        cy: f32,
        cw: f32,
        ch: f32,
        offset_x: f32,
        offset_y: f32,
        rotation: f32,
        scale: f32,
        alpha: f32,
        orbit_angle_offset: f32,
        color_override: Option<Color>,
        anim_ctx: &mut AnimationContext<'_>,
    ) {
        let mut target = RenderTarget::new(renderer, None);
        render_draw_inner(
            &mut target,
            draw,
            cx,
            cy,
            cw,
            ch,
            offset_x,
            offset_y,
            rotation,
            scale,
            alpha,
            orbit_angle_offset,
            color_override,
            anim_ctx,
        );
    }

    fn transition_arc(end_angle: f32, duration_ms: u32) -> DrawCommand {
        DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(crate::tree::HostTransitionDef {
                id_hash: 42,
                duration_ms,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(DrawCommand::Arc {
                cx: 20.0,
                cy: 30.0,
                radius: 40.0,
                start_angle: 0.0,
                end_angle,
                width: 6.0,
                fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
                segments: ArcSegments::Continuous,
                cap: ArcCap::Round,
            }),
        }
    }

    fn transition_arc_frame(
        renderer: &mut RecordingRenderer,
        anim_ctx: &mut AnimationContext<'_>,
        delta_ms: u32,
        end_angle: f32,
        duration_ms: u32,
    ) {
        renderer.events.clear();
        anim_ctx.has_active = false;
        anim_ctx.delta_ms = delta_ms;
        anim_ctx.frame_counter += 1;
        render_draw_inner_for_test(
            renderer,
            &transition_arc(end_angle, duration_ms),
            0.0,
            0.0,
            100.0,
            100.0,
            0.0,
            0.0,
            0.0,
            1.0,
            1.0,
            0.0,
            None,
            anim_ctx,
        );
    }

    fn recorded_arc_end_angle(renderer: &RecordingRenderer) -> f32 {
        let [RenderEvent::Arc { end_angle, .. }] = &renderer.events[..] else {
            panic!("BUG: expected one arc draw event");
        };
        *end_angle
    }

    fn transition_sphere(center_lon: f32, duration_ms: u32) -> DrawCommand {
        transition_sphere_with_easing(center_lon, duration_ms, Easing::Linear)
    }

    fn transition_sphere_with_easing(
        center_lon: f32,
        duration_ms: u32,
        easing: Easing,
    ) -> DrawCommand {
        DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(crate::tree::HostTransitionDef {
                id_hash: 43,
                duration_ms,
                easing,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(DrawCommand::Sphere {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
                bitmap_id: BitmapId::from_wire(1),
                atmosphere: true,
                center_lat: 0.0,
                center_lon,
                zoom: 1.8,
                light_lat: 0.0,
                light_lon: 0.0,
            }),
        }
    }

    fn transition_sphere_frame(
        renderer: &mut RecordingRenderer,
        anim_ctx: &mut AnimationContext<'_>,
        delta_ms: u32,
        center_lon: f32,
        duration_ms: u32,
    ) -> f32 {
        renderer.events.clear();
        anim_ctx.has_active = false;
        anim_ctx.delta_ms = delta_ms;
        anim_ctx.frame_counter += 1;
        render_draw_inner_for_test(
            renderer,
            &transition_sphere(center_lon, duration_ms),
            0.0,
            0.0,
            100.0,
            100.0,
            0.0,
            0.0,
            0.0,
            1.0,
            1.0,
            0.0,
            None,
            anim_ctx,
        );
        let [RenderEvent::Sphere { center_lon }] = &renderer.events[..] else {
            panic!("BUG: expected one sphere draw event");
        };
        *center_lon
    }

    #[test]
    fn shortest_degree_delta_handles_accumulated_turns() {
        for (from, to, expected) in [
            (539.98, -179.95, 0.07),
            (-539.98, 179.95, -0.07),
            (540.0, -180.0, 0.0),
            (0.0, 180.0, 180.0),
            (0.0, -180.0, -180.0),
        ] {
            let actual = shortest_angle_delta_deg(from, to);
            assert!(
                (actual - expected).abs() < 0.001,
                "{from}° to {to}° must move {expected}°, got {actual}°"
            );
        }
    }

    #[test]
    fn shortest_radian_delta_handles_accumulated_turns() {
        let pi = std::f32::consts::PI;
        for (from, to, expected) in [
            (3.0 * pi, -pi + 0.05, 0.05),
            (-3.0 * pi, pi - 0.05, -0.05),
            (0.0, pi, pi),
            (0.0, -pi, -pi),
        ] {
            let actual = shortest_angle_delta(from, to);
            assert!(
                (actual - expected).abs() < 0.000_01,
                "{from} rad to {to} rad must move {expected} rad, got {actual} rad"
            );
        }
    }

    #[test]
    fn interpolation_bounds_wrapped_angles_in_the_target_frame() {
        let turns = 20.0 * std::f32::consts::TAU;
        let from = PrevDrawValues {
            angle: turns + 0.1,
            rotation: turns + 0.2,
            arc_start_angle: turns + 0.3,
            center_lon: 7_200.4,
            light_lon: 7_200.5,
            ..PrevDrawValues::default()
        };
        let target = PrevDrawValues {
            angle: 4.0,
            rotation: 4.1,
            arc_start_angle: 4.2,
            center_lon: 200.0,
            light_lon: -200.0,
            ..PrevDrawValues::default()
        };
        let values = interpolate_draw_values(&from, &target, 0.0, ColorSpace::default());

        assert!((values.angle - target.angle).abs() <= std::f32::consts::PI);
        assert!((values.rotation - target.rotation).abs() <= std::f32::consts::PI);
        assert!((values.arc_start_angle - target.arc_start_angle).abs() <= std::f32::consts::PI);
        assert!((values.center_lon - target.center_lon).abs() <= 180.0);
        assert!((values.light_lon - target.light_lon).abs() <= 180.0);

        let settled = interpolate_draw_values(&from, &target, 1.0, ColorSpace::default());
        assert_eq!(settled.angle.to_bits(), target.angle.to_bits());
        assert_eq!(settled.rotation.to_bits(), target.rotation.to_bits());
        assert_eq!(
            settled.arc_start_angle.to_bits(),
            target.arc_start_angle.to_bits()
        );
        assert_eq!(settled.center_lon.to_bits(), target.center_lon.to_bits());
        assert_eq!(settled.light_lon.to_bits(), target.light_lon.to_bits());
    }

    #[test]
    fn repeated_antimeridian_crossings_never_reverse_the_globe() {
        const TARGET_STEP_DEGREES: f32 = 0.065;
        const CACHED_FRAME_MS: u32 = 100;
        const CACHED_FRAMES_PER_TARGET: usize = 10;

        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
        let mut previous = transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 170.0, 1_000);

        for step in 1..=6_000 {
            let target = normalize_angle_deg(170.0 + step as f32 * TARGET_STEP_DEGREES);
            for _ in 0..CACHED_FRAMES_PER_TARGET {
                let rendered = transition_sphere_frame(
                    &mut renderer,
                    &mut anim_ctx,
                    CACHED_FRAME_MS,
                    target,
                    1_000,
                );
                let movement = normalize_angle_deg(rendered - previous).abs();
                assert!(
                    movement <= TARGET_STEP_DEGREES,
                    "target {step} moved globe {movement}° from {previous}° to {rendered}°"
                );
                previous = rendered;
            }
        }
    }

    #[test]
    fn zero_duration_retarget_snaps_and_the_next_transition_starts_there() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 10.0, 1_000);
        let snapped = transition_sphere_frame(&mut renderer, &mut anim_ctx, 30_000, 50.0, 0);

        assert_eq!(snapped.to_bits(), 50.0_f32.to_bits());
        assert!(
            !anim_ctx.has_active,
            "a snap must not request cached frames"
        );

        let transition_start =
            transition_sphere_frame(&mut renderer, &mut anim_ctx, 300, 51.0, 1_000);
        assert_eq!(transition_start.to_bits(), 50.0_f32.to_bits());
        assert!(
            anim_ctx.has_active,
            "the ordinary update after a snap must animate"
        );

        let halfway = transition_sphere_frame(&mut renderer, &mut anim_ctx, 500, 51.0, 1_000);
        assert_eq!(halfway.to_bits(), 50.5_f32.to_bits());
    }

    #[test]
    fn retarget_anchors_against_the_active_transition_duration() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 0.0, 1_000);
        transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 11.0, 1_100);
        let before_retarget =
            transition_sphere_frame(&mut renderer, &mut anim_ctx, 1_000, 11.0, 1_100);
        let retargeted = transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 20.0, 1_000);

        assert_eq!(before_retarget.to_bits(), 10.0_f32.to_bits());
        assert_eq!(retargeted.to_bits(), before_retarget.to_bits());

        let halfway = transition_sphere_frame(&mut renderer, &mut anim_ctx, 500, 20.0, 1_000);
        assert_eq!(halfway.to_bits(), 15.0_f32.to_bits());
    }

    #[test]
    fn retarget_anchors_against_the_active_transition_easing() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        let mut render = |delta_ms, center_lon, easing| {
            renderer.events.clear();
            anim_ctx.has_active = false;
            anim_ctx.delta_ms = delta_ms;
            anim_ctx.frame_counter += 1;
            render_draw_inner_for_test(
                &mut renderer,
                &transition_sphere_with_easing(center_lon, 1_000, easing),
                0.0,
                0.0,
                100.0,
                100.0,
                0.0,
                0.0,
                0.0,
                1.0,
                1.0,
                0.0,
                None,
                &mut anim_ctx,
            );
            let [RenderEvent::Sphere { center_lon }] = &renderer.events[..] else {
                panic!("BUG: expected one sphere draw event");
            };
            *center_lon
        };

        render(0, 0.0, Easing::EaseOut);
        render(0, 10.0, Easing::EaseOut);
        let before_retarget = render(500, 10.0, Easing::EaseOut);
        let retargeted = render(0, 20.0, Easing::Linear);

        assert_eq!(before_retarget.to_bits(), 7.5_f32.to_bits());
        assert_eq!(retargeted.to_bits(), before_retarget.to_bits());
    }

    /// A drag following a finger retargets every frame, and the clock has to
    /// advance on those frames too: re-anchoring `from` at `t = 0` each time
    /// pins the draw to its first value for the whole interaction.
    #[test]
    fn a_transition_retargeted_every_frame_still_advances() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        // Settles the state at 1.0, the way a first frame does.
        transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);

        let mut rendered = Vec::new();
        for step in 1..=6_u8 {
            let target = 1.0 + f32::from(step);
            transition_arc_frame(&mut renderer, &mut anim_ctx, 100, target, 1_000);
            rendered.push(recorded_arc_end_angle(&renderer));
        }

        assert!(
            rendered.windows(2).all(|w| w[1] > w[0]),
            "each frame must land past the one before it, got {rendered:?}"
        );
        assert!(
            rendered.last().is_some_and(|last| *last > 1.5),
            "six frames of chasing a rising target must leave the start well \
             behind, got {rendered:?}"
        );
    }

    /// The advance belongs *before* the retarget, not after it. A frame slower
    /// than the whole duration must still start the new transition at its old
    /// position. Charged afterwards, the clock lands past the duration on the
    /// retarget frame itself, and the new transition is skipped entirely.
    #[test]
    fn a_frame_slower_than_the_duration_still_starts_the_new_transition() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);
        // 1500 ms against the arc's 1000 ms duration.
        transition_arc_frame(&mut renderer, &mut anim_ctx, 1500, 3.0, 1_000);

        assert_eq!(
            recorded_arc_end_angle(&renderer).to_bits(),
            1.0_f32.to_bits(),
            "the new transition starts where the old one ended, not at its target"
        );
    }

    #[test]
    fn arc_transition_interpolates_sweep() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);
        transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 3.0, 1_000);
        transition_arc_frame(&mut renderer, &mut anim_ctx, 500, 3.0, 1_000);

        let [
            RenderEvent::Arc {
                cx,
                cy,
                radius,
                start_angle: 0.0,
                end_angle,
                width,
                fill,
                segments,
                cap,
            },
        ] = &renderer.events[..]
        else {
            panic!("BUG: expected one arc draw event");
        };
        assert_eq!(cx.to_bits(), 20.0_f32.to_bits());
        assert_eq!(cy.to_bits(), 30.0_f32.to_bits());
        assert_eq!(radius.to_bits(), 40.0_f32.to_bits());
        assert_eq!(end_angle.to_bits(), 2.0_f32.to_bits());
        assert_eq!(width.to_bits(), 6.0_f32.to_bits());
        assert_eq!(*fill, ArcFill::Solid(Color::from_rgb(1, 2, 3)));
        assert_eq!(*segments, ArcSegments::Continuous);
        assert_eq!(*cap, ArcCap::Round);
    }

    #[test]
    fn transition_start_frame_does_not_consume_the_gap_delta() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);

        // A widget rendering at 1 Hz: the whole 1000 ms gap elapsed before the
        // new target appeared, so the 1000 ms transition starts here rather
        // than ending here.
        transition_arc_frame(&mut renderer, &mut anim_ctx, 1_000, 3.0, 1_000);
        assert_eq!(
            recorded_arc_end_angle(&renderer).to_bits(),
            1.0_f32.to_bits()
        );
        assert!(
            anim_ctx.has_active,
            "an unfinished transition must keep asking for frames"
        );
    }

    #[test]
    fn zero_duration_transition_stays_at_target_when_animation_resumes() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);
        transition_arc_frame(&mut renderer, &mut anim_ctx, 2_001, 3.0, 0);
        assert_eq!(
            recorded_arc_end_angle(&renderer).to_bits(),
            3.0_f32.to_bits()
        );
        assert!(
            !anim_ctx.has_active,
            "a snapped update needs no animation frames"
        );

        transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 3.0, 1_000);
        assert_eq!(
            recorded_arc_end_angle(&renderer).to_bits(),
            3.0_f32.to_bits(),
            "an immediate refresh must not move back toward the pre-gap position"
        );

        assert!(
            !anim_ctx.has_active,
            "a settled snap must not schedule redundant animation frames"
        );

        transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 5.0, 1_000);
        assert_eq!(
            recorded_arc_end_angle(&renderer).to_bits(),
            3.0_f32.to_bits(),
            "the next animation must start at the snapped position"
        );
        transition_arc_frame(&mut renderer, &mut anim_ctx, 500, 5.0, 1_000);
        assert_eq!(
            recorded_arc_end_angle(&renderer).to_bits(),
            4.0_f32.to_bits()
        );
        assert!(anim_ctx.has_active, "normal updates must animate again");
    }

    #[test]
    fn first_frame_snap_does_not_replay_frames_once_a_duration_returns() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        // A slot's first frame carries a zero delta
        // even when the widget saw a long gap and asked for a snap.
        transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 3.0, 0);
        assert!(!anim_ctx.has_active);

        transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 3.0, 500);
        assert_eq!(
            recorded_arc_end_angle(&renderer).to_bits(),
            3.0_f32.to_bits()
        );
        assert!(
            !anim_ctx.has_active,
            "a snap with no elapsed time must still count as finished"
        );
    }

    #[test]
    fn curved_text_dispatches_scaled_style_and_color_override() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
        let draw = DrawCommand::CurvedText {
            cx: 5.0,
            cy: 6.0,
            radius: 7.0,
            angle: 0.25,
            anchor: ArcAnchor::Center,
            facing: ArcTextFacing::Inward,
            text: "hashrate".to_owned(),
            style: TextStyle {
                size: 12,
                color: Color::from_rgb(1, 2, 3),
                ..Default::default()
            },
        };

        render_draw_inner_for_test(
            &mut renderer,
            &draw,
            10.0,
            20.0,
            100.0,
            80.0,
            3.0,
            4.0,
            0.0,
            2.0,
            0.5,
            0.0,
            Some(Color::from_rgba(10, 20, 30, 128)),
            &mut anim_ctx,
        );

        let expected_style = TextStyle {
            size: 24,
            color: Color::from_rgba(10, 20, 30, 64),
            ..Default::default()
        };
        let [
            RenderEvent::CurvedText {
                cx: 18.0,
                cy: 30.0,
                radius: 14.0,
                angle: 0.25,
                anchor: ArcAnchor::Center,
                facing: ArcTextFacing::Inward,
                text,
                style,
            },
        ] = &renderer.events[..]
        else {
            panic!("expected one curved text draw event");
        };
        assert_eq!(text, "hashrate");
        assert_eq!(style.size, expected_style.size);
        assert_eq!(style.color, expected_style.color);
    }

    #[test]
    fn curved_text_dispatches_inside_outer_rotation() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
        let draw = DrawCommand::CurvedText {
            cx: 5.0,
            cy: 6.0,
            radius: 7.0,
            angle: 0.25,
            anchor: ArcAnchor::Center,
            facing: ArcTextFacing::Outward,
            text: "hashrate".to_owned(),
            style: TextStyle {
                size: 12,
                color: Color::from_rgb(1, 2, 3),
                ..Default::default()
            },
        };

        render_draw_inner_for_test(
            &mut renderer,
            &draw,
            10.0,
            20.0,
            100.0,
            80.0,
            3.0,
            4.0,
            0.75,
            2.0,
            1.0,
            0.0,
            None,
            &mut anim_ctx,
        );

        let expected_style = TextStyle {
            size: 24,
            color: Color::from_rgb(1, 2, 3),
            ..Default::default()
        };
        let [
            RenderEvent::Save,
            RenderEvent::Translate(60.0, 60.0),
            RenderEvent::Rotate(0.75),
            RenderEvent::CurvedText {
                cx: -42.0,
                cy: -30.0,
                radius: 14.0,
                angle: 0.25,
                anchor: ArcAnchor::Center,
                facing: ArcTextFacing::Outward,
                text,
                style,
            },
            RenderEvent::Restore,
        ] = &renderer.events[..]
        else {
            panic!("expected rotated curved text draw event");
        };
        assert_eq!(text, "hashrate");
        assert_eq!(style.size, expected_style.size);
        assert_eq!(style.color, expected_style.color);
    }

    #[test]
    fn autofit_text_dispatches_to_renderer() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
        let draw = DrawCommand::AutofitText {
            x: 5.0,
            y: 7.0,
            box_width: 100.0,
            box_height: 40.0,
            mode: AutoFit::Shrink,
            min_size: 16,
            max_size: 0,
            text: "hi".to_string(),
            style: TextStyle {
                size: 32,
                ..TextStyle::default()
            },
        };

        render_draw_inner_for_test(
            &mut renderer,
            &draw,
            0.0,
            0.0,
            200.0,
            100.0,
            0.0,
            0.0,
            0.0,
            1.0,
            1.0,
            0.0,
            None,
            &mut anim_ctx,
        );

        let [
            RenderEvent::AutofitText {
                x,
                y,
                box_width,
                box_height,
                text,
                size,
                mode,
                min_size,
                max_size,
            },
        ] = &renderer.events[..]
        else {
            panic!(
                "expected exactly one AutofitText render event, got: {:?}",
                renderer.events
            );
        };
        assert!((*x - 5.0).abs() < f32::EPSILON);
        assert!((*y - 7.0).abs() < f32::EPSILON);
        assert!((*box_width - 100.0).abs() < f32::EPSILON);
        assert!((*box_height - 40.0).abs() < f32::EPSILON);
        assert_eq!(text, "hi");
        assert_eq!(*size, 32);
        assert_eq!(*mode, AutoFit::Shrink);
        assert_eq!(*min_size, 16);
        assert_eq!(*max_size, 0);
    }
}
