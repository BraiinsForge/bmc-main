// Copyright (C) 2026  Braiins Systems s.r.o.

//! Canvas draw command rendering.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]
#![allow(clippy::wildcard_imports)]

use glam::{Quat, Vec3};

use bmc_wasm_protocol::*;

use crate::animation::{apply_easing, compute_animation_value, interpolate_color};
use crate::gpu::mesh::{MeshDrawArgs, MeshLighting, MeshTransform};
use crate::renderer::Renderer;
use crate::tree::{AnimationContext, DrawCommand, HostAnimationDef};
use crate::{AnimationState, PrevDrawValues, TransitionState};

/// Interpolated mesh parameters for transition override. Wraps the same
/// `MeshDrawArgs` shape rendered later, so override application is a struct
/// copy instead of a 21-field shuffle.
#[derive(Debug, Clone, Copy)]
struct MeshOverride {
    args: MeshDrawArgs,
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
    let base = match color_override {
        Some(c) => ArcFill::Solid(c),
        None => *fill,
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

pub(crate) fn render_draw_command(
    renderer: &mut dyn Renderer,
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

/// Render a draw command with accumulated transforms and animation modifiers.
#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_draw_inner(
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
        } => {
            let er = *radius * scale;
            let ew = *width * scale;
            let scx = cx + *arc_cx + offset_x;
            let scy = cy + *arc_cy + offset_y;
            let eff = effective_arc_fill(fill, color_override, alpha);
            if rotation == 0.0 {
                renderer.stroke_arc(scx, scy, er, *start_angle, *end_angle, ew, &eff, segments);
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
                        elapsed_ms: trans_def.duration_ms, // start finished
                        last_seen_frame: anim_ctx.frame_counter,
                    }
                });
                state.last_seen_frame = anim_ctx.frame_counter;

                // Detect target change
                if state.target != current_values {
                    // D3-style: interpolate from current interpolated position
                    let t = if trans_def.duration_ms > 0 {
                        (state.elapsed_ms as f32 / trans_def.duration_ms as f32).min(1.0)
                    } else {
                        1.0
                    };
                    let eased_t = apply_easing(trans_def.easing, t);
                    state.from =
                        interpolate_draw_values(&state.from, &state.target, eased_t, *color_space);
                    state.target = current_values;
                    state.elapsed_ms = 0;
                }

                state.elapsed_ms = state.elapsed_ms.saturating_add(anim_ctx.delta_ms);

                if state.elapsed_ms < trans_def.duration_ms {
                    anim_ctx.has_active = true;
                    let t = state.elapsed_ms as f32 / trans_def.duration_ms as f32;
                    let eased_t = apply_easing(trans_def.easing, t);
                    let interp =
                        interpolate_draw_values(&state.from, &state.target, eased_t, *color_space);
                    // Apply interpolated overrides to accumulated state
                    acc_offset_x += interp.x - current_values.x;
                    acc_offset_y += interp.y - current_values.y;
                    acc_scale *= if current_values.w > 0.0 {
                        interp.w / current_values.w
                    } else {
                        1.0
                    };
                    acc_orbit_angle += interp.angle - current_values.angle;
                    acc_rotation += interp.rotation - current_values.rotation;
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
                PathPaint::Stroke { color, width } => {
                    renderer.stroke_path(
                        pts,
                        *width * scale,
                        stroke_color(*color, color_override, alpha),
                        *closed,
                        *smooth,
                    );
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
fn extract_draw_values(draw: &DrawCommand) -> PrevDrawValues {
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
            fill,
            ..
        } => PrevDrawValues {
            x: *cx,
            y: *cy,
            w: *radius,
            color: fill.primary_color(),
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
    }
}

/// Shortest-path delta for angle interpolation (wraps around TAU).
fn shortest_angle_delta(from: f32, to: f32) -> f32 {
    let mut d = to - from;
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    if d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

/// Shortest-path delta for degrees (wraps around 360°).
fn shortest_angle_delta_deg(from: f32, to: f32) -> f32 {
    let mut d = to - from;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
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
        angle: a.angle + shortest_angle_delta(a.angle, b.angle) * t,
        radius: a.radius + (b.radius - a.radius) * t,
        rotation: a.rotation + shortest_angle_delta(a.rotation, b.rotation) * t,
        center_lat: a.center_lat + (b.center_lat - a.center_lat) * t,
        center_lon: a.center_lon + shortest_angle_delta_deg(a.center_lon, b.center_lon) * t,
        zoom: a.zoom + (b.zoom - a.zoom) * t,
        light_lat: a.light_lat + (b.light_lat - a.light_lat) * t,
        light_lon: a.light_lon + shortest_angle_delta_deg(a.light_lon, b.light_lon) * t,
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
    use std::collections::HashMap;

    use super::*;
    use crate::TransitionStateKey;
    use crate::tree::SpanData;

    #[derive(Debug)]
    enum RenderEvent {
        Save,
        Restore,
        Translate(f32, f32),
        Rotate(f32),
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
    }

    #[derive(Default)]
    struct RecordingRenderer {
        events: Vec<RenderEvent>,
    }

    impl Renderer for RecordingRenderer {
        fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _color: Color) {}

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

        fn fill_circle(&mut self, _cx: f32, _cy: f32, _r: f32, _color: Color) {}

        fn fill_rect_paint(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _fill: &Fill) {}

        fn fill_circle_paint(&mut self, _cx: f32, _cy: f32, _r: f32, _fill: &Fill) {}

        fn stroke_arc(
            &mut self,
            _cx: f32,
            _cy: f32,
            _radius: f32,
            _start_angle: f32,
            _end_angle: f32,
            _width: f32,
            _fill: &ArcFill,
            _segments: &ArcSegments,
        ) {
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
            _icon_id: SvgId,
            _anti_alias: bool,
            _fills: &[(String, Color)],
        ) {
        }

        fn register_bitmap(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
            None
        }

        fn register_bitmap_nearest(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
            None
        }

        fn draw_bitmap(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _bitmap_id: BitmapId) {}

        fn draw_nine_patch(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _bitmap_id: BitmapId,
            _left: u16,
            _top: u16,
            _right: u16,
            _bottom: u16,
        ) {
        }

        fn bitmap_sample(
            &self,
            _bitmap_id: BitmapId,
            _x: u32,
            _y: u32,
            _w: u32,
            _h: u32,
        ) -> Option<Color> {
            None
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
            _mesh_id: MeshId,
            _args: MeshDrawArgs,
        ) {
        }

        fn draw_sphere(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _bitmap_id: BitmapId,
            _center_lat: f32,
            _center_lon: f32,
            _zoom: f32,
            _light_lat: f32,
            _light_lon: f32,
            _atmosphere: bool,
        ) {
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
    }

    fn animation_context<'a>(
        animation_states: &'a mut HashMap<u64, AnimationState>,
        transition_states: &'a mut HashMap<TransitionStateKey, TransitionState>,
    ) -> AnimationContext<'a> {
        AnimationContext {
            animation_states,
            transition_states,
            delta_ms: 0,
            frame_counter: 0,
            draw_counter: 0,
            canvas_index: 0,
            draw_in_canvas: 0,
            mesh_slot_counter: 0,
            has_active: false,
        }
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

        render_draw_inner(
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

        render_draw_inner(
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
}
