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

//! Render- and interaction-focused guest imports.

#![expect(clippy::cast_precision_loss)]

mod assets;

#[cfg(feature = "frame-timings")]
use std::time::Instant;

use anyhow::Result;
use bmc_ui::Color;
use wasmi::{Caller, Linker};

use bmc_render::components::{ButtonSize, ButtonStyle, draw_button};
use bmc_render::partition::static_layer::{AssetGeneration, LayerCommit, LayerPlan};
use bmc_render::renderer::Renderer;
use bmc_render::tree;
use bmc_render::{FrameTimings, layout_and_render_with_asset_resolver};

use crate::host_api::{CachedTree, HostState};

use super::super::backend::{RendererAssetRestorer, write_touch_hit};
use super::super::memory::{read_bytes, read_string};
use super::guards::{forbid_unload, render_or_warn, require_render};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_primitives(linker)?;
    register_frame_control(linker)?;
    register_button_import(linker)?;
    assets::register(linker)?;
    register_tree_imports(linker)?;
    Ok(())
}

fn register_primitives(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fill_rect",
        |mut caller: Caller<'_, HostState>,
         x: i32,
         y: i32,
         w: u32,
         h: u32,
         color: u32|
         -> Result<(), wasmi::Error> {
            super::with_renderer(&mut caller, |renderer| {
                renderer.fill_rect(
                    x as f32,
                    y as f32,
                    w as f32,
                    h as f32,
                    Color::from_raw(color),
                );
            })
        },
    )?;

    linker.func_wrap(
        "env",
        "host_draw_rounded_rect",
        |mut caller: Caller<'_, HostState>,
         x: i32,
         y: i32,
         w: u32,
         h: u32,
         radius: u32,
         color: u32|
         -> Result<(), wasmi::Error> {
            super::with_renderer(&mut caller, |renderer| {
                renderer.fill_rounded_rect(
                    x as f32,
                    y as f32,
                    w as f32,
                    h as f32,
                    radius as f32,
                    Color::from_raw(color),
                );
            })
        },
    )?;

    linker.func_wrap(
        "env",
        "host_draw_text",
        |mut caller: Caller<'_, HostState>,
         text_ptr: u32,
         text_len: u32,
         x: i32,
         y: i32,
         size: u32,
         color: u32|
         -> Result<(), wasmi::Error> {
            let Some(text) = read_string(&caller, text_ptr, text_len) else {
                return Ok(());
            };
            super::with_renderer(&mut caller, |renderer| {
                renderer.draw_text(
                    &text,
                    x as f32,
                    y as f32,
                    size as f32,
                    Color::from_raw(color),
                );
            })
        },
    )?;

    Ok(())
}

fn register_frame_control(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_request_frame",
        |mut caller: Caller<'_, HostState>| {
            if !forbid_unload(&mut caller, "host_request_frame") {
                return;
            }
            let state = caller.data_mut();
            state.frame_schedule.widget_delay_ms = Some(0);
        },
    )?;

    linker.func_wrap(
        "env",
        "host_request_frame_after",
        |mut caller: Caller<'_, HostState>, delay_ms: u32| {
            if !forbid_unload(&mut caller, "host_request_frame_after") {
                return;
            }
            let state = caller.data_mut();
            let now = state.monotonic_ms;
            state.frame_schedule.request_frame_after(delay_ms, now);
        },
    )?;

    Ok(())
}

fn register_button_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_button",
        |mut caller: Caller<'_, HostState>,
         key_ptr: u32,
         key_len: u32,
         label_ptr: u32,
         label_len: u32,
         x: i32,
         y: i32,
         w: u32,
         h: u32,
         style: u32|
         -> Result<i32, wasmi::Error> {
            let key = read_string(&caller, key_ptr, key_len);
            let label = read_string(&caller, label_ptr, label_len);
            let (Some(key), Some(label)) = (key, label) else {
                return Ok(0);
            };

            super::with_renderer_and_state(&mut caller, |renderer, state| {
                super::require_renderer_gpu_access(state)?;
                let clicked = draw_button(
                    renderer,
                    &mut state.walk.interaction,
                    &key,
                    &label,
                    x as f32,
                    y as f32,
                    w as f32,
                    h as f32,
                    ButtonStyle::from(style),
                    ButtonSize::Normal,
                    None,
                    false,
                    None,
                );
                Ok(i32::from(clicked.0))
            })
            .and_then(std::convert::identity)
        },
    )?;

    Ok(())
}

/// Whether the guest submitted its tree at the size the surface has.
///
/// A tree submitted at another size means the guest and the host
/// disagree about the surface, which invalidates the whole frame
/// rather than one draw: taffy laid every node out for those dimensions,
/// so the dynamic half is painted for the wrong surface too,
/// and the static layer is captured at one size while
/// the raw blit stretches it over another.
///
/// Refuse the frame rather than paint a consistent-looking lie.
fn submitted_at_surface_size(state: &HostState, width: u32, height: u32) -> bool {
    let matches = width == state.widget_width && height == state.widget_height;
    if !matches {
        tracing::error!(
            guest = format!("{width}x{height}"),
            host = format!("{}x{}", state.widget_width, state.widget_height),
            "tree submitted at a size the surface does not have; frame dropped"
        );
    }
    matches
}

fn submit_tree(
    mut caller: Caller<'_, HostState>,
    ptr: u32,
    len: u32,
    width: u32,
    height: u32,
) -> Result<(), wasmi::Error> {
    require_render(&caller, "host_submit_tree")?;
    let Some(data) = read_bytes(&caller, ptr, len) else {
        return Ok(());
    };
    let w = width as f32;
    let h = height as f32;
    super::with_renderer_and_state(&mut caller, |renderer, state| {
        if !submitted_at_surface_size(state, width, height) {
            return;
        }
        #[cfg(feature = "frame-timings")]
        let deserialize_started = Instant::now();
        let (tree_node, static_bytes) = match tree::deserialize_tree_keyed(&data) {
            Ok(decoded) => decoded,
            Err(error) => {
                tracing::error!("tree processing failed: {error}");
                return;
            }
        };
        #[cfg(feature = "frame-timings")]
        let deserialize_us =
            u32::try_from(deserialize_started.elapsed().as_micros()).unwrap_or(u32::MAX);
        let now_unix_secs = state.system_time.timestamp();
        let frame = state.layer_frame(&tree_node, static_bytes, true, now_unix_secs);
        let plan = state.static_layer.plan(renderer, frame);
        state.last_asset_restoration = None;
        let delta_ms = state.delta_ms;
        let mut timings = FrameTimings::default();
        #[cfg(feature = "frame-timings")]
        {
            timings.deserialize_us = deserialize_us;
        }
        let mut ctx = bmc_render::ProcessContext {
            static_layer: state.static_layer.layer_use(plan),
            layout_outlines: state.layout_outlines.is_some(),
            stroke_outlines: bmc_render::tree::debug_layout_enabled(),
            ..state
                .walk
                .next_context(delta_ms, now_unix_secs, &state.host_intl)
        };
        let mut resolver = RendererAssetRestorer::new(
            state.instance_id.as_str(),
            state.asset_cache.as_ref(),
            state.package_assets.as_ref(),
            &mut state.renderer_assets,
            &mut state.profile_sections,
        );
        let render_result = layout_and_render_with_asset_resolver(
            &tree_node,
            w,
            h,
            renderer,
            &mut resolver,
            &mut timings,
            &mut ctx,
        );
        match resolver.finish() {
            Ok(observation) => state.last_asset_restoration = observation,
            Err(error) => {
                state.renderer_asset_failure = Some(error);
                state.static_layer.forget();
                return;
            }
        }
        match render_result {
            Ok((result, has_active)) => commit_submitted_frame(
                renderer,
                state,
                SubmittedFrame {
                    tree_node,
                    w,
                    h,
                    static_bytes,
                    plan,
                    timings,
                    has_active,
                },
                result,
            ),
            Err(error) => {
                tracing::error!("tree processing failed: {error}");
                state.static_layer.forget();
            }
        }
    })
}

/// What a completed guest frame contributes to [`HostState`] beyond its draws.
struct SubmittedFrame {
    tree_node: bmc_render::tree::Node,
    w: f32,
    h: f32,
    static_bytes: bmc_render::partition::StaticBytesKey,
    plan: LayerPlan,
    timings: bmc_render::FrameTimings,
    has_active: bool,
}

/// Fold a rendered guest frame's results into the host state.
fn commit_submitted_frame(
    renderer: &mut dyn Renderer,
    state: &mut HostState,
    frame: SubmittedFrame,
    result: bmc_render::tree::TreeResult,
) {
    let had_interaction = !result.clicks.is_empty() || !result.drags.is_empty();
    state.tree_clicks = result.clicks;
    state.tree_drags = result.drags;
    state.last_timings = frame.timings;
    state.frame_schedule.has_active_animations = frame.has_active;
    state.frame_schedule.interaction_pending = had_interaction;
    state.frame_schedule.host_frame_delay_ms = result.next_frame_delay_ms;
    state.cached_tree = Some(CachedTree {
        node: frame.tree_node,
        width: frame.w,
        height: frame.h,
        static_bytes: frame.static_bytes,
    });
    state.guest_tree = crate::host_api::GuestTree::Committed;
    let walked_assets = AssetGeneration::Ledger(state.renderer_assets.generation());
    let committed =
        state
            .static_layer
            .commit(renderer, frame.plan, result.static_layer, walked_assets);
    // This frame painted the buffer it holds in full. One that reused its layer, blitted or
    // missed, kept its static half. Any other may have changed it, and every other buffer
    // keeps showing the old one until it repaints in full too.
    match committed {
        LayerCommit::Reused | LayerCommit::Missed => state.painted_in_full(),
        LayerCommit::Rewritten | LayerCommit::Unlayered => {
            state.stale_export_buffers = crate::host_api::EXPORT_BUFFERS - 1;
        }
    }
    state.keep_walk(&result.dynamic_rects, result.layout_outlines);
    state.last_repaint = crate::host_api::FrameRepaint::Full;
}

fn register_tree_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "host_submit_tree", submit_tree)?;

    linker.func_wrap(
        "env",
        "host_get_touch_click",
        |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, out_ptr: u32| -> i32 {
            if !render_or_warn(&mut caller, "host_get_touch_click") {
                return 0;
            }
            let Some(key) = read_string(&caller, key_ptr, key_len) else {
                return 0;
            };
            let hit = caller.data().tree_clicks.get(&key).copied();
            let Some(hit) = hit else { return 0 };
            write_touch_hit(&mut caller, out_ptr, &hit);
            1
        },
    )?;

    linker.func_wrap(
        "env",
        "host_get_touch_drag",
        |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, out_ptr: u32| -> i32 {
            if !render_or_warn(&mut caller, "host_get_touch_drag") {
                return 0;
            }
            let Some(key) = read_string(&caller, key_ptr, key_len) else {
                return 0;
            };
            let hit = caller.data().tree_drags.get(&key).copied();
            let Some(hit) = hit else { return 0 };
            write_touch_hit(&mut caller, out_ptr, &hit);
            1
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HostState, SubmittedFrame, commit_submitted_frame};
    use bmc_render::partition::StaticBytesKey;
    use bmc_render::partition::static_layer::{AssetGeneration, LayerPlan};
    use bmc_render::renderer::test_support::ShapingRecorder;
    use bmc_render::tree::{Node, StaticLayer, TreeResult};
    use bmc_ui::{Color, PropsData};

    const EMPTY_BYTES: StaticBytesKey = StaticBytesKey(1);
    const OTHER_BYTES: StaticBytesKey = StaticBytesKey(4);

    fn state() -> HostState {
        HostState::new(
            crate::runtime_limits::RuntimeResourceLimits::default(),
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .expect("BUG: fixed test timestamp must parse"),
            crate::runtime::FetchAgent::default(),
        )
    }

    /// Static content as far as the tree can tell.
    fn panel() -> Node {
        Node::Column(
            PropsData {
                background: Color::from_rgb(1, 2, 3),
                ..PropsData::default()
            },
            Vec::new(),
        )
    }

    /// Plan a guest frame of `tree`, whose wire bytes key as `bytes`.
    fn plan(
        renderer: &mut ShapingRecorder,
        state: &mut HostState,
        tree: &Node,
        bytes: StaticBytesKey,
    ) -> LayerPlan {
        let frame = state.layer_frame(tree, bytes, true, 0);
        state.static_layer.plan(renderer, frame)
    }

    /// Commit a guest frame of `panel()` keyed as `bytes`, whose walk did `layer`.
    fn commit(
        renderer: &mut ShapingRecorder,
        state: &mut HostState,
        bytes: StaticBytesKey,
        layer: StaticLayer,
    ) {
        let plan = plan(renderer, state, &panel(), bytes);
        commit_plan(renderer, state, bytes, plan, layer);
    }

    /// Commit a guest frame of `panel()` keyed as `bytes` and planned as `plan`,
    /// whose walk did `layer`.
    fn commit_plan(
        renderer: &mut ShapingRecorder,
        state: &mut HostState,
        bytes: StaticBytesKey,
        plan: LayerPlan,
        layer: StaticLayer,
    ) {
        commit_submitted_frame(
            renderer,
            state,
            SubmittedFrame {
                tree_node: panel(),
                w: 100.0,
                h: 100.0,
                static_bytes: bytes,
                plan,
                timings: bmc_render::FrameTimings::default(),
                has_active: true,
            },
            TreeResult {
                static_layer: layer,
                ..TreeResult::default()
            },
        );
    }

    fn commit_empty_capture(renderer: &mut ShapingRecorder, state: &mut HostState) {
        commit(
            renderer,
            state,
            EMPTY_BYTES,
            StaticLayer::Captured { empty: true },
        );
    }

    #[test]
    fn a_capture_whose_walk_restored_an_asset_is_reused_next() {
        let (mut renderer, mut state) = (ShapingRecorder::default(), state());
        let captured = plan(&mut renderer, &mut state, &panel(), OTHER_BYTES);
        // Stands in for an asset the walk restored, which moves the generation.
        state.renderer_assets.clear();
        commit_plan(
            &mut renderer,
            &mut state,
            OTHER_BYTES,
            captured,
            StaticLayer::Captured { empty: false },
        );
        assert!(matches!(
            plan(&mut renderer, &mut state, &panel(), OTHER_BYTES),
            LayerPlan::Reuse(_)
        ));
    }

    #[test]
    fn a_guest_frame_reports_a_full_repaint() {
        let (mut renderer, mut state) = (ShapingRecorder::default(), state());
        commit_empty_capture(&mut renderer, &mut state);
        assert!(
            matches!(state.last_repaint, crate::host_api::FrameRepaint::Full),
            "got {:?}",
            state.last_repaint
        );
    }

    #[test]
    fn a_reused_layer_owes_no_buffer_a_repaint() {
        let (mut renderer, mut state) = (ShapingRecorder::default(), state());
        commit(
            &mut renderer,
            &mut state,
            OTHER_BYTES,
            StaticLayer::Captured { empty: false },
        );
        assert_eq!(
            state.stale_export_buffers,
            crate::host_api::EXPORT_BUFFERS - 1
        );
        commit(&mut renderer, &mut state, OTHER_BYTES, StaticLayer::Blitted);
        assert_eq!(
            state.stale_export_buffers,
            crate::host_api::EXPORT_BUFFERS - 2
        );
    }

    #[test]
    fn a_missed_reuse_owes_no_buffer_a_repaint() {
        let (mut renderer, mut state) = (ShapingRecorder::default(), state());
        commit(
            &mut renderer,
            &mut state,
            OTHER_BYTES,
            StaticLayer::Captured { empty: false },
        );
        commit(&mut renderer, &mut state, OTHER_BYTES, StaticLayer::Missed);
        assert_eq!(
            state.stale_export_buffers,
            crate::host_api::EXPORT_BUFFERS - 2,
            "a miss repaints its buffer in full and keeps the static half"
        );
    }

    #[test]
    fn a_reuse_the_walk_captured_owes_the_other_buffers() {
        let (mut renderer, mut state) = (ShapingRecorder::default(), state());
        commit(
            &mut renderer,
            &mut state,
            OTHER_BYTES,
            StaticLayer::Captured { empty: false },
        );
        state.stale_export_buffers = 0;
        let reuse = plan(&mut renderer, &mut state, &panel(), OTHER_BYTES);
        assert!(matches!(reuse, LayerPlan::Reuse(_)));
        // As when the layer was stale against the frame's background, so the walk captured it.
        commit_plan(
            &mut renderer,
            &mut state,
            OTHER_BYTES,
            reuse,
            StaticLayer::Captured { empty: false },
        );
        assert_eq!(
            state.stale_export_buffers,
            crate::host_api::EXPORT_BUFFERS - 1,
            "the other buffers still show the static half the walk replaced"
        );
    }

    /// Whether a tree keyed like the last frame keeps its layer while a finger drags.
    fn keeps_its_layer_under_a_drag(tree: &Node) -> bool {
        use bmc_render::interaction::TouchEvent;
        let (mut renderer, mut state) = (ShapingRecorder::default(), state());
        state
            .walk
            .interaction
            .push_event(TouchEvent::Down { x: 10.0, y: 10.0 });
        state.walk.interaction.begin_frame();
        state
            .walk
            .interaction
            .push_event(TouchEvent::Move { x: 10.0, y: 14.0 });
        state.walk.interaction.begin_frame();
        assert!(state.walk.dragging(), "the move must register as a drag");
        let captured = plan(&mut renderer, &mut state, tree, OTHER_BYTES);
        state.static_layer.commit(
            &mut renderer,
            captured,
            StaticLayer::Captured { empty: false },
            AssetGeneration::Ledger(state.renderer_assets.generation()),
        );
        matches!(
            plan(&mut renderer, &mut state, tree, OTHER_BYTES),
            LayerPlan::Reuse(_)
        )
    }

    #[test]
    fn a_drag_without_a_scroll_container_keeps_the_layer() {
        assert!(keeps_its_layer_under_a_drag(&panel()));
    }

    #[test]
    fn a_drag_over_a_scroll_container_captures_again() {
        let list = bmc_ui::scroll("list", PropsData::default(), [panel()]);
        assert!(!keeps_its_layer_under_a_drag(&list));
    }
}
