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

//! Painting laid-out taffy nodes through a [`Renderer`].

#![expect(clippy::cast_lossless)]

use super::DEBUG_LAYOUT;
use super::layout::NodeContext;
use super::layout::ParagraphData;
use super::walk::AnimationContext;
use super::walk::EmitMode;
use super::walk::TouchHit;
use super::walk::TreeResult;
use crate::ScrollState;
use crate::components::draw::render_draw_command;
use crate::components::notification::render_notification;
use crate::components::progress_bar::render_progress_bar;
use crate::components::skeleton::render_skeleton;
use crate::components::switcher::{register_switcher_hits, render_switcher};
use crate::components::tag::render_tag;
use crate::components::{ButtonSize, ButtonStyle, draw_button_with_target};
use crate::interaction::InteractionState;
use crate::interaction::Rect;
use crate::partition::Band;
use crate::renderer::{RenderTarget, Renderer};
use bmc_wasm_protocol::{Color, GRAY_50, GRAY_60, TextOverflow};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use taffy::prelude::*;

/// Hot-pink-ish colors cycling by depth for debug outlines.
const DEBUG_COLORS: [Color; 6] = [
    Color::from_hex(0xFF_00_FF), // magenta
    Color::from_hex(0x00_FF_FF), // cyan
    Color::from_hex(0xFF_FF_00), // yellow
    Color::from_hex(0xFF_00_00), // red
    Color::from_hex(0x00_FF_00), // green
    Color::from_hex(0xFF_80_00), // orange
];

/// Whether this node paints anything inside its own box, canvas draws aside.
///
/// The box decoration plus every host-rendered component.
/// Canvas draws are left out: they band and damage one by one,
/// which bounds them far more tightly than the node's box.
fn paints_own_box(ctx: &NodeContext) -> bool {
    crate::partition::decoration_paints(
        ctx.bg_nine_patch.bitmap_id,
        ctx.background,
        ctx.border_width,
    ) || ctx.tag.is_some()
        || ctx.paragraph.is_some()
        || ctx.button.is_some()
        || ctx.progress_bar.is_some()
        || ctx.notification.is_some()
        || ctx.switcher.is_some()
        || ctx.skeleton.is_some()
}

/// Paint `node_id` and its subtree at its [`super::TreeNode::Dimmed`] factor, if any,
/// restoring the renderer's brightness after.
#[expect(clippy::too_many_arguments)]
pub(crate) fn render_taffy_node(
    taffy: &TaffyTree<NodeContext>,
    node_id: taffy::NodeId,
    parent_x: f32,
    parent_y: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    interaction: &mut InteractionState,
    scroll_states: &mut HashMap<String, ScrollState>,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
    depth: usize,
) {
    let dim = taffy.get_node_context(node_id).and_then(|ctx| ctx.dim);
    let outer = renderer.brightness();
    if let Some(dim) = dim {
        renderer.set_brightness(outer * dim);
    }
    render_taffy_node_body(
        taffy,
        node_id,
        parent_x,
        parent_y,
        renderer,
        interaction,
        scroll_states,
        result,
        anim_ctx,
        depth,
    );
    if dim.is_some() {
        renderer.set_brightness(outer);
    }
}

#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_taffy_node_body(
    taffy: &TaffyTree<NodeContext>,
    node_id: taffy::NodeId,
    parent_x: f32,
    parent_y: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    interaction: &mut InteractionState,
    scroll_states: &mut HashMap<String, ScrollState>,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
    depth: usize,
) {
    let Ok(layout) = taffy.layout(node_id) else {
        return;
    };
    let x = parent_x + layout.location.x;
    let y = parent_y + layout.location.y;
    let w = layout.size.width;
    let h = layout.size.height;

    // Extract IDs before immutable context borrow
    let scroll_key = taffy
        .get_node_context(node_id)
        .and_then(|ctx| ctx.scroll_key.clone());
    let touch_key = taffy
        .get_node_context(node_id)
        .and_then(|ctx| ctx.touch_key.clone());

    // Traverse everything, always: emission is what costs, and skipping a
    // subtree would mean restoring the positional counters its walk would have
    // advanced and replaying the hit regions it never registered.
    //
    // The gate is per node, not per subtree: a dynamic subtree still holds
    // static nodes a dynamic-only pass must not repaint, and vice versa. An
    // untagged node falls through to `true` and draws in every mode rather
    // than vanishing.
    //
    // The gate reads a band rather than the classification alone.
    // The walk runs in paint order, so a static node landing on pixels
    // the dynamic half has already painted repaints with that half
    // rather than sitting in the layer beneath them.
    //
    // Bands read the laid-out box, not the ink:
    // a paragraph set tighter than its font's natural line height
    // paints a few rows outside it (BDK-932).
    let node_rect = Rect::new(x, y, w, h);
    let band = taffy.get_node_context(node_id).map(|ctx| {
        if ctx.self_dynamic {
            Band::Dynamic
        } else if anim_ctx.dynamic_half.covers(node_rect) {
            Band::Above
        } else {
            Band::Below
        }
    });
    let emits = band.is_none_or(|band| anim_ctx.emit.emits_band(band));

    // The index this node's canvas draws key their transition state on, read
    // before the walk below advances it.
    let canvas_index = anim_ctx.canvas_index;

    if anim_ctx.emit == EmitMode::StaticOnly
        && emits
        && taffy.get_node_context(node_id).is_some_and(paints_own_box)
    {
        anim_ctx.layer_painted = true;
    }

    if let Some(ctx) = taffy.get_node_context(node_id) {
        if !emits {
            // Fall through to children: a static container can hold a dynamic
            // descendant, and vice versa.
        } else if let Some(bitmap_id) = ctx.bg_nine_patch.bitmap_id {
            let np = &ctx.bg_nine_patch;
            renderer.draw_nine_patch(x, y, w, h, bitmap_id, np.left, np.top, np.right, np.bottom);
        } else {
            if ctx.background != Color::default() {
                if ctx.border_radius > 0.0 {
                    renderer.fill_rounded_rect(x, y, w, h, ctx.border_radius, ctx.background);
                } else {
                    renderer.fill_rect(x, y, w, h, ctx.background);
                }
            }
            // The border strokes inside the box, over the background's edge,
            // as CSS paints a border-box border.
            if ctx.border_width > 0.0 {
                let inset = ctx.border_width / 2.0;
                renderer.stroke_rounded_rect(
                    x + inset,
                    y + inset,
                    w - ctx.border_width,
                    h - ctx.border_width,
                    (ctx.border_radius - inset).max(0.0),
                    ctx.border_width,
                    ctx.border_color,
                );
            }
        }

        // Tag pill + leading icon, painted behind the content child.
        if emits && let Some(ref tag) = ctx.tag {
            render_tag(tag, x, y, w, h, renderer);
        }

        if emits && let Some(ref para) = ctx.paragraph {
            draw_paragraph_node(renderer, para, x, y, w);
        }

        // Registration is unconditional, only the painting is gated: a button
        // whose half this pass skipped still has to claim its hit region and
        // consume a pending click, or it goes dead.
        if let Some(ref btn) = ctx.button {
            let (clicked, click_pos) = if emits {
                draw_button_with_target(
                    renderer,
                    interaction,
                    &btn.id,
                    &btn.label,
                    x,
                    y,
                    w,
                    h,
                    ButtonStyle::from(btn.style as u32),
                    ButtonSize::from(btn.size),
                    btn.icon_id,
                    btn.disabled,
                    btn.skin.as_ref(),
                )
            } else if btn.disabled {
                (false, None)
            } else {
                interaction.button_with_pos(&btn.id, crate::interaction::Rect::new(x, y, w, h))
            };
            if clicked {
                result.clicks.insert(
                    btn.id.clone(),
                    TouchHit {
                        x: click_pos.map_or(0.0, |p| p.0),
                        y: click_pos.map_or(0.0, |p| p.1),
                        width: w,
                        height: h,
                    },
                );
            }
        }

        // Render canvas draw commands with local coordinates
        if !ctx.draws.is_empty() {
            renderer.push_scissor(x, y, w, h);
            anim_ctx.draw_in_canvas = 0;
            // Split per draw: a canvas is tagged dynamic when any single draw
            // is, so the node-level gate is too coarse here. Skipping a
            // non-`Modified` draw cannot desynchronise `draw_counter`, which
            // only advances inside the `Modified` arm.
            let bands = crate::partition::canvas_bands_after(&ctx.draws, band == Some(Band::Above));
            for (draw, band) in ctx.draws.iter().zip(bands) {
                if anim_ctx.emit.emits_band(band) {
                    anim_ctx.layer_painted |= anim_ctx.emit == EmitMode::StaticOnly;
                    render_draw_command(renderer, draw, x, y, w, h, anim_ctx);
                }
                anim_ctx.draw_in_canvas += 1;
            }
            anim_ctx.canvas_index += 1;
            renderer.pop_scissor();
        }

        // Progress bar: host-rendered track + fill + squiggle + dot
        if emits && let Some(ref pb) = ctx.progress_bar {
            renderer.push_scissor(x, y, w, h);
            let has_active = render_progress_bar(renderer, pb, x, y, w, h, anim_ctx);
            if has_active {
                anim_ctx.has_active = true;
            }
            renderer.pop_scissor();
        }
    }

    // Whatever this node paints outside the layer joins the dynamic half,
    // so the nodes after it band against it.
    // Its own box bounds that: a canvas's draws are scissored to it,
    // and a container's background covers the children that follow.
    if let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(band) = band
        && match band {
            Band::Dynamic => true,
            Band::Above => paints_own_box(ctx) || !ctx.draws.is_empty(),
            // A canvas of its own, holding at least one animating draw.
            Band::Below => ctx.dynamic && !ctx.draws.is_empty(),
        }
    {
        anim_ctx.dynamic_half.add(node_rect);
    }

    // Record what a damage-tracked frame would have to repaint.
    // Dynamic nodes count at the leaves: `dynamic` on a container merely
    // says some descendant animates, so counting containers would union
    // the whole subtree and report the root as damaged.
    // A container painting itself `Above` is the one exception,
    // and it owes only its own box.
    //
    // After the draws above, not before: that pass advances each transition's
    // recorded span, and a rect taken beforehand describes the span the
    // previous walk interpolated, so a target the guest just moved sweeps
    // outside it.
    if let Some(ctx) = taffy.get_node_context(node_id)
        && (ctx.dynamic || band == Some(Band::Above))
    {
        // An `Above` node is static but paints with the dynamic half,
        // so it owes a rect for the same reason a dynamic one does:
        // outside the damage set the scissor drops the repaint,
        // and the layer blitted underneath holds no copy of it.
        // Containers too — a background painted `Above` covers what the layer
        // supplied beneath it.
        let mut damaged = (band == Some(Band::Above) && paints_own_box(ctx)).then_some(node_rect);
        if taffy.child_count(node_id) == 0 {
            // A canvas is one leaf holding many draws,
            // so its own rect is far coarser than what the dynamic pass repaints:
            // `crate::damage` bounds each draw,
            // and only one it cannot bound falls back to the whole canvas.
            // A dynamic leaf with no draws is host-driven and repaints entirely.
            if ctx.draws.is_empty() && ctx.dynamic {
                damaged = Some(node_rect);
            }
            let bands = crate::partition::canvas_bands_after(&ctx.draws, band == Some(Band::Above));
            for (draw, _) in ctx
                .draws
                .iter()
                .zip(bands)
                .filter(|(_, band)| !band.is_in_layer())
            {
                match crate::damage::canvas_draw_damage(
                    draw,
                    node_rect,
                    anim_ctx.transition_states,
                    canvas_index,
                ) {
                    crate::damage::DrawDamage::Region(rect) => {
                        Rect::union_bounds(&mut damaged, rect);
                    }
                    crate::damage::DrawDamage::Nothing => {}
                    crate::damage::DrawDamage::WholeCanvas => {
                        damaged = Some(node_rect);
                        break;
                    }
                }
            }
        }
        if let Some(damaged) = damaged {
            Rect::union_bounds(&mut result.dynamic_bounds, damaged);
            result.dynamic_rects.push(damaged);
            result.dynamic_node_count += 1;
        }
    }

    // Canvas touch hit-testing (needs mutable interaction, outside the immutable ctx borrow)
    if let Some(ref tk) = touch_key {
        let bounds = crate::interaction::Rect::new(x, y, w, h);
        let (clicked, click_pos) = interaction.button_with_pos(tk, bounds);
        if clicked && let Some((lx, ly)) = click_pos {
            result.clicks.insert(
                tk.clone(),
                TouchHit {
                    x: lx,
                    y: ly,
                    width: w,
                    height: h,
                },
            );
        }
        // Active drag: finger is down on this element
        if let Some((lx, ly)) = interaction.get_drag_pos(tk, bounds) {
            result.drags.insert(
                tk.clone(),
                TouchHit {
                    x: lx,
                    y: ly,
                    width: w,
                    height: h,
                },
            );
        }
    }

    // Gated like every other paint site: ungated, a static draw accumulates a
    // copy per frame, since nothing repaints underneath it.
    if emits
        && let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(ref notif) = ctx.notification
    {
        render_notification(notif, x, y, w, h, renderer);
    }

    if let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(ref sw) = ctx.switcher
    {
        if emits {
            render_switcher(sw, x, y, w, h, renderer);
        }
        // Ungated, like the canvas hit-testing below: a pass that paints
        // nothing still owns its share of the frame's hit regions.
        register_switcher_hits(sw, x, y, w, h, interaction, result);
    }

    if emits
        && let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(ref sk) = ctx.skeleton
    {
        render_skeleton(&mut *renderer, sk, x, y, w, h);
    }

    let Ok(children) = taffy.children(node_id) else {
        return;
    };

    // Scroll container: scissor-clip + offset children by scroll amount
    if let Some(ref sk) = scroll_key {
        // Compute content height from children's layouts
        let content_height = children
            .iter()
            .filter_map(|&cid| taffy.layout(cid).ok())
            .map(|cl| cl.location.y + cl.size.height)
            .fold(0.0_f32, f32::max);

        let max_scroll = (content_height - h).max(0.0);
        let has_scrollbar = content_height > h;

        // Scrollbar hit region (overlaid, right edge of viewport)
        let sbar_width_normal = 4.0_f32;
        let sbar_width_active = 12.0_f32;
        let sbar_margin = 2.0_f32;
        // Hit region is generous (active width) so it's easy to grab
        let sbar_hit_w = sbar_width_active + sbar_margin;
        let sbar_key = format!("{sk}::sbar");
        let sbar_pressed = if has_scrollbar {
            let sbar_hit_rect = crate::interaction::Rect::new(x + w - sbar_hit_w, y, sbar_hit_w, h);
            interaction.button(&sbar_key, sbar_hit_rect);
            interaction.is_pressed(&sbar_key)
        } else {
            false
        };

        // Register content hit region for drag/wheel scrolling
        let scroll_region = crate::interaction::Rect::new(x, y, w, h);
        interaction.button(sk, scroll_region);

        // Read scroll delta — scrollbar drag scales by content/viewport ratio
        let scroll_delta = if sbar_pressed {
            let ratio = if h > 0.0 { content_height / h } else { 1.0 };
            interaction.get_scroll_delta(&sbar_key) * ratio
        } else if interaction.is_pressed(sk) {
            -interaction.get_scroll_delta(sk)
        } else {
            interaction.get_scroll_delta_in(&scroll_region)
        };
        // Once per container per frame: the offset advances inside the walk,
        // and a capture frame walks the tree twice before the modal overlays
        // are walked a third time.
        let scroll_delta = interaction.take_scroll_delta(sk, scroll_delta);

        let state = scroll_states.entry(sk.clone()).or_default();
        state.scroll_offset += scroll_delta;
        state.scroll_offset = state.scroll_offset.clamp(0.0, max_scroll);

        let offset = state.scroll_offset;

        renderer.push_scissor(x, y, w, h);
        let outer_clip = anim_ctx.dynamic_half.enter_viewport(Rect::new(x, y, w, h));
        for child_id in children {
            // Render culling: skip children entirely outside the visible viewport
            if let Ok(cl) = taffy.layout(child_id) {
                let child_top = y - offset + cl.location.y;
                let child_bottom = child_top + cl.size.height;
                if child_bottom < y || child_top > y + h {
                    continue;
                }
            }
            render_taffy_node(
                taffy,
                child_id,
                x,
                y - offset,
                renderer,
                interaction,
                scroll_states,
                result,
                anim_ctx,
                depth + 1,
            );
        }
        anim_ctx.dynamic_half.leave_viewport(outer_clip);
        renderer.pop_scissor();

        // Draw scrollbar overlay (no reflow — drawn after content, outside scissor)
        if has_scrollbar {
            let sbar_w = if sbar_pressed {
                sbar_width_active
            } else {
                sbar_width_normal
            };
            let sbar_x = x + w - sbar_w - sbar_margin;
            let thumb_ratio = h / content_height;
            let thumb_height = (h * thumb_ratio).max(16.0);
            let scroll_ratio = if max_scroll > 0.0 {
                offset / max_scroll
            } else {
                0.0
            };
            let thumb_y = y + scroll_ratio * (h - thumb_height);
            let sbar_color = if sbar_pressed { GRAY_50 } else { GRAY_60 };

            renderer.fill_rect(sbar_x, thumb_y, sbar_w, thumb_height, sbar_color);
        }

        return;
    }

    for child_id in children {
        render_taffy_node(
            taffy,
            child_id,
            x,
            y,
            renderer,
            interaction,
            scroll_states,
            result,
            anim_ctx,
            depth + 1,
        );
    }

    // Debug layout outlines — drawn last so they're on top of everything
    if DEBUG_LAYOUT.load(Ordering::Relaxed) && w > 0.0 && h > 0.0 {
        let color = DEBUG_COLORS[depth % DEBUG_COLORS.len()];
        renderer.stroke_rect(x, y, w, h, 1.0, color);
    }
}

/// Draw a paragraph node into its laid-out box.
///
/// A `Clip` or `Ellipsis` line that still overflows the box is scissored
/// at its inline edges only: at a `line_height` near 1.0, ascenders
/// and descenders sit outside the box, and a vertical cut would take them too.
fn draw_paragraph_node(
    renderer: &mut RenderTarget<'_, '_, '_>,
    para: &ParagraphData,
    x: f32,
    y: f32,
    width: f32,
) {
    let (style, spans) = (&para.base_style, para.spans.as_slice());
    let ellipsized = match style.text_overflow {
        TextOverflow::Wrap => {
            renderer.draw_paragraph(style, spans, x, y, width);
            return;
        }
        TextOverflow::Clip => false,
        TextOverflow::Ellipsis => true,
    };
    if width <= 0.0 {
        return;
    }
    // Measure and draw widths can differ by taffy's rounding,
    // so the overflow check shapes at the width actually drawn.
    let (shaped_width, _) = renderer.measure_paragraph(style, spans, ellipsized.then_some(width));
    if shaped_width > width {
        let clip_bottom = renderer.height();
        renderer.draw_paragraph_clipped(style, spans, x, y, width, 0.0, clip_bottom);
    } else {
        renderer.draw_paragraph(style, spans, x, y, width);
    }
}
