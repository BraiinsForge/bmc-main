// Copyright (C) 2026  Braiins Systems s.r.o.

//! Render- and interaction-focused guest imports.

#![expect(clippy::cast_precision_loss)]

mod assets;

use anyhow::Result;
use bmc_wasm_protocol::colors::Color;
use wasmi::{Caller, Linker};

use bmc_render::components::{ButtonSize, ButtonStyle, draw_button};
use bmc_render::tree;

use crate::host_api::HostState;

use super::super::backend::write_touch_hit;
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
            state.frame_schedule.widget_delay_ms = Some(delay_ms);
            state.frame_schedule.deferred_wasm_render_at_ms =
                Some(state.monotonic_ms + u64::from(delay_ms));
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

fn register_tree_imports(linker: &mut Linker<HostState>) -> Result<()> {
    static TOUCH_CLICK_WARNED: std::sync::atomic::AtomicBool = warned_latch();
    static TOUCH_DRAG_WARNED: std::sync::atomic::AtomicBool = warned_latch();

    linker.func_wrap(
        "env",
        "host_submit_tree",
        |mut caller: Caller<'_, HostState>,
         ptr: u32,
         len: u32,
         width: u32,
         height: u32|
         -> Result<(), wasmi::Error> {
            require_render(&caller, "host_submit_tree")?;
            let Some(data) = read_bytes(&caller, ptr, len) else {
                return Ok(());
            };
            let w = width as f32;
            let h = height as f32;
            super::with_renderer_and_state(&mut caller, |renderer, state| {
                let delta_ms = state.delta_ms;
                let frame_counter = state.frame_counter;
                state.frame_counter += 1;
                let mut ctx = bmc_render::ProcessContext {
                    interaction: &mut state.interaction,
                    modal_states: &mut state.modal_states,
                    scroll_states: &mut state.scroll_states,
                    animation_states: &mut state.animation_states,
                    transition_states: &mut state.transition_states,
                    taffy: &mut state.taffy,
                    frame_counter,
                    delta_ms,
                };
                match tree::process_tree(&data, w, h, renderer, &mut ctx) {
                    Ok((tree_node, result, has_active, timings)) => {
                        let had_interaction = !result.clicks.is_empty() || !result.drags.is_empty();
                        state.tree_clicks = result.clicks;
                        state.tree_drags = result.drags;
                        state.last_timings = timings;
                        state.frame_schedule.has_active_animations = has_active;
                        state.frame_schedule.interaction_pending = had_interaction;
                        state.cached_tree = Some((tree_node, w, h));
                    }
                    Err(e) => {
                        tracing::error!("tree processing failed: {e}");
                    }
                }
            })
        },
    )?;

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
