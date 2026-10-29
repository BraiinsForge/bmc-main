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

//! One frame's pass over the tree: layout, the static/dynamic split, and rendering.

use super::Node;
use super::decode::deserialize_tree;
#[cfg(feature = "frame-timings")]
use super::gpu_pass_timing_enabled;
use super::layout::NodeContext;
use super::layout::build_taffy_node;
use super::layout::compute_taffy_layout;
use super::paint::render_taffy_node;
use super::walk::AnimationContext;
use super::walk::DynamicHalf;
use super::walk::EmitMode;
use super::walk::Outlines;
use super::walk::StaticLayer;
use super::walk::TreeResult;
use crate::components::modal::{ModalInfo, render_modal};
use crate::interaction::InteractionState;
use crate::interaction::Rect;
use crate::renderer::prelude::*;
use crate::renderer::{RenderTarget, RendererAssetResolver};
use crate::{
    AnimationState, FrameTimings, ModalState, ScrollState, TransitionState, TransitionStateKey,
};
use anyhow::Result;
use bmc_ui::Color;
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(feature = "frame-timings")]
use std::time::Instant;
use taffy::prelude::*;

/// What a frame does with the renderer's cached static layer.
///
/// Capturing and reusing are alternatives, not flags: a frame either rebuilds
/// the layer or paints the stored one. Only meaningful under [`EmitMode::All`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LayerUse {
    /// Leave the layer alone and emit the whole tree. The fallback for a
    /// caller with no layer of its own — overlays, the gallery.
    #[default]
    Ignore,
    /// Re-render the static half into the layer, then emit the dynamic half
    /// over it. For a frame whose static content has changed, or the first one.
    Capture,
    /// Blit the stored layer instead of rasterising the static half again.
    /// Falls back to a full pass if the blit fails.
    Reuse,
}

/// Per-frame mutable state passed through the render pipeline.
///
/// Bundles the persistent interaction/modal/scroll/animation/transition
/// caches plus the current frame's counter and delta into a single
/// reference-bag, so [`process_tree`] and [`layout_and_render`] don't need
/// a 12-argument signature each.
#[expect(missing_debug_implementations)]
pub struct ProcessContext<'a> {
    pub interaction: &'a mut InteractionState,
    pub modal_states: &'a mut HashMap<String, ModalState>,
    pub scroll_states: &'a mut HashMap<String, ScrollState>,
    pub animation_states: &'a mut HashMap<u64, AnimationState>,
    pub transition_states: &'a mut HashMap<TransitionStateKey, TransitionState>,
    pub taffy: &'a mut TaffyTree<NodeContext>,
    pub frame_counter: u64,
    pub delta_ms: u32,
    /// Injected unix-seconds clock for host-formatted time nodes.
    pub now_unix_secs: i64,
    /// Which half of the static/dynamic partition to emit; see [`EmitMode`].
    /// [`EmitMode::All`] reproduces a full frame.
    pub emit: EmitMode,

    /// Regions to repaint, in logical pixels, when the frame can reuse what the
    /// target already holds. Empty means repaint everything.
    ///
    /// These come from the *previous* frames' walks: the damage has to be known
    /// before the layer is composited rather than after the walk that would
    /// discover it, and layout is identical on a cached frame.
    pub damage_rects: &'a [Rect],
    /// What this frame does with the cached static layer; see [`LayerUse`].
    pub static_layer: LayerUse,
    /// Identifies whose static layer to capture and blit. One renderer serves
    /// every widget slot, so callers must pass distinct keys or they overwrite
    /// each other's layer. Use the caller's asset namespace, so slot teardown
    /// reclaims the layer with its other assets.
    pub static_layer_key: &'a str,
    /// Report the laid-out boxes in [`TreeResult::layout_outlines`],
    /// for a host to outline over the frame.
    pub layout_outlines: bool,
    /// Stroke an outline around each laid-out box into the frame itself,
    /// as [`super::debug_layout_enabled`] asks of a host whose flag cannot change.
    pub stroke_outlines: bool,
}

impl ProcessContext<'_> {
    fn outlines(&self) -> Outlines {
        Outlines {
            record: self.layout_outlines,
            stroke: self.stroke_outlines,
        }
    }
}

/// Process a tree: deserialize, layout, render.
///
/// Returns `(tree_node, result, has_active_animations, timings)` — the caller
/// can cache `tree_node` for animation-only frames to skip deserialization.
pub fn process_tree(
    data: &[u8],
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    ctx: &mut ProcessContext<'_>,
) -> Result<(Node, TreeResult, bool, FrameTimings)> {
    let mut timings = FrameTimings::default();

    // Phase 1: Deserialize
    #[cfg(feature = "frame-timings")]
    let t0 = Instant::now();
    let tree_node = deserialize_tree(data)?;
    #[cfg(feature = "frame-timings")]
    {
        timings.deserialize_us = elapsed_us(t0);
    }

    let (result, has_active) =
        layout_and_render(&tree_node, width, height, renderer, &mut timings, ctx)?;

    Ok((tree_node, result, has_active, timings))
}

/// Layout and render a previously deserialized tree.
///
/// Populates `timings.layout_us` and `timings.render_us`.
pub fn layout_and_render(
    tree_node: &Node,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    timings: &mut FrameTimings,
    ctx: &mut ProcessContext<'_>,
) -> Result<(TreeResult, bool)> {
    layout_and_render_inner(tree_node, width, height, renderer, None, timings, ctx)
}

/// Layout and render a tree, resolving suspended assets immediately before their draws.
pub fn layout_and_render_with_asset_resolver(
    tree_node: &Node,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    resolver: &mut dyn RendererAssetResolver,
    timings: &mut FrameTimings,
    ctx: &mut ProcessContext<'_>,
) -> Result<(TreeResult, bool)> {
    layout_and_render_inner(
        tree_node,
        width,
        height,
        renderer,
        Some(resolver),
        timings,
        ctx,
    )
}

fn layout_and_render_inner(
    tree_node: &Node,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    resolver: Option<&mut dyn RendererAssetResolver>,
    timings: &mut FrameTimings,
    ctx: &mut ProcessContext<'_>,
) -> Result<(TreeResult, bool)> {
    // Phase 2: Build Taffy tree + compute layout
    #[cfg(feature = "frame-timings")]
    let t1 = Instant::now();

    let mut result = TreeResult::default();
    let mut modals: Vec<ModalInfo> = Vec::new();

    // Reuse taffy tree — clear nodes but keep internal allocations
    ctx.taffy.clear();
    let root_id = build_taffy_node(
        ctx.taffy,
        tree_node,
        ctx.now_unix_secs,
        &mut result,
        &mut modals,
    )?;

    pin_root_size(ctx.taffy, root_id, width, height)?;

    #[cfg(feature = "frame-timings")]
    {
        timings.taffy_build_us = elapsed_us(t1);
    }

    #[cfg(feature = "frame-timings")]
    let t_compute = Instant::now();
    compute_taffy_layout(ctx.taffy, root_id, renderer)?;
    #[cfg(feature = "frame-timings")]
    {
        timings.taffy_compute_us = elapsed_us(t_compute);
        let (misses, entries) = renderer.paragraph_cache_stats();
        timings.paragraph_misses = misses;
        timings.paragraph_entries = u32::try_from(entries).unwrap_or(u32::MAX);
    }

    result.content_size = root_content_size(ctx.taffy, root_id);

    #[cfg(feature = "frame-timings")]
    {
        timings.layout_us = elapsed_us(t1);
    }

    // Phase 3: Render
    #[cfg(feature = "frame-timings")]
    let t2 = Instant::now();

    let outlines = ctx.outlines();
    let mut anim_ctx = AnimationContext {
        emit: ctx.emit,
        animation_states: ctx.animation_states,
        transition_states: ctx.transition_states,
        delta_ms: ctx.delta_ms,
        frame_counter: ctx.frame_counter,
        draw_counter: 0,
        canvas_index: 0,
        draw_in_canvas: 0,
        mesh_slot_counter: 0,
        dynamic_half: DynamicHalf::default(),
        layer_painted: false,
        has_active: false,
        now_unix_secs: ctx.now_unix_secs,
        outlines,
    };
    let resolver = resolver.map(RefCell::new);
    let mut target = RenderTarget::new(renderer, resolver.as_ref());

    render_tree(
        root_id,
        width,
        height,
        &mut target,
        ctx.taffy,
        ctx.interaction,
        ctx.scroll_states,
        ctx.static_layer,
        ctx.static_layer_key,
        ctx.damage_rects,
        &mut result,
        timings,
        &mut anim_ctx,
    );

    render_modal_overlays(
        &modals,
        width,
        height,
        &mut target,
        ctx.interaction,
        ctx.modal_states,
        ctx.scroll_states,
        ctx.delta_ms,
        ctx.taffy,
        &mut result,
        &mut anim_ctx,
    );

    record_modal_damage(ctx.modal_states, width, height, &mut result);

    sweep_stale_state(&mut anim_ctx, ctx.frame_counter, timings);
    #[cfg(feature = "frame-timings")]
    {
        timings.hit_region_count = ctx.interaction.hit_region_count();
        timings.dynamic_node_count = result.dynamic_node_count;
        timings.dynamic_area_pct = dynamic_area_pct(result.dynamic_bounds, width, height);
        timings.render_us = elapsed_us(t2);
    }

    // Ends `anim_ctx`'s borrows of `ctx` before the check below takes it.
    let has_active = anim_ctx.has_active;
    let _ = anim_ctx;

    // Progress alone is ambiguous: 0.0 means "fully closed" only when `is_open`
    // is false, and otherwise means the modal has just started opening.
    let modal_animating = ctx.modal_states.values().any(|s| {
        if s.is_open {
            s.animation_progress < 1.0
        } else {
            s.animation_progress > 0.0
        }
    });

    Ok((result, has_active || modal_animating))
}

/// Damage the whole surface while a modal is on screen.
///
/// A modal lays out as a `Display::None` leaf and is painted by a later overlay
/// pass, so nothing has described its backdrop to damage tracking. Left
/// unrecorded, the darkening lands only inside the rects the walk reported, and
/// the pixels it covered never repaint once it closes.
///
/// The whole surface, not the panel: the backdrop dims everything. Call after
/// the overlay pass, where a modal's state first appears, or the frame it opens
/// on records nothing.
fn record_modal_damage(
    modal_states: &HashMap<String, ModalState>,
    width: f32,
    height: f32,
    result: &mut TreeResult,
) {
    let visible = modal_states
        .values()
        .any(|s| s.is_open || s.animation_progress > 0.0);
    if !visible {
        return;
    }
    let surface = Rect::new(0.0, 0.0, width, height);
    Rect::union_bounds(&mut result.dynamic_bounds, surface);
    result.dynamic_rects.push(surface);
    result.dynamic_node_count += 1;
}

/// Paint the frame's open modals, always emitting in full.
///
/// A modal is never in the cached layer, so the static half of its body has
/// nowhere else to come from: emitting under [`EmitMode::DynamicOnly`], which is
/// where [`render_tree`] leaves `emit` after a blit, drops the body text.
#[expect(clippy::too_many_arguments, reason = "render plumbing is irreducible")]
fn render_modal_overlays(
    modals: &[ModalInfo],
    width: f32,
    height: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<String, ModalState>,
    scroll_states: &mut HashMap<String, ScrollState>,
    delta_ms: u32,
    taffy: &mut TaffyTree<NodeContext>,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
) {
    if modals.is_empty() {
        return;
    }
    let emit_before = anim_ctx.emit;
    anim_ctx.emit = EmitMode::All;
    // An overlay paints over the whole frame, layer and dynamic half alike,
    // so it starts its own paint order rather than inheriting the tree's.
    let painted_before = std::mem::take(&mut anim_ctx.dynamic_half);
    for modal in modals {
        render_modal(
            modal,
            width,
            height,
            renderer,
            interaction,
            modal_states,
            scroll_states,
            delta_ms,
            result,
            anim_ctx,
            taffy,
        );
    }
    anim_ctx.emit = emit_before;
    anim_ctx.dynamic_half = painted_before;
}

/// Render the tree, splitting into a static and a dynamic pass when this frame
/// is refreshing the cached layer.
///
/// A refreshing frame renders in the same shape as one reusing the layer:
/// static into the layer, blit, dynamic on top — so static content is
/// rasterised once per guest frame rather than once for the screen and again
/// for the layer. Falls back to one full pass when the layer is unavailable.
#[expect(clippy::too_many_arguments, reason = "render plumbing is irreducible")]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "surface dimensions are small positive values"
)]
fn render_tree(
    root_id: taffy::NodeId,
    width: f32,
    height: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    taffy: &TaffyTree<NodeContext>,
    interaction: &mut InteractionState,
    scroll_states: &mut HashMap<String, ScrollState>,
    static_layer: LayerUse,
    static_layer_key: &str,
    damage_rects: &[Rect],
    result: &mut TreeResult,
    timings: &mut FrameTimings,
    anim_ctx: &mut AnimationContext<'_>,
) {
    // The parameter stays either way, so callers need no cfg of their own.
    #[cfg(not(feature = "frame-timings"))]
    let _ = timings;

    // Drain whatever is already queued so the first measured pass is not
    // charged for work recorded before it.
    #[cfg(feature = "frame-timings")]
    let pass_timing = gpu_pass_timing_enabled();
    #[cfg(feature = "frame-timings")]
    if pass_timing {
        renderer.flush_and_fence_us();
    }

    match (anim_ctx.emit, static_layer) {
        (EmitMode::All, LayerUse::Capture) => {
            if renderer.begin_static_layer(static_layer_key, width as u32, height as u32) {
                anim_ctx.emit = EmitMode::StaticOnly;
                render_taffy_node(
                    taffy,
                    root_id,
                    0.0,
                    0.0,
                    renderer,
                    interaction,
                    scroll_states,
                    result,
                    anim_ctx,
                    0,
                );
                renderer.end_static_layer(static_layer_key);
                #[cfg(feature = "frame-timings")]
                if pass_timing {
                    timings.gpu_capture_us = renderer.flush_and_fence_us();
                }
                let blitted = renderer.blit_static_layer(static_layer_key);
                #[cfg(feature = "frame-timings")]
                if pass_timing {
                    timings.gpu_blit_us = renderer.flush_and_fence_us();
                }
                restart_walk(anim_ctx);
                if blitted {
                    result.static_layer = StaticLayer::Captured {
                        empty: !anim_ctx.layer_painted,
                    };
                    anim_ctx.emit = EmitMode::DynamicOnly;
                } else {
                    paint_in_full(renderer, width, height, result, anim_ctx);
                }
            } else {
                paint_in_full(renderer, width, height, result, anim_ctx);
            }
        }
        // Nothing static changed since the layer was captured,
        // so paint it rather than rasterising the same content again.
        (EmitMode::All, LayerUse::Reuse) => {
            // Repaint only the damaged regions when the caller supplied them;
            // the rest of the target still holds a correct earlier frame.
            let blitted = if damage_rects.is_empty() {
                renderer.blit_static_layer(static_layer_key)
            } else {
                renderer.blit_static_layer_rects(static_layer_key, damage_rects)
            };
            if blitted {
                result.static_layer = StaticLayer::Blitted;
                #[cfg(feature = "frame-timings")]
                if pass_timing {
                    timings.gpu_blit_us = renderer.flush_and_fence_us();
                }
                anim_ctx.emit = EmitMode::DynamicOnly;
            } else {
                paint_in_full(renderer, width, height, result, anim_ctx);
            }
        }
        (EmitMode::All, LayerUse::Ignore) | (EmitMode::StaticOnly | EmitMode::DynamicOnly, _) => {}
    }
    render_taffy_node(
        taffy,
        root_id,
        0.0,
        0.0,
        renderer,
        interaction,
        scroll_states,
        result,
        anim_ctx,
        0,
    );

    #[cfg(feature = "frame-timings")]
    if pass_timing {
        timings.gpu_dynamic_us = renderer.flush_and_fence_us();
    }
}

/// Restart the positional counters, so the pass after a capture walks the tree
/// as a frame without one does.
fn restart_walk(anim_ctx: &mut AnimationContext<'_>) {
    // Animation state keys on `draw_counter`.
    anim_ctx.draw_counter = 0;
    anim_ctx.canvas_index = 0;
    // Bands are positional too: starting from what the last pass painted
    // would band the whole tree `Above` and repaint what the layer has
    // just supplied.
    anim_ctx.dynamic_half.clear();
    // The mesh atlas caches per slot and skips re-rendering when a slot
    // already holds the same mesh, so a draw landing on a different slot
    // depending on frame type would re-render on every alternation.
    anim_ctx.mesh_slot_counter = 0;
}

/// Give up on the layer for this frame and paint the whole tree.
fn paint_in_full(
    renderer: &mut RenderTarget<'_, '_, '_>,
    width: f32,
    height: f32,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
) {
    // The walk repaints every draw but only where a draw paints,
    // so on a target the caller left uncleared,
    // whatever the dynamic half painted last frame would stay where it was.
    // Opaque black, to match what `begin_static_layer` captures against.
    renderer.fill_rect(0.0, 0.0, width, height, Color::from_rgb(0, 0, 0));
    result.static_layer = StaticLayer::Missed;
    anim_ctx.emit = EmitMode::All;
}

/// The root's laid-out content extent, or zero when layout produced none.
fn root_content_size(taffy: &TaffyTree<NodeContext>, root_id: NodeId) -> (f32, f32) {
    taffy.layout(root_id).map_or((0.0, 0.0), |layout| {
        (layout.content_size.width, layout.content_size.height)
    })
}

/// Override the root's size per axis: a non-zero value pins that axis, and zero
/// keeps the node's own style so its content sizes it.
fn pin_root_size(
    taffy: &mut TaffyTree<NodeContext>,
    root_id: NodeId,
    width: f32,
    height: f32,
) -> Result<()> {
    if width <= 0.0 && height <= 0.0 {
        return Ok(());
    }
    if let Ok(style) = taffy.style(root_id) {
        let mut new_style = style.clone();
        if width > 0.0 {
            new_style.size.width = length(width);
        }
        if height > 0.0 {
            new_style.size.height = length(height);
        }
        taffy.set_style(root_id, new_style)?;
    }
    Ok(())
}

/// Microseconds since `start`, saturating rather than wrapping.
#[cfg(feature = "frame-timings")]
fn elapsed_us(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX)
}

/// Share of a `width` x `height` surface covered by `bounds`, in percent.
#[cfg(feature = "frame-timings")]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a percentage of a surface is a small non-negative value"
)]
fn dynamic_area_pct(bounds: Option<Rect>, width: f32, height: f32) -> u32 {
    let surface = width * height;
    if surface <= 0.0 {
        return 0;
    }
    bounds.map_or(0, |Rect { w, h, .. }| {
        (100.0 * w * h / surface).round() as u32
    })
}

/// Drop animation and transition states no walk touched this frame, and record
/// what survived.
///
/// Both are keyed by content, so a node that stopped drawing would otherwise
/// keep its state alive forever. The counts ride along because this is the only
/// point where the swept sizes are known.
fn sweep_stale_state(
    anim_ctx: &mut AnimationContext<'_>,
    frame_counter: u64,
    timings: &mut FrameTimings,
) {
    // The parameter stays either way, so callers need no cfg of their own.
    #[cfg(not(feature = "frame-timings"))]
    let _ = timings;

    anim_ctx
        .animation_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);
    anim_ctx
        .transition_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);

    #[cfg(feature = "frame-timings")]
    {
        timings.animation_state_count = anim_ctx.animation_states.len();
        timings.transition_state_count = anim_ctx.transition_states.len();
    }
}
