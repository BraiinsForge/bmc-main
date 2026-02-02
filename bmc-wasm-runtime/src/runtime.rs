// Copyright (C) 2025  Braiins Systems s.r.o.

//! WASM runtime wrapper using wasmi.

#![expect(clippy::too_many_lines)]

use anyhow::Result;
use wasmi::{Caller, Extern, Linker};

use crate::components::{ButtonStyle, draw_button};
use crate::drawing::shapes::{draw_rounded_rect, fill_rect};
use crate::drawing::text::draw_text;
use crate::host_api::HostState;

/// WebAssembly widget runtime.
///
/// Executes WASM modules in a sandboxed environment with fuel metering.
#[expect(missing_debug_implementations)]
pub struct WasmWidgetRuntime {
    store: wasmi::Store<HostState>,
    instance: wasmi::Instance,
    render_func: wasmi::TypedFunc<u32, ()>,
}

impl WasmWidgetRuntime {
    /// Maximum fuel (instructions) per frame.
    pub const FUEL_PER_FRAME: u64 = 10_000_000;

    /// Create a new runtime from WASM bytes.
    pub fn new(wasm_bytes: &[u8], width: u32, height: u32) -> Result<Self> {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = wasmi::Engine::new(&config);
        let module = wasmi::Module::new(&engine, wasm_bytes)?;

        let host_state = HostState::new(width, height)?;
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
        // Drawing functions
        linker.func_wrap(
            "env",
            "host_fill_rect",
            |mut caller: Caller<'_, HostState>, x: i32, y: i32, w: u32, h: u32, color: u32| {
                let state = caller.data_mut();
                fill_rect(&mut state.pixmap, x, y, w, h, color);
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
                draw_rounded_rect(&mut state.pixmap, x, y, w, h, radius, color);
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
                    draw_text(
                        &mut state.pixmap,
                        &mut state.font_system,
                        &mut state.swash_cache,
                        &text,
                        x,
                        y,
                        size,
                        color,
                    );
                }
            },
        )?;

        // Frame control
        linker.func_wrap(
            "env",
            "host_request_frame",
            |mut caller: Caller<'_, HostState>| {
                caller.data_mut().frame_requested = true;
            },
        )?;

        linker.func_wrap(
            "env",
            "host_request_frame_after",
            |mut caller: Caller<'_, HostState>, delay_ms: u32| {
                let state = caller.data_mut();
                state.frame_requested = true;
                state.frame_delay_ms = Some(delay_ms);
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
                        &mut state.pixmap,
                        &mut state.font_system,
                        &mut state.swash_cache,
                        &mut state.interaction,
                        &key,
                        &label,
                        x,
                        y,
                        w,
                        h,
                        ButtonStyle::from(style),
                    );
                    return i32::from(clicked);
                }
                0
            },
        )?;

        Ok(())
    }

    /// Render a frame. Host auto-clears before and auto-commits after.
    pub fn render(&mut self, delta_ms: u32) -> Result<()> {
        // Clear overlay
        self.store.data_mut().clear_overlay();

        // Reset fuel budget
        self.store.set_fuel(Self::FUEL_PER_FRAME)?;

        // Call WASM render
        self.render_func.call(&mut self.store, delta_ms)?;

        Ok(())
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

    /// Get the rendered overlay pixmap.
    #[must_use]
    pub fn get_overlay(&self) -> &tiny_skia::Pixmap {
        &self.store.data().pixmap
    }

    /// Get the instance for additional exports.
    #[must_use]
    pub fn instance(&self) -> &wasmi::Instance {
        &self.instance
    }
}
