// Copyright (C) 2026  Braiins Systems s.r.o.

//! Render- and interaction-focused guest imports.

#![expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use anyhow::Result;
use chrono::{Datelike, Timelike};
use wasmi::{Caller, Extern, Linker};

use crate::components::{ButtonSize, ButtonStyle, draw_button};
use crate::host_api::HostState;
use crate::renderer::Renderer;
use crate::tree;

use super::super::backend::{decode_image_rgba_limited, write_touch_hit};
use super::super::memory::{read_bytes, read_string};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_primitives(linker)?;
    register_frame_control(linker)?;
    register_button_import(linker)?;
    register_bitmap_storage_imports(linker)?;
    register_image_decode_import(linker)?;
    register_tree_imports(linker)?;
    register_system_time_import(linker)?;
    Ok(())
}

fn register_primitives(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fill_rect",
        |mut caller: Caller<'_, HostState>, x: i32, y: i32, w: u32, h: u32, color: u32| {
            let state = caller.data_mut();
            state
                .renderer
                .fill_rect(x as f32, y as f32, w as f32, h as f32, color);
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
         color: u32| {
            let state = caller.data_mut();
            state.renderer.fill_rounded_rect(
                x as f32,
                y as f32,
                w as f32,
                h as f32,
                radius as f32,
                color,
            );
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
         color: u32| {
            let Some(text) = read_string(&caller, text_ptr, text_len) else {
                return;
            };
            let state = caller.data_mut();
            state
                .renderer
                .draw_text(&text, x as f32, y as f32, size as f32, color);
        },
    )?;

    Ok(())
}

fn register_frame_control(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_request_frame",
        |mut caller: Caller<'_, HostState>| {
            let state = caller.data_mut();
            state.frame_requested = true;
            state.animation_only_frame = false;
        },
    )?;

    linker.func_wrap(
        "env",
        "host_request_frame_after",
        |mut caller: Caller<'_, HostState>, delay_ms: u32| {
            let state = caller.data_mut();
            state.frame_requested = true;
            state.frame_delay_ms = Some(delay_ms);
            state.animation_only_frame = false;
            state.deferred_wasm_render_at_ms = Some(state.monotonic_ms + u64::from(delay_ms));
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
         -> i32 {
            let key = read_string(&caller, key_ptr, key_len);
            let label = read_string(&caller, label_ptr, label_len);
            let (Some(key), Some(label)) = (key, label) else {
                return 0;
            };

            let state = caller.data_mut();
            let clicked = draw_button(
                &mut state.renderer,
                &mut state.interaction,
                &key,
                &label,
                x as f32,
                y as f32,
                w as f32,
                h as f32,
                ButtonStyle::from(style),
                ButtonSize::Normal,
                0,
                false,
                None,
            );
            i32::from(clicked.0)
        },
    )?;

    Ok(())
}

fn register_bitmap_storage_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_icon",
        |mut caller: Caller<'_, HostState>, data_ptr: u32, data_len: u32| -> u32 {
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return 0;
            };
            let state = caller.data_mut();
            u32::from(state.renderer.register_icon(&data))
        },
    )?;

    linker.func_wrap(
        "env",
        "host_register_bitmap",
        |mut caller: Caller<'_, HostState>, data_ptr: u32, data_len: u32| -> u32 {
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return 0;
            };
            let state = caller.data_mut();
            u32::from(state.renderer.register_bitmap(&data))
        },
    )?;

    linker.func_wrap(
        "env",
        "host_register_bitmap_nearest",
        |mut caller: Caller<'_, HostState>, data_ptr: u32, data_len: u32| -> u32 {
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return 0;
            };
            let state = caller.data_mut();
            u32::from(state.renderer.register_bitmap_nearest(&data))
        },
    )?;

    linker.func_wrap(
        "env",
        "host_bitmap_sample",
        |caller: Caller<'_, HostState>, bitmap_id: u32, x: u32, y: u32, w: u32, h: u32| -> u32 {
            let state = caller.data();
            state
                .renderer
                .bitmap_sample(bitmap_id as u16, x, y, w, h)
                .unwrap_or(0)
        },
    )?;

    Ok(())
}

fn register_image_decode_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_decode_image",
        |mut caller: Caller<'_, HostState>,
         data_ptr: u32,
         data_len: u32,
         rgba_out_ptr: u32,
         rgba_out_cap: u32|
         -> i64 {
            let Some(image_data) = read_bytes(&caller, data_ptr, data_len) else {
                return -1;
            };

            let rgba = match decode_image_rgba_limited(&image_data) {
                Ok(rgba) => rgba,
                Err(e) => {
                    tracing::error!("host_decode_image: {e}");
                    return -1;
                }
            };
            let (w, h) = (rgba.width(), rgba.height());
            let pixels = rgba.as_raw();
            let needed = pixels.len() as u32;

            if needed <= rgba_out_cap && rgba_out_ptr != 0 {
                let memory = caller.get_export("memory").and_then(Extern::into_memory);
                if let Some(memory) = memory {
                    let data = memory.data_mut(&mut caller);
                    let start = rgba_out_ptr as usize;
                    let end = start + needed as usize;
                    if end <= data.len() {
                        data[start..end].copy_from_slice(pixels);
                    }
                }
            }

            #[expect(clippy::cast_lossless)]
            {
                ((w as i64) << 32) | (h as i64)
            }
        },
    )?;

    Ok(())
}

fn register_tree_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_submit_tree",
        |mut caller: Caller<'_, HostState>, ptr: u32, len: u32, width: u32, height: u32| {
            let Some(data) = read_bytes(&caller, ptr, len) else {
                return;
            };

            let state = caller.data_mut();
            let delta_ms = state.delta_ms;
            let frame_counter = state.frame_counter;
            state.frame_counter += 1;
            let w = width as f32;
            let h = height as f32;
            match tree::process_tree(
                &data,
                w,
                h,
                &mut state.renderer,
                &mut state.interaction,
                &mut state.modal_states,
                &mut state.scroll_states,
                &mut state.animation_states,
                &mut state.transition_states,
                frame_counter,
                delta_ms,
                &mut state.taffy,
            ) {
                Ok((tree_node, result, has_active, timings)) => {
                    let had_interaction = !result.clicks.is_empty() || !result.drags.is_empty();
                    state.tree_clicks = result.clicks;
                    state.tree_drags = result.drags;
                    state.last_timings = timings;
                    if has_active || had_interaction {
                        state.frame_requested = true;
                        state.animation_only_frame = !had_interaction;
                    }
                    state.cached_tree = Some((tree_node, w, h));
                }
                Err(e) => {
                    tracing::error!("tree processing failed: {e}");
                }
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "host_get_touch_click",
        |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, out_ptr: u32| -> i32 {
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

fn register_system_time_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_get_system_time",
        |mut caller: Caller<'_, HostState>, out_ptr: u32| {
            let now = caller.data().system_time;
            let mut buf = [0_u8; 20];
            buf[0..8].copy_from_slice(&now.timestamp().to_le_bytes());
            buf[8..12].copy_from_slice(&now.offset().local_minus_utc().to_le_bytes());
            #[expect(clippy::cast_sign_loss)]
            let year = now.year() as u16;
            buf[12..14].copy_from_slice(&year.to_le_bytes());
            buf[14] = now.month() as u8;
            buf[15] = now.day() as u8;
            buf[16] = now.hour() as u8;
            buf[17] = now.minute() as u8;
            buf[18] = now.second() as u8;
            buf[19] = now.weekday().num_days_from_monday() as u8;

            let memory = caller.get_export("memory").and_then(Extern::into_memory);
            if let Some(memory) = memory {
                let data = memory.data_mut(&mut caller);
                let start = out_ptr as usize;
                if start + 20 <= data.len() {
                    data[start..start + 20].copy_from_slice(&buf);
                }
            }
        },
    )?;

    Ok(())
}
