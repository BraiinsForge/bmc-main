// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime wrapper using wasmi.

#![expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use bmc_wasm_protocol::{
    FormatPreferences, NumberFormat, SDK_VERSION, SDK_VERSION_EXPORT, TemperatureUnit, UnitSystem,
    version_unpack,
};
use chrono::{DateTime, Datelike, Local, Timelike, Utc};
use formato::{FormatOptions, Formato};
use wasmi::{Caller, Extern, Linker};

use crate::components::{ButtonSize, ButtonStyle, draw_button};
use crate::gpu::FemtoVgRenderer;
use crate::host_api::{CompletedFetch, DelayedFetch, HostState};
use crate::renderer::Renderer;
use crate::tree;

/// Call the widget's `__bmc_sdk_version` export and validate against the host.
///
/// Returns the widget's `(major, minor, patch)` version on success.
/// Rejects on missing export or major version mismatch.
fn check_sdk_version(
    instance: wasmi::Instance,
    store: &mut wasmi::Store<HostState>,
) -> Result<(u16, u16, u16)> {
    let (major, minor, patch) = SDK_VERSION;

    let version_func = instance
        .get_typed_func::<(), u64>(&*store, SDK_VERSION_EXPORT)
        .map_err(|_| {
            anyhow::anyhow!(
                "widget missing '{SDK_VERSION_EXPORT}' export — \
             if using Rust SDK, update bmc-wasm-sdk; \
             otherwise export a `{SDK_VERSION_EXPORT}() -> u64` function \
             (packed major|minor<<16|patch<<32, host expects {major}.{minor}.{patch})"
            )
        })?;

    let packed = version_func.call(store, ())?;
    let widget_version = version_unpack(packed);
    let (w_major, w_minor, w_patch) = widget_version;

    if w_major != major {
        bail!(
            "SDK major version mismatch: widget is {w_major}.{w_minor}.{w_patch}, \
             host expects {major}.{minor}.{patch}"
        );
    }

    tracing::info!(
        "widget SDK version {w_major}.{w_minor}.{w_patch} \
         (host {major}.{minor}.{patch})"
    );
    Ok(widget_version)
}

/// Result of a single `render()` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    /// Frame rendered successfully within fuel budget.
    Ok,
    /// Widget exceeded its fuel budget this frame.
    /// The last good frame is shown with a warning indicator.
    FuelExhausted,
    /// Widget exceeded its budget too many times and has been killed.
    /// An error overlay is shown; WASM will not be called again
    /// until [`WasmWidgetRuntime::reset_fuel_state`] is called.
    Dead,
}

/// WebAssembly widget runtime.
///
/// Executes WASM modules in a sandboxed environment with fuel metering.
/// Owns the GPU renderer inside `HostState`.
#[expect(missing_debug_implementations)]
pub struct WasmWidgetRuntime {
    store: wasmi::Store<HostState>,
    instance: wasmi::Instance,
    render_func: wasmi::TypedFunc<u32, ()>,
    sdk_version: (u16, u16, u16),
    /// Instruction budget reset before each WASM frame execution.
    fuel_per_frame: u64,
    /// Consecutive frames that exceeded the fuel budget.
    fuel_strikes: u32,
    /// Widget permanently stopped after exceeding [`Self::max_fuel_strikes`].
    fuel_dead: bool,
    /// How many consecutive fuel-outs before the widget is killed.
    max_fuel_strikes: u32,
}

impl WasmWidgetRuntime {
    /// Maximum fuel (instructions) per frame.
    pub const FUEL_PER_FRAME: u64 = 10_000_000;

    /// Create a new runtime from WASM bytes and a GL function loader.
    ///
    /// The `fbo_id` is the OpenGL framebuffer object that FemtoVG should render to.
    /// This is typically the staging FBO from the EGL two-FBO pipeline.
    ///
    /// `fuel_per_frame` sets the instruction budget per frame. Use
    /// [`Self::FUEL_PER_FRAME`] for the default.
    ///
    /// The runtime creates and owns the GPU renderer. The host (testbed / BMC)
    /// only provides the GL context and event loop.
    ///
    /// # Safety
    /// `load_fn` must return valid OpenGL function pointers for the current GL context.
    pub unsafe fn new<F>(
        wasm_bytes: &[u8],
        load_fn: F,
        width: u32,
        height: u32,
        fbo_id: u32,
        fuel_per_frame: u64,
        prefs: FormatPreferences,
    ) -> Result<Self>
    where
        F: FnMut(&str) -> *const c_void,
    {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = wasmi::Engine::new(&config);
        let module = wasmi::Module::new(&engine, wasm_bytes)?;

        let renderer = unsafe { FemtoVgRenderer::new(load_fn, width, height, fbo_id) }?;
        let host_state = HostState::new(renderer, prefs);

        let mut store = wasmi::Store::new(&engine, host_state);
        store.set_fuel(fuel_per_frame)?;

        let mut linker = Linker::new(&engine);
        Self::register_host_functions(&mut linker)?;

        let instance = linker.instantiate_and_start(&mut store, &module)?;

        // Check SDK version before running any widget code
        let sdk_version = check_sdk_version(instance, &mut store)?;

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
            sdk_version,
            fuel_per_frame,
            fuel_strikes: 0,
            fuel_dead: false,
            max_fuel_strikes: 5,
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
                        ButtonSize::Normal,
                        0,
                    );
                    return i32::from(clicked);
                }
                0
            },
        )?;

        // Icon registration
        linker.func_wrap(
            "env",
            "host_register_icon",
            |mut caller: Caller<'_, HostState>, data_ptr: u32, data_len: u32| -> u32 {
                let icon_data = {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    if let Some(memory) = memory {
                        let data = memory.data(&caller);
                        let start = data_ptr as usize;
                        let end = start + data_len as usize;
                        if end <= data.len() {
                            Some(data[start..end].to_vec())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(data) = icon_data {
                    let state = caller.data_mut();
                    u32::from(state.renderer.register_icon(&data))
                } else {
                    0
                }
            },
        )?;

        // Bitmap registration
        linker.func_wrap(
            "env",
            "host_register_bitmap",
            |mut caller: Caller<'_, HostState>, data_ptr: u32, data_len: u32| -> u32 {
                let bitmap_data = {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    if let Some(memory) = memory {
                        let data = memory.data(&caller);
                        let start = data_ptr as usize;
                        let end = start + data_len as usize;
                        if end <= data.len() {
                            Some(data[start..end].to_vec())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(data) = bitmap_data {
                    let state = caller.data_mut();
                    u32::from(state.renderer.register_bitmap(&data))
                } else {
                    0
                }
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

        // ── Fetch ──────────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_fetch",
            |mut caller: Caller<'_, HostState>,
             url_ptr: u32,
             url_len: u32,
             headers_ptr: u32,
             headers_len: u32|
             -> u32 {
                let url = read_string(&caller, url_ptr, url_len);
                let Some(url) = url else { return 0 };
                let headers = parse_headers(&caller, headers_ptr, headers_len);

                let state = caller.data_mut();
                let request_id = state.next_request_id;
                state.next_request_id += 1;

                let tx = state.fetch_tx.clone();
                std::thread::spawn(move || {
                    let (status, body) = do_fetch(&url, &headers);
                    let _ = tx.send(CompletedFetch {
                        request_id,
                        status,
                        body,
                    });
                });

                request_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_fetch_after",
            |mut caller: Caller<'_, HostState>,
             delay_ms: u32,
             url_ptr: u32,
             url_len: u32,
             headers_ptr: u32,
             headers_len: u32|
             -> u32 {
                let url = read_string(&caller, url_ptr, url_len);
                let Some(url) = url else { return 0 };
                let headers = parse_headers(&caller, headers_ptr, headers_len);

                let state = caller.data_mut();
                let request_id = state.next_request_id;
                state.next_request_id += 1;

                let fire_at = Instant::now() + Duration::from_millis(u64::from(delay_ms));
                state.delayed_fetches.push(DelayedFetch {
                    fire_at,
                    url,
                    headers,
                    request_id,
                });

                request_id
            },
        )?;

        // ── Logging ────────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_log",
            |caller: Caller<'_, HostState>, ptr: u32, len: u32, level: u32| {
                let msg = read_string(&caller, ptr, len);
                let Some(msg) = msg else { return };
                match level {
                    0 => tracing::debug!("{msg}"),
                    1 => tracing::info!("{msg}"),
                    2 => tracing::warn!("{msg}"),
                    _ => tracing::error!("{msg}"),
                }
            },
        )?;

        // ── JSON ───────────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_json_parse",
            |mut caller: Caller<'_, HostState>, body_ptr: u32, body_len: u32| -> u32 {
                let bytes = read_bytes(&caller, body_ptr, body_len);
                let Some(bytes) = bytes else { return 0 };

                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return 0;
                };

                let state = caller.data_mut();
                let doc_id = state.next_json_id;
                state.next_json_id += 1;
                state.json_docs.insert(doc_id, value);
                doc_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_json_get_str",
            |mut caller: Caller<'_, HostState>,
             doc_id: u32,
             path_ptr: u32,
             path_len: u32,
             out_ptr: u32,
             out_len: u32|
             -> i32 {
                let path = read_string(&caller, path_ptr, path_len);
                let Some(path) = path else { return -1 };

                // Copy string out to break the borrow on caller
                let str_bytes = {
                    let state = caller.data();
                    let Some(doc) = state.json_docs.get(&doc_id) else {
                        return -1;
                    };
                    let Some(val) = doc.pointer(&path) else {
                        return -1;
                    };
                    let Some(s) = val.as_str() else {
                        return -2;
                    };
                    s.as_bytes().to_vec()
                };

                let actual_len = str_bytes.len();
                let copy_len = actual_len.min(out_len as usize);

                // Write into WASM memory
                if copy_len > 0 {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    if let Some(memory) = memory {
                        let data = memory.data_mut(&mut caller);
                        let start = out_ptr as usize;
                        if start + copy_len <= data.len() {
                            data[start..start + copy_len].copy_from_slice(&str_bytes[..copy_len]);
                        }
                    }
                }

                #[expect(clippy::cast_possible_wrap)]
                {
                    actual_len as i32
                }
            },
        )?;

        linker.func_wrap(
            "env",
            "host_json_get_i64",
            |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> i64 {
                let path = read_string(&caller, path_ptr, path_len);
                let Some(path) = path else {
                    return i64::MIN;
                };
                let state = caller.data();
                let Some(doc) = state.json_docs.get(&doc_id) else {
                    return i64::MIN;
                };
                let Some(val) = doc.pointer(&path) else {
                    return i64::MIN;
                };
                val.as_i64().unwrap_or(i64::MIN)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_json_get_f64",
            |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> f64 {
                let path = read_string(&caller, path_ptr, path_len);
                let Some(path) = path else {
                    return f64::NAN;
                };
                let state = caller.data();
                let Some(doc) = state.json_docs.get(&doc_id) else {
                    return f64::NAN;
                };
                let Some(val) = doc.pointer(&path) else {
                    return f64::NAN;
                };
                val.as_f64().unwrap_or(f64::NAN)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_json_get_bool",
            |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> i32 {
                let path = read_string(&caller, path_ptr, path_len);
                let Some(path) = path else { return -1 };
                let state = caller.data();
                let Some(doc) = state.json_docs.get(&doc_id) else {
                    return -1;
                };
                let Some(val) = doc.pointer(&path) else {
                    return -1;
                };
                match val.as_bool() {
                    Some(true) => 1,
                    Some(false) => 0,
                    None => -1,
                }
            },
        )?;

        // Parse an ISO 8601 date string → unix timestamp.
        // Returns i64::MIN on parse failure.
        linker.func_wrap(
            "env",
            "host_parse_date",
            |caller: Caller<'_, HostState>, str_ptr: u32, str_len: u32| -> i64 {
                let s = read_string(&caller, str_ptr, str_len);
                let Some(s) = s else { return i64::MIN };
                s.parse::<DateTime<Utc>>()
                    .map(|dt| dt.timestamp())
                    .unwrap_or(i64::MIN)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_json_free",
            |mut caller: Caller<'_, HostState>, doc_id: u32| {
                caller.data_mut().json_docs.remove(&doc_id);
            },
        )?;

        // ── Formatting ────────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_format_number",
            |mut caller: Caller<'_, HostState>,
             value: f64,
             decimals: u32,
             out_ptr: u32,
             out_len: u32|
             -> i32 {
                let formatted =
                    format_number_with_prefs(caller.data().prefs.number_format, value, decimals);
                write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_format_speed",
            |mut caller: Caller<'_, HostState>,
             value: f64,
             decimals: u32,
             out_ptr: u32,
             out_len: u32|
             -> i32 {
                let prefs = caller.data().prefs;
                let (converted, suffix) = match prefs.unit_system {
                    UnitSystem::Metric => (value, " km/h"),
                    UnitSystem::Imperial => (value * 0.621_371_192, " mph"),
                };
                let num = format_number_with_prefs(prefs.number_format, converted, decimals);
                let formatted = format!("{num}{suffix}");
                write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_format_temperature",
            |mut caller: Caller<'_, HostState>,
             value: f64,
             decimals: u32,
             out_ptr: u32,
             out_len: u32|
             -> i32 {
                let prefs = caller.data().prefs;
                let (converted, suffix) = match prefs.temperature_unit {
                    TemperatureUnit::Celsius => (value, " \u{00b0}C"),
                    TemperatureUnit::Fahrenheit => (value * 9.0 / 5.0 + 32.0, " \u{00b0}F"),
                };
                let num = format_number_with_prefs(prefs.number_format, converted, decimals);
                let formatted = format!("{num}{suffix}");
                write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
            },
        )?;

        Ok(())
    }

    /// Render a frame. Call `renderer().begin_frame()` before and `renderer().flush()` after.
    ///
    /// On animation-only frames (no pending input, host auto-requested),
    /// skips WASM execution and re-renders from cached tree data.
    ///
    /// Returns [`RenderStatus::FuelExhausted`] if the widget blew its budget
    /// (last good frame is shown with a warning bar). After
    /// [`Self::max_fuel_strikes`] consecutive fuel-outs the widget is killed
    /// and [`RenderStatus::Dead`] is returned on every subsequent call.
    pub fn render(&mut self, delta_ms: u32) -> Result<RenderStatus> {
        let state = self.store.data_mut();

        // Dead widget — show overlay on every frame.
        // Use `reset_fuel_state()` to revive (e.g. from a testbed button).
        if self.fuel_dead {
            state.interaction.begin_frame();
            state.begin_render_frame();
            Self::render_cached_tree(state, delta_ms);
            Self::draw_dead_overlay(state);
            return Ok(RenderStatus::Dead);
        }

        // Decide frame type BEFORE begin_frame consumes events
        let animation_only = state.animation_only_frame
            && !state.interaction.has_pending_events()
            && state.cached_tree_data.is_some();

        state.interaction.begin_frame();
        state.begin_render_frame();
        state.delta_ms = delta_ms;

        if animation_only {
            Self::render_cached_tree(state, delta_ms);
            return Ok(RenderStatus::Ok);
        }

        // Full frame: run WASM with per-frame fuel budget.
        self.store.set_fuel(self.fuel_per_frame)?;
        match self.render_func.call(&mut self.store, delta_ms) {
            Ok(()) => {
                self.fuel_strikes = 0;
                Ok(RenderStatus::Ok)
            }
            Err(e) if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) => {
                self.fuel_strikes += 1;
                tracing::warn!(
                    "widget exceeded fuel budget (strike {}/{})",
                    self.fuel_strikes,
                    self.max_fuel_strikes,
                );
                if self.fuel_strikes >= self.max_fuel_strikes {
                    self.fuel_dead = true;
                    let state = self.store.data_mut();
                    Self::render_cached_tree(state, delta_ms);
                    Self::draw_dead_overlay(state);
                    return Ok(RenderStatus::Dead);
                }
                // Show last good frame + warning bar, and request a
                // retry so the widget can run again with any state
                // changes that happened before the fuel trap.
                let state = self.store.data_mut();
                Self::render_cached_tree(state, delta_ms);
                Self::draw_fuel_warning(state, self.fuel_strikes, self.max_fuel_strikes);
                state.frame_requested = true;
                Ok(RenderStatus::FuelExhausted)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Re-render the last successfully submitted tree (no WASM execution).
    fn render_cached_tree(state: &mut HostState, delta_ms: u32) {
        let Some((data, width, height)) = state.cached_tree_data.clone() else {
            return;
        };
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
    }

    /// Subtle red bar at the top edge indicating fuel exhaustion.
    fn draw_fuel_warning(state: &mut HostState, strikes: u32, max_strikes: u32) {
        let w = state.renderer.width();
        let fraction = strikes as f32 / max_strikes as f32;
        let bar_w = w * fraction;
        // Red bar, increasingly opaque as strikes accumulate
        #[expect(clippy::cast_sign_loss)] // fraction is always 0..=1
        let alpha = (100.0 + 155.0 * fraction) as u32;
        let color = 0xFF_00_00_00 | (alpha & 0xFF);
        state.renderer.fill_rect(0.0, 0.0, bar_w, 3.0, color);
    }

    /// Full error overlay for a dead widget — CDS notification banner.
    fn draw_dead_overlay(state: &mut HostState) {
        let canvas_w = state.renderer.width();
        let canvas_h = state.renderer.height();

        let title = "This widget has been stopped";
        let subtitle = "It used too many resources and was suspended.";
        let banner_w = f32::clamp(canvas_w * 0.6, 250.0, 400.0);
        let banner_h =
            tree::measure_notification_banner(title, subtitle, banner_w, &mut state.renderer);

        // Semi-transparent dark scrim
        state
            .renderer
            .fill_rect(0.0, 0.0, canvas_w, canvas_h, 0x00_00_00_B0);

        tree::render_notification_banner(
            title,
            subtitle,
            bmc_wasm_protocol::RED_60,
            bmc_wasm_protocol::ICON_METER,
            (canvas_w - banner_w) / 2.0,
            (canvas_h - banner_h) / 2.0,
            banner_w,
            banner_h,
            &mut state.renderer,
        );
    }

    /// Reset the fuel strike counter and dead state.
    ///
    /// Call this after hot-reloading a widget or when the host wants to
    /// give the widget another chance.
    pub fn reset_fuel_state(&mut self) {
        self.fuel_strikes = 0;
        self.fuel_dead = false;
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

    /// The SDK version the widget was compiled with (major, minor, patch).
    #[must_use]
    pub fn sdk_version(&self) -> (u16, u16, u16) {
        self.sdk_version
    }

    /// The SDK version the host expects (major, minor, patch).
    #[must_use]
    pub fn host_sdk_version() -> (u16, u16, u16) {
        SDK_VERSION
    }

    /// Get the instance for additional exports.
    #[must_use]
    pub fn instance(&self) -> &wasmi::Instance {
        &self.instance
    }

    /// Check for completed fetch responses and delayed fetches, then deliver
    /// them to WASM by calling `__on_fetch_response`.
    ///
    /// Call this before `render()` each frame.
    pub fn deliver_fetch_responses(&mut self) {
        // Fire any delayed fetches whose time has come
        let now = Instant::now();
        let state = self.store.data_mut();
        let mut ready = Vec::new();
        state.delayed_fetches.retain(|df| {
            if now >= df.fire_at {
                ready.push((df.url.clone(), df.headers.clone(), df.request_id));
                false
            } else {
                true
            }
        });
        for (url, headers, request_id) in ready {
            let tx = state.fetch_tx.clone();
            std::thread::spawn(move || {
                let (status, body) = do_fetch(&url, &headers);
                let _ = tx.send(CompletedFetch {
                    request_id,
                    status,
                    body,
                });
            });
        }

        // Collect all completed responses
        let mut responses = Vec::new();
        let state = self.store.data_mut();
        while let Ok(resp) = state.fetch_rx.try_recv() {
            responses.push(resp);
        }

        if responses.is_empty() {
            return;
        }

        tracing::debug!("delivering {} fetch response(s)", responses.len());

        // Get the __on_fetch_response and __alloc exports
        let on_response = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_fetch_response");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_response), Ok(alloc_func)) = (on_response, alloc_func) else {
            tracing::warn!("widget missing __on_fetch_response or __alloc export");
            return;
        };

        for resp in responses {
            let body_len = resp.body.len() as u32;
            tracing::debug!(
                id = resp.request_id,
                status = resp.status,
                body_len,
                "delivering fetch response"
            );

            // Allocate WASM memory for the body
            let body_ptr = if body_len > 0 {
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    continue;
                }
                match alloc_func.call(&mut self.store, body_len) {
                    Ok(ptr) => {
                        // Write body into WASM memory
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + body_len as usize;
                            if end <= data.len() {
                                data[start..end].copy_from_slice(&resp.body);
                            }
                        }
                        ptr
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed: {e}");
                        continue;
                    }
                }
            } else {
                0
            };

            // Call __on_fetch_response(request_id, status, body_ptr, body_len)
            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_response.call(
                &mut self.store,
                (resp.request_id, resp.status, body_ptr, body_len),
            ) {
                tracing::error!("__on_fetch_response failed: {e}");
            }
        }
    }

    /// Whether there are pending delayed fetches that need polling.
    #[must_use]
    pub fn has_pending_fetches(&self) -> bool {
        !self.store.data().delayed_fetches.is_empty()
    }
}

/// Read a UTF-8 string from WASM memory.
fn read_string(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<String> {
    let memory = caller.get_export("memory").and_then(Extern::into_memory)?;
    let data = memory.data(caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    String::from_utf8(data[start..end].to_vec()).ok()
}

/// Read raw bytes from WASM memory.
fn read_bytes(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    let memory = caller.get_export("memory").and_then(Extern::into_memory)?;
    let data = memory.data(caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    Some(data[start..end].to_vec())
}

/// Parse newline-separated "Key: Value" headers from WASM memory.
fn parse_headers(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Vec<(String, String)> {
    if len == 0 {
        return Vec::new();
    }
    let Some(raw) = read_string(caller, ptr, len) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect()
}

/// Format a number using the given number format preference and `formato` crate.
fn format_number_with_prefs(nf: NumberFormat, value: f64, decimals: u32) -> String {
    let (group_sep, decimal_sep) = match nf {
        NumberFormat::SpaceComma => ("\u{00a0}", ","),
        NumberFormat::CommaDot => (",", "."),
        NumberFormat::DotComma => (".", ","),
        NumberFormat::SpaceDot => ("\u{00a0}", "."),
    };

    let options = FormatOptions::new()
        .with_thousands(group_sep)
        .with_decimal(decimal_sep);

    let pattern = if decimals == 0 {
        "#,##0".to_owned()
    } else {
        format!("#,##0.{}", "0".repeat(decimals as usize))
    };

    value.formato_ops(&pattern, &options)
}

/// Write a UTF-8 string into WASM memory at `out_ptr`, returning actual byte length.
/// Negative return on error (no memory export).
#[expect(clippy::cast_possible_wrap)]
fn write_to_wasm(caller: &mut Caller<'_, HostState>, s: &str, out_ptr: u32, out_len: u32) -> i32 {
    let bytes = s.as_bytes();
    let actual_len = bytes.len();
    let copy_len = actual_len.min(out_len as usize);

    if copy_len > 0 {
        let memory = caller.get_export("memory").and_then(Extern::into_memory);
        if let Some(memory) = memory {
            let data = memory.data_mut(caller);
            let start = out_ptr as usize;
            if start + copy_len <= data.len() {
                data[start..start + copy_len].copy_from_slice(&bytes[..copy_len]);
            }
        }
    }

    actual_len as i32
}

/// Perform an HTTP GET request, returning (status_code, body).
/// Returns (0, error_message) on network errors.
fn do_fetch(url: &str, headers: &[(String, String)]) -> (u32, Vec<u8>) {
    let mut req = ureq::get(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    match req.call() {
        Ok(response) => {
            let status = u32::from(response.status().as_u16());
            match response.into_body().read_to_vec() {
                Ok(body) => (status, body),
                Err(e) => (0, format!("body read error: {e}").into_bytes()),
            }
        }
        Err(e) => (0, format!("fetch error: {e}").into_bytes()),
    }
}
