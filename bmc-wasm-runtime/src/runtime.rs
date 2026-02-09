// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime wrapper using wasmi.

#![expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use std::ffi::c_void;

use anyhow::Result;
use chrono::{Datelike, Local, Timelike};
use wasmi::{Caller, Extern, Linker};

use crate::components::{ButtonStyle, draw_button};
use crate::gpu::FemtoVgRenderer;
use crate::host_api::HostState;
use crate::renderer::Renderer;
use crate::tree;

/// WebAssembly widget runtime.
///
/// Executes WASM modules in a sandboxed environment with fuel metering.
/// Owns the GPU renderer inside `HostState`.
#[expect(missing_debug_implementations)]
pub struct WasmWidgetRuntime {
    store: wasmi::Store<HostState>,
    instance: wasmi::Instance,
    render_func: wasmi::TypedFunc<u32, ()>,
}

impl WasmWidgetRuntime {
    /// Maximum fuel (instructions) per frame.
    pub const FUEL_PER_FRAME: u64 = 10_000_000;

    /// Create a new runtime from WASM bytes and a GL function loader.
    ///
    /// The runtime creates and owns the GPU renderer. The host (testbed / BMC)
    /// only provides the GL context and event loop.
    ///
    /// # Safety
    /// `load_fn` must return valid OpenGL function pointers for the current GL context.
    pub unsafe fn new<F>(wasm_bytes: &[u8], load_fn: F, width: u32, height: u32) -> Result<Self>
    where
        F: FnMut(&str) -> *const c_void,
    {
        let renderer = unsafe { FemtoVgRenderer::new(load_fn, width, height) }?;
        let host_state = HostState::new(renderer);

        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = wasmi::Engine::new(&config);
        let module = wasmi::Module::new(&engine, wasm_bytes)?;

        let mut store = wasmi::Store::new(&engine, host_state);
        store.set_fuel(Self::FUEL_PER_FRAME)?;

        let mut linker = Linker::new(&engine);
        Self::register_host_functions(&mut linker)?;

        let instance = linker.instantiate(&mut store, &module)?.start(&mut store)?;

        // Get render function
        let render_func = instance.get_typed_func::<u32, ()>(&store, "render")?;

        // Call init if present
        if let Ok(init_func) = instance.get_typed_func::<(u32, u32), ()>(&store, "init") {
            init_func.call(&mut store, (width, height))?;
        }

        Ok(Self {
            store,
            instance,
            render_func,
        })
    }

    fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
        // Drawing functions — renderer accessed via state.renderer
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
                // Read string from WASM memory - copy to avoid borrow conflict
                let text_owned = {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    if let Some(memory) = memory {
                        let data = memory.data(&caller);
                        let start = text_ptr as usize;
                        let end = start + text_len as usize;
                        if end <= data.len() {
                            String::from_utf8(data[start..end].to_vec()).ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(text) = text_owned {
                    let state = caller.data_mut();
                    state
                        .renderer
                        .draw_text(&text, x as f32, y as f32, size as f32, color);
                }
            },
        )?;

        // Frame control
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
            },
        )?;

        // Interaction - styled button (draws + handles clicks)
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
                // Read key and label from WASM memory - copy to avoid borrow conflict
                let (key_owned, label_owned) = {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    if let Some(memory) = memory {
                        let data = memory.data(&caller);

                        let key = {
                            let start = key_ptr as usize;
                            let end = start + key_len as usize;
                            if end <= data.len() {
                                String::from_utf8(data[start..end].to_vec()).ok()
                            } else {
                                None
                            }
                        };

                        let label = {
                            let start = label_ptr as usize;
                            let end = start + label_len as usize;
                            if end <= data.len() {
                                String::from_utf8(data[start..end].to_vec()).ok()
                            } else {
                                None
                            }
                        };

                        (key, label)
                    } else {
                        (None, None)
                    }
                };

                if let (Some(key), Some(label)) = (key_owned, label_owned) {
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
                    );
                    return i32::from(clicked);
                }
                0
            },
        )?;

        // New tree-based API
        linker.func_wrap(
            "env",
            "host_submit_tree",
            |mut caller: Caller<'_, HostState>, ptr: u32, len: u32, width: u32, height: u32| {
                let tree_data = {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    if let Some(memory) = memory {
                        let data = memory.data(&caller);
                        let start = ptr as usize;
                        let end = start + len as usize;
                        if end <= data.len() {
                            Some(data[start..end].to_vec())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(data) = tree_data {
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
                        &mut state.animation_states,
                        &mut state.transition_states,
                        frame_counter,
                        delta_ms,
                    ) {
                        Ok((result, has_active)) => {
                            let had_clicks = result.clicks.iter().any(|&c| c);
                            state.tree_clicks = result.clicks;
                            if has_active {
                                state.frame_requested = true;
                                // Only skip WASM next frame if no clicks need processing
                                state.animation_only_frame = !had_clicks;
                            }
                            state.cached_tree_data = Some((data, w, h));
                        }
                        Err(e) => {
                            tracing::error!("tree processing failed: {e}");
                        }
                    }
                }
            },
        )?;

        linker.func_wrap(
            "env",
            "host_get_button_count",
            |caller: Caller<'_, HostState>| -> u32 { caller.data().tree_clicks.len() as u32 },
        )?;

        linker.func_wrap(
            "env",
            "host_get_click",
            |caller: Caller<'_, HostState>, index: u32| -> i32 {
                caller
                    .data()
                    .tree_clicks
                    .get(index as usize)
                    .map_or(0, |&c| i32::from(c))
            },
        )?;

        // System time — writes 20-byte struct to WASM memory at out_ptr
        linker.func_wrap(
            "env",
            "host_get_system_time",
            |mut caller: Caller<'_, HostState>, out_ptr: u32| {
                let now = Local::now();
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

    /// Render a frame. Call `renderer().begin_frame()` before and `renderer().flush()` after.
    ///
    /// On animation-only frames (no pending input, host auto-requested),
    /// skips WASM execution and re-renders from cached tree data.
    pub fn render(&mut self, delta_ms: u32) -> Result<()> {
        let state = self.store.data_mut();

        // Decide frame type BEFORE begin_frame consumes events
        let animation_only = state.animation_only_frame
            && !state.interaction.has_pending_events()
            && state.cached_tree_data.is_some();

        state.interaction.begin_frame();
        state.begin_render_frame();
        state.delta_ms = delta_ms;

        if animation_only {
            // Animation-only frame: skip WASM, re-render from cached tree
            let (data, width, height) = state.cached_tree_data.clone().unwrap();
            let frame_counter = state.frame_counter;
            state.frame_counter += 1;
            match tree::process_tree(
                &data,
                width,
                height,
                &mut state.renderer,
                &mut state.interaction,
                &mut state.modal_states,
                &mut state.animation_states,
                &mut state.transition_states,
                frame_counter,
                delta_ms,
            ) {
                Ok((result, has_active)) => {
                    state.tree_clicks = result.clicks;
                    if has_active {
                        state.frame_requested = true;
                        state.animation_only_frame = true;
                    }
                }
                Err(e) => {
                    tracing::error!("cached tree render failed: {e}");
                }
            }
        } else {
            // Full frame: run WASM
            self.store.set_fuel(Self::FUEL_PER_FRAME)?;
            self.render_func.call(&mut self.store, delta_ms)?;
        }

        Ok(())
    }

    /// Access the GPU renderer (for begin_frame, flush, and testbed drawing).
    pub fn renderer(&mut self) -> &mut FemtoVgRenderer {
        &mut self.store.data_mut().renderer
    }

    /// Check if WASM requested another frame via `request_frame()`.
    #[must_use]
    pub fn wants_next_frame(&self) -> bool {
        self.store.data().frame_requested
    }

    /// Get the delay if `request_frame_after(ms)` was called.
    #[must_use]
    pub fn next_frame_delay(&self) -> Option<u32> {
        self.store.data().frame_delay_ms
    }

    /// Push a touch event to be processed next frame.
    pub fn push_touch_event(&mut self, event: crate::interaction::TouchEvent) {
        self.store.data_mut().interaction.push_event(event);
    }

    /// Get the instance for additional exports.
    #[must_use]
    pub fn instance(&self) -> &wasmi::Instance {
        &self.instance
    }
}
