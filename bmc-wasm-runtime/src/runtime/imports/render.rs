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

use std::time::Instant;

use anyhow::Result;
use bmc_wasm_protocol::colors::Color;
use wasmi::{Caller, Linker};

use bmc_render::components::{ButtonSize, ButtonStyle, draw_button};
use bmc_render::tree;
use bmc_render::{FrameTimings, layout_and_render_with_asset_resolver};

use crate::host_api::HostState;

use super::super::backend::{RendererAssetRestorer, write_touch_hit};
use super::super::memory::{read_bytes, read_string};
use super::guards::{forbid_unload, render_or_warn, require_render, warned_latch};

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
    static REQUEST_FRAME_WARNED: std::sync::atomic::AtomicBool = warned_latch();
    static REQUEST_FRAME_AFTER_WARNED: std::sync::atomic::AtomicBool = warned_latch();

    linker.func_wrap(
        "env",
        "host_request_frame",
        |mut caller: Caller<'_, HostState>| {
            if !forbid_unload(&caller, "host_request_frame", &REQUEST_FRAME_WARNED) {
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
            if !forbid_unload(
                &caller,
                "host_request_frame_after",
                &REQUEST_FRAME_AFTER_WARNED,
            ) {
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
                let clicked = draw_button(
                    renderer,
                    &mut state.interaction,
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
                i32::from(clicked.0)
            })
        },
    )?;

    Ok(())
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
        let deserialize_started = Instant::now();
        let tree_node = match tree::deserialize_tree(&data) {
            Ok(tree_node) => tree_node,
            Err(error) => {
                tracing::error!("tree processing failed: {error}");
                return;
            }
        };
        let deserialize_us =
            u32::try_from(deserialize_started.elapsed().as_micros()).unwrap_or(u32::MAX);
        state.last_asset_restoration = None;
        let delta_ms = state.delta_ms;
        let frame_counter = state.frame_counter;
        state.frame_counter += 1;
        let now_unix_secs = state.system_time.timestamp();
        let mut timings = FrameTimings {
            deserialize_us,
            ..FrameTimings::default()
        };
        let mut ctx = bmc_render::ProcessContext {
            interaction: &mut state.interaction,
            modal_states: &mut state.modal_states,
            scroll_states: &mut state.scroll_states,
            animation_states: &mut state.animation_states,
            transition_states: &mut state.transition_states,
            taffy: &mut state.taffy,
            frame_counter,
            delta_ms,
            now_unix_secs,
        };
        let mut resolver = RendererAssetRestorer::new(
            &state.instance_id,
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
                return;
            }
        }
        match render_result {
            Ok((result, has_active)) => {
                let had_interaction = !result.clicks.is_empty() || !result.drags.is_empty();
                state.tree_clicks = result.clicks;
                state.tree_drags = result.drags;
                state.last_timings = timings;
                state.frame_schedule.has_active_animations = has_active;
                state.frame_schedule.interaction_pending = had_interaction;
                state.frame_schedule.host_frame_delay_ms = result.next_frame_delay_ms;
                state.cached_tree = Some((tree_node, w, h));
            }
            Err(error) => {
                tracing::error!("tree processing failed: {error}");
            }
        }
    })
}

fn register_tree_imports(linker: &mut Linker<HostState>) -> Result<()> {
    static TOUCH_CLICK_WARNED: std::sync::atomic::AtomicBool = warned_latch();
    static TOUCH_DRAG_WARNED: std::sync::atomic::AtomicBool = warned_latch();

    linker.func_wrap("env", "host_submit_tree", submit_tree)?;

    linker.func_wrap(
        "env",
        "host_get_touch_click",
        |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, out_ptr: u32| -> i32 {
            if !render_or_warn(&caller, "host_get_touch_click", &TOUCH_CLICK_WARNED) {
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
            if !render_or_warn(&caller, "host_get_touch_drag", &TOUCH_DRAG_WARNED) {
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
