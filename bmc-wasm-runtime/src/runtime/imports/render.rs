// Copyright (C) 2026  Braiins Systems s.r.o.

//! Render- and interaction-focused guest imports.

#![expect(clippy::cast_precision_loss)]

mod assets;

use anyhow::Result;
use wasmi::{Caller, Linker};

use crate::components::{ButtonSize, ButtonStyle, draw_button};
use crate::host_api::HostState;
use crate::renderer::Renderer;
use crate::tree;

use super::super::backend::write_touch_hit;
use super::super::memory::{read_bytes, read_string};

fn clamp_animation_frame_delay(frame_delay_ms: Option<u32>, cadence_ms: u32) -> u32 {
    frame_delay_ms.map_or(cadence_ms, |delay_ms| delay_ms.min(cadence_ms))
}

fn requested_frame_delay(delay_ms: u32, has_active_animations: bool, cadence_ms: u32) -> u32 {
    if has_active_animations {
        delay_ms.min(cadence_ms)
    } else {
        delay_ms
    }
}

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
            state.frame_schedule.frame_requested = true;
            state.frame_schedule.animation_only_frame = false;
        },
    )?;

    linker.func_wrap(
        "env",
        "host_request_frame_after",
        |mut caller: Caller<'_, HostState>, delay_ms: u32| {
            let state = caller.data_mut();
            state.frame_schedule.frame_requested = true;
            state.frame_schedule.frame_delay_ms = Some(requested_frame_delay(
                delay_ms,
                state.frame_schedule.has_active_animations,
                state.frame_schedule.animation_frame_delay_ms,
            ));
            state.frame_schedule.animation_only_frame = false;
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
                        state.frame_schedule.frame_requested = true;
                        state.frame_schedule.animation_only_frame = !had_interaction;
                    }
                    state.frame_schedule.has_active_animations = has_active;
                    if has_active {
                        state.frame_schedule.frame_delay_ms = Some(clamp_animation_frame_delay(
                            state.frame_schedule.frame_delay_ms,
                            state.frame_schedule.animation_frame_delay_ms,
                        ));
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

#[cfg(test)]
mod tests {
    use super::{clamp_animation_frame_delay, requested_frame_delay};

    #[test]
    fn request_after_tree_submission_clamps_host_wake_to_animation_cadence() {
        assert_eq!(requested_frame_delay(1_000, true, 33), 33);
    }

    #[test]
    fn request_after_tree_submission_preserves_shorter_widget_delay() {
        assert_eq!(requested_frame_delay(16, true, 33), 16);
    }

    #[test]
    fn request_before_tree_submission_clamps_existing_host_wake_to_animation_cadence() {
        assert_eq!(clamp_animation_frame_delay(Some(1_000), 33), 33);
    }

    #[test]
    fn animation_only_wake_uses_cadence_when_no_widget_delay_exists() {
        assert_eq!(clamp_animation_frame_delay(None, 33), 33);
    }
}
