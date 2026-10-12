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

use bmc_wasm_protocol::{
    AnimProperty, ArcFill, ArcSegments, Color, Dash, Fill, PathPaint, TRANSPARENT,
};

use crate::animation::{apply_easing, compute_animation_value, interpolate_color};
use crate::gpu::mesh::{MeshDrawArgs, MeshLighting, MeshTransform};
use crate::renderer::{RenderTarget, Renderer};
use crate::tree::{AnimationContext, DrawCommand, HostAnimationDef};
use crate::{AnimationState, TransitionState};

mod transition;

use transition::interpolate_draw_values;
pub(crate) use transition::{extract_draw_values, shortest_angle_delta};

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
        renderer,
        draw,
        cx,
        cy,
        cw,
        ch,
        DrawOffset::default(),
        0.0,
        1.0,
        1.0,
        0.0,
        None,
        anim_ctx,
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

/// The two offsets a draw accumulates on its way down the tree, kept apart.
///
/// Folded into one number they spoil each other: a `Centered` under an `Orbit`
/// would re-centre over its parent's placement, and an outer translate would
/// die at the `Centered` below it.
#[derive(Clone, Copy, Default)]
struct DrawOffset {
    layout_x: f32,
    layout_y: f32,
    translate_x: f32,
    translate_y: f32,
}

impl DrawOffset {
    fn x(self) -> f32 {
        self.layout_x + self.translate_x
    }

    fn y(self) -> f32 {
        self.layout_y + self.translate_y
    }

    fn with_layout(self, layout_x: f32, layout_y: f32) -> Self {
        Self {
            layout_x,
            layout_y,
            ..self
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
    offset: DrawOffset,
    rotation: f32,
    scale: f32,
    alpha: f32,
    orbit_angle_offset: f32,
    color_override: Option<Color>,
    anim_ctx: &mut AnimationContext<'_>,
) {
    let (offset_x, offset_y) = (offset.x(), offset.y());
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
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                offset.with_layout((cw - iw) / 2.0, (ch - ih) / 2.0),
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
            let (iw, ih) = get_draw_bounds(inner);
            let layout_x = cw / 2.0 + radius * effective_angle.cos() - iw / 2.0;
            let layout_y = ch / 2.0 + radius * effective_angle.sin() - ih / 2.0;
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                offset.with_layout(layout_x, layout_y),
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
                offset,
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
                    offset,
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
            let mut acc_offset = offset;
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
                    AnimProperty::TranslateX => acc_offset.translate_x += value,
                    AnimProperty::TranslateY => acc_offset.translate_y += value,
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
                        acc_offset.translate_x += interp.x - current_values.x;
                        acc_offset.translate_y += interp.y - current_values.y;
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
                    acc_offset,
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
                    acc_offset,
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
                    acc_offset,
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
                    acc_offset,
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

#[cfg(test)]
mod tests;
