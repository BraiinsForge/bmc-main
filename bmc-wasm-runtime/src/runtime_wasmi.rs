// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime wrapper using wasmi.

#![expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use std::collections::HashMap;
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
use crate::host_api::{
    ActiveHttpListener, ActiveMdnsBrowse, ActiveMdnsRegistration, ActiveSocket, ActiveSsdpSearch,
    ActiveUdpBroadcast, ActiveWebSocket, CompletedFetch, DelayedFetch, FrameTimings, HostState,
    HttpInboundRequest, HttpListenerResponse, MdnsEvent, SocketEvent, SocketOutbound, SsdpEvent,
    UdpBroadcastEvent, WsEvent, WsOutbound,
};
use crate::renderer::Renderer;
use crate::tree::{self, TouchHit};

/// Write a `TouchHit` (4×f32 LE = 16 bytes) to WASM memory at `out_ptr`.
fn write_touch_hit(caller: &mut Caller<'_, HostState>, out_ptr: u32, hit: &TouchHit) {
    let memory = caller.get_export("memory").and_then(Extern::into_memory);
    if let Some(memory) = memory {
        let data = memory.data_mut(caller);
        let start = out_ptr as usize;
        if start + 16 <= data.len() {
            data[start..start + 4].copy_from_slice(&hit.x.to_le_bytes());
            data[start + 4..start + 8].copy_from_slice(&hit.y.to_le_bytes());
            data[start + 8..start + 12].copy_from_slice(&hit.width.to_le_bytes());
            data[start + 12..start + 16].copy_from_slice(&hit.height.to_le_bytes());
        }
    }
}

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
        config.set_max_cached_stacks(4);
        // Disable Wasm proposals not used by our Rust-compiled widgets.
        // Saves validation/translation overhead.
        config.wasm_tail_call(false);
        config.wasm_multi_memory(false);
        config.wasm_memory64(false);
        config.wasm_extended_const(false);
        config.wasm_custom_page_sizes(false);
        config.wasm_wide_arithmetic(false);
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
                state.deferred_wasm_render_at =
                    Some(Instant::now() + Duration::from_millis(u64::from(delay_ms)));
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
                        false,
                        None,
                    );
                    return i32::from(clicked.0);
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

        // Bitmap registration with nearest-neighbor filtering (no bilinear interpolation).
        // For pixel-art / 9-patch skin assets.
        linker.func_wrap(
            "env",
            "host_register_bitmap_nearest",
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
                    u32::from(state.renderer.register_bitmap_nearest(&data))
                } else {
                    0
                }
            },
        )?;

        // Sample average color of a rectangular region within a registered bitmap.
        // Returns packed RGBA u32 (0xRRGGBBAA), or 0 if bitmap not found / empty region.
        linker.func_wrap(
            "env",
            "host_bitmap_sample",
            |caller: Caller<'_, HostState>,
             bitmap_id: u32,
             x: u32,
             y: u32,
             w: u32,
             h: u32|
             -> u32 {
                let state = caller.data();
                state
                    .renderer
                    .bitmap_sample(bitmap_id as u16, x, y, w, h)
                    .unwrap_or(0)
            },
        )?;

        // Decode image to RGBA pixels (for color extraction in WASM)
        linker.func_wrap(
            "env",
            "host_decode_image",
            |mut caller: Caller<'_, HostState>,
             data_ptr: u32,
             data_len: u32,
             rgba_out_ptr: u32,
             rgba_out_cap: u32|
             -> i64 {
                let image_data = read_bytes(&caller, data_ptr, data_len);
                let Some(image_data) = image_data else {
                    return -1;
                };

                let rgba = match std::panic::catch_unwind(|| {
                    image::ImageReader::new(std::io::Cursor::new(&image_data))
                        .with_guessed_format()
                        .map_err(image::ImageError::IoError)
                        .and_then(image::ImageReader::decode)
                }) {
                    Ok(Ok(img)) => img.to_rgba8(),
                    Ok(Err(e)) => {
                        tracing::error!("host_decode_image: {e}");
                        return -1;
                    }
                    Err(_) => {
                        tracing::error!("host_decode_image: decoder panicked");
                        return -1;
                    }
                };
                let (w, h) = (rgba.width(), rgba.height());
                let pixels = rgba.as_raw();
                let needed = pixels.len() as u32;

                // Write RGBA pixels to WASM memory if buffer is large enough
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

                // Return packed width/height (or just needed size if buffer too small)
                #[expect(clippy::cast_lossless)]
                {
                    ((w as i64) << 32) | (h as i64)
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
                        &mut state.scroll_states,
                        &mut state.animation_states,
                        &mut state.transition_states,
                        frame_counter,
                        delta_ms,
                        &mut state.taffy,
                    ) {
                        Ok((tree_node, result, has_active, timings)) => {
                            let had_interaction = result.clicks.iter().any(|&c| c)
                                || !result.touch_clicks.is_empty()
                                || !result.touch_drags.is_empty();
                            state.tree_clicks = result.clicks;
                            state.tree_touch_clicks = result.touch_clicks;
                            state.tree_touch_drags = result.touch_drags;
                            state.last_timings = timings;
                            if has_active || had_interaction {
                                state.frame_requested = true;
                                // Only skip WASM next frame if no interactions need processing
                                state.animation_only_frame = !had_interaction;
                            }
                            state.cached_tree = Some((tree_node, w, h));
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

        // Touch click: writes TouchHit (16 bytes: x,y,w,h as f32 LE) to out_ptr.
        // Returns 1 if clicked, 0 otherwise.
        linker.func_wrap(
            "env",
            "host_get_touch_click",
            |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, out_ptr: u32| -> i32 {
                let key = {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    memory.and_then(|m| {
                        let data = m.data(&caller);
                        let start = key_ptr as usize;
                        let end = start + key_len as usize;
                        if end <= data.len() {
                            String::from_utf8(data[start..end].to_vec()).ok()
                        } else {
                            None
                        }
                    })
                };
                let Some(key) = key else { return 0 };
                let hit = caller.data().tree_touch_clicks.get(&key).copied();
                let Some(hit) = hit else { return 0 };
                write_touch_hit(&mut caller, out_ptr, &hit);
                1
            },
        )?;

        // Touch drag: writes TouchHit (16 bytes) to out_ptr while finger is down.
        // Returns 1 if dragging, 0 otherwise.
        linker.func_wrap(
            "env",
            "host_get_touch_drag",
            |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32, out_ptr: u32| -> i32 {
                let key = {
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    memory.and_then(|m| {
                        let data = m.data(&caller);
                        let start = key_ptr as usize;
                        let end = start + key_len as usize;
                        if end <= data.len() {
                            String::from_utf8(data[start..end].to_vec()).ok()
                        } else {
                            None
                        }
                    })
                };
                let Some(key) = key else { return 0 };
                let hit = caller.data().tree_touch_drags.get(&key).copied();
                let Some(hit) = hit else { return 0 };
                write_touch_hit(&mut caller, out_ptr, &hit);
                1
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
             method_ptr: u32,
             method_len: u32,
             url_ptr: u32,
             url_len: u32,
             headers_ptr: u32,
             headers_len: u32,
             body_ptr: u32,
             body_len: u32|
             -> u32 {
                let method = read_string(&caller, method_ptr, method_len);
                let Some(method) = method else { return 0 };
                let url = read_string(&caller, url_ptr, url_len);
                let Some(url) = url else { return 0 };
                let headers = parse_headers(&caller, headers_ptr, headers_len);
                let body = read_optional_bytes(&caller, body_ptr, body_len);

                let state = caller.data_mut();
                let request_id = state.next_request_id;
                state.next_request_id += 1;

                let tx = state.fetch_tx.clone();
                state.in_flight_fetches += 1;
                std::thread::spawn(move || {
                    let (status, resp_body) = do_fetch(&method, &url, &headers, body.as_deref());
                    let _ = tx.send(CompletedFetch {
                        request_id,
                        status,
                        body: resp_body,
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
             method_ptr: u32,
             method_len: u32,
             url_ptr: u32,
             url_len: u32,
             headers_ptr: u32,
             headers_len: u32,
             body_ptr: u32,
             body_len: u32|
             -> u32 {
                let method = read_string(&caller, method_ptr, method_len);
                let Some(method) = method else { return 0 };
                let url = read_string(&caller, url_ptr, url_len);
                let Some(url) = url else { return 0 };
                let headers = parse_headers(&caller, headers_ptr, headers_len);
                let body = read_optional_bytes(&caller, body_ptr, body_len);

                let state = caller.data_mut();
                let request_id = state.next_request_id;
                state.next_request_id += 1;

                let fire_at = Instant::now() + Duration::from_millis(u64::from(delay_ms));
                state.delayed_fetches.push(DelayedFetch {
                    fire_at,
                    method,
                    url,
                    headers,
                    body,
                    request_id,
                });

                request_id
            },
        )?;

        // ── WebSocket ─────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_ws_connect",
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
                let ws_id = state.next_ws_id;
                state.next_ws_id += 1;

                let (msg_tx, msg_rx) = std::sync::mpsc::channel::<WsOutbound>();
                let (event_tx, event_rx) = std::sync::mpsc::channel::<WsEvent>();

                state
                    .websockets
                    .insert(ws_id, ActiveWebSocket { msg_tx, event_rx });

                std::thread::spawn(move || {
                    ws_background_thread(ws_id, &url, &headers, event_tx, msg_rx);
                });

                ws_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_ws_send",
            |mut caller: Caller<'_, HostState>, ws_id: u32, msg_ptr: u32, msg_len: u32| -> u32 {
                let msg = read_string(&caller, msg_ptr, msg_len);
                let Some(msg) = msg else { return 1 };

                let state = caller.data_mut();
                let ok = state
                    .websockets
                    .get(&ws_id)
                    .is_some_and(|ws| ws.msg_tx.send(WsOutbound::Text(msg)).is_ok());
                u32::from(!ok)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_ws_close",
            |mut caller: Caller<'_, HostState>, ws_id: u32| {
                let state = caller.data_mut();
                if let Some(ws) = state.websockets.remove(&ws_id) {
                    let _ = ws.msg_tx.send(WsOutbound::Close);
                }
            },
        )?;

        // ── TCP Sockets (plain) ─────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_tcp_connect",
            |mut caller: Caller<'_, HostState>, host_ptr: u32, host_len: u32, port: u32| -> u32 {
                let host = read_string(&caller, host_ptr, host_len);
                let Some(host) = host else { return 0 };

                let state = caller.data_mut();
                let socket_id = state.next_socket_id;
                state.next_socket_id += 1;

                let (write_tx, write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
                let (event_tx, event_rx) = std::sync::mpsc::channel::<SocketEvent>();

                state
                    .sockets
                    .insert(socket_id, ActiveSocket { write_tx, event_rx });

                let port = port as u16;
                std::thread::spawn(move || {
                    tcp_background_thread(socket_id, &host, port, event_tx, write_rx);
                });

                socket_id
            },
        )?;

        // ── TLS Sockets ─────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_tls_connect",
            |mut caller: Caller<'_, HostState>, host_ptr: u32, host_len: u32, port: u32| -> u32 {
                let host = read_string(&caller, host_ptr, host_len);
                let Some(host) = host else { return 0 };

                let state = caller.data_mut();
                let socket_id = state.next_socket_id;
                state.next_socket_id += 1;

                let (write_tx, write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
                let (event_tx, event_rx) = std::sync::mpsc::channel::<SocketEvent>();

                state
                    .sockets
                    .insert(socket_id, ActiveSocket { write_tx, event_rx });

                let port = port as u16;
                std::thread::spawn(move || {
                    tls_background_thread(socket_id, &host, port, event_tx, write_rx);
                });

                socket_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_socket_write",
            |mut caller: Caller<'_, HostState>,
             socket_id: u32,
             data_ptr: u32,
             data_len: u32|
             -> u32 {
                let bytes = read_bytes(&caller, data_ptr, data_len);
                let Some(bytes) = bytes else { return 1 };

                let state = caller.data_mut();
                let ok = state
                    .sockets
                    .get(&socket_id)
                    .is_some_and(|s| s.write_tx.send(SocketOutbound::Data(bytes)).is_ok());
                u32::from(!ok)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_socket_close",
            |mut caller: Caller<'_, HostState>, socket_id: u32| {
                let state = caller.data_mut();
                if let Some(sock) = state.sockets.remove(&socket_id) {
                    let _ = sock.write_tx.send(SocketOutbound::Close);
                }
            },
        )?;

        // ── mDNS ──────────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_mdns_browse",
            |mut caller: Caller<'_, HostState>, svc_types_ptr: u32, svc_types_len: u32| -> u32 {
                let raw = read_string(&caller, svc_types_ptr, svc_types_len);
                let Some(raw) = raw else { return 0 };
                let service_types: Vec<String> = raw
                    .lines()
                    .map(|l| {
                        let l = l.trim();
                        // mdns-sd requires ".local." suffix
                        if l.ends_with(".local.") {
                            l.to_owned()
                        } else {
                            format!("{l}.local.")
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
                if service_types.is_empty() {
                    return 0;
                }

                let state = caller.data_mut();
                let browse_id = state.next_mdns_browse_id;
                state.next_mdns_browse_id += 1;

                let (event_tx, event_rx) = std::sync::mpsc::channel::<MdnsEvent>();
                let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

                state
                    .mdns_browses
                    .insert(browse_id, ActiveMdnsBrowse { event_rx, stop_tx });

                std::thread::spawn(move || {
                    mdns_browse_thread(service_types, event_tx, stop_rx);
                });

                browse_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_mdns_stop",
            |mut caller: Caller<'_, HostState>, browse_id: u32| {
                let state = caller.data_mut();
                if let Some(browse) = state.mdns_browses.remove(&browse_id) {
                    let _ = browse.stop_tx.send(());
                }
            },
        )?;

        linker.func_wrap(
            "env",
            "host_mdns_register",
            |mut caller: Caller<'_, HostState>,
             svc_ptr: u32,
             svc_len: u32,
             name_ptr: u32,
             name_len: u32,
             port: u32,
             txt_ptr: u32,
             txt_len: u32|
             -> u32 {
                let svc_type = read_string(&caller, svc_ptr, svc_len);
                let name = read_string(&caller, name_ptr, name_len);
                let txt_raw = if txt_len > 0 {
                    read_string(&caller, txt_ptr, txt_len)
                } else {
                    Some(String::new())
                };
                let (Some(svc_type), Some(name), Some(txt_raw)) = (svc_type, name, txt_raw) else {
                    return 0;
                };

                // mdns-sd requires ".local." suffix
                let svc_type = if svc_type.ends_with(".local.") {
                    svc_type
                } else {
                    format!("{svc_type}.local.")
                };

                let port = port as u16;

                // Parse TXT records: newline-delimited "key=value"
                let mut properties: Vec<(String, String)> = Vec::new();
                for line in txt_raw.lines() {
                    if let Some((k, v)) = line.split_once('=') {
                        properties.push((k.trim().to_owned(), v.trim().to_owned()));
                    }
                }

                let daemon = match mdns_sd::ServiceDaemon::new() {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("mDNS daemon creation failed: {e}");
                        return 0;
                    }
                };

                let hostname = format!("{}.local.", name.replace(' ', "-"));
                let txt: HashMap<String, String> = properties.into_iter().collect();
                let info =
                    match mdns_sd::ServiceInfo::new(&svc_type, &name, &hostname, "", port, txt) {
                        Ok(info) => info,
                        Err(e) => {
                            tracing::error!("mDNS ServiceInfo creation failed: {e}");
                            return 0;
                        }
                    };
                let fullname = info.get_fullname().to_owned();

                if let Err(e) = daemon.register(info) {
                    tracing::error!("mDNS register failed: {e}");
                    return 0;
                }

                let state = caller.data_mut();
                let reg_id = state.next_mdns_reg_id;
                state.next_mdns_reg_id += 1;
                state
                    .mdns_registrations
                    .insert(reg_id, ActiveMdnsRegistration { daemon, fullname });

                reg_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_mdns_unregister",
            |mut caller: Caller<'_, HostState>, reg_id: u32| {
                let state = caller.data_mut();
                if let Some(reg) = state.mdns_registrations.remove(&reg_id) {
                    let _ = reg.daemon.unregister(&reg.fullname);
                    let _ = reg.daemon.shutdown();
                }
            },
        )?;

        // ── SSDP ──────────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_ssdp_search",
            |mut caller: Caller<'_, HostState>,
             st_ptr: u32,
             st_len: u32,
             timeout_secs: u32|
             -> u32 {
                let raw = read_string(&caller, st_ptr, st_len);
                let Some(search_target) = raw else { return 0 };
                if search_target.is_empty() {
                    return 0;
                }

                let state = caller.data_mut();
                let search_id = state.next_ssdp_search_id;
                state.next_ssdp_search_id += 1;

                let (event_tx, event_rx) = std::sync::mpsc::channel::<SsdpEvent>();
                let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

                state
                    .ssdp_searches
                    .insert(search_id, ActiveSsdpSearch { event_rx, stop_tx });

                std::thread::spawn(move || {
                    ssdp_search_thread(search_target, timeout_secs, event_tx, stop_rx);
                });

                search_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_ssdp_stop",
            |mut caller: Caller<'_, HostState>, search_id: u32| {
                let state = caller.data_mut();
                if let Some(search) = state.ssdp_searches.remove(&search_id) {
                    let _ = search.stop_tx.send(());
                }
            },
        )?;

        // ── UDP Broadcast ────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_udp_broadcast",
            |mut caller: Caller<'_, HostState>,
             port: u32,
             msg_ptr: u32,
             msg_len: u32,
             timeout_secs: u32|
             -> u32 {
                let raw = read_string(&caller, msg_ptr, msg_len);
                let Some(message) = raw else { return 0 };
                if message.is_empty() {
                    return 0;
                }

                let state = caller.data_mut();
                let broadcast_id = state.next_udp_broadcast_id;
                state.next_udp_broadcast_id += 1;

                let (event_tx, event_rx) = std::sync::mpsc::channel::<UdpBroadcastEvent>();
                let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

                state
                    .udp_broadcasts
                    .insert(broadcast_id, ActiveUdpBroadcast { event_rx, stop_tx });

                std::thread::spawn(move || {
                    udp_broadcast_thread(port, message, timeout_secs, event_tx, stop_rx);
                });

                broadcast_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_udp_broadcast_stop",
            |mut caller: Caller<'_, HostState>, broadcast_id: u32| {
                let state = caller.data_mut();
                if let Some(broadcast) = state.udp_broadcasts.remove(&broadcast_id) {
                    let _ = broadcast.stop_tx.send(());
                }
            },
        )?;

        // ── Key-Value persistence ─────────────────────────────────

        linker.func_wrap(
            "env",
            "host_kv_set",
            |mut caller: Caller<'_, HostState>,
             key_ptr: u32,
             key_len: u32,
             val_ptr: u32,
             val_len: u32| {
                let key = read_string(&caller, key_ptr, key_len);
                let val = read_bytes(&caller, val_ptr, val_len);
                let (Some(key), Some(val)) = (key, val) else {
                    return;
                };

                let state = caller.data_mut();
                state.kv_cache.insert(key.clone(), val.clone());

                // Persist to disk if path is configured
                if let Some(ref base) = state.kv_store_path {
                    let dir = base.clone();
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        tracing::warn!("kv_set: failed to create dir: {e}");
                        return;
                    }
                    let path = dir.join(&key);
                    if let Err(e) = std::fs::write(&path, &val) {
                        tracing::warn!("kv_set: failed to write {}: {e}", path.display());
                    }
                }
            },
        )?;

        linker.func_wrap(
            "env",
            "host_kv_get",
            |mut caller: Caller<'_, HostState>,
             key_ptr: u32,
             key_len: u32,
             out_ptr: u32,
             out_cap: u32|
             -> i32 {
                let key = read_string(&caller, key_ptr, key_len);
                let Some(key) = key else { return -1 };

                let state = caller.data_mut();

                // Check cache first
                if let Some(val) = state.kv_cache.get(&key) {
                    let val_len = i32::try_from(val.len()).unwrap_or(i32::MAX);
                    if out_cap > 0 && out_cap as usize >= val.len() {
                        let val = val.clone();
                        // Write to WASM memory
                        let memory = caller.get_export("memory").and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem = memory.data_mut(&mut caller);
                            let start = out_ptr as usize;
                            let end = start + val.len();
                            if end <= mem.len() {
                                mem[start..end].copy_from_slice(&val);
                            }
                        }
                    }
                    return val_len;
                }

                // Try loading from disk
                if let Some(ref base) = state.kv_store_path.clone() {
                    let path = base.join(&key);
                    if let Ok(val) = std::fs::read(&path) {
                        let val_len = i32::try_from(val.len()).unwrap_or(i32::MAX);
                        state.kv_cache.insert(key, val.clone());
                        if out_cap > 0 && out_cap as usize >= val.len() {
                            let memory = caller.get_export("memory").and_then(Extern::into_memory);
                            if let Some(memory) = memory {
                                let mem = memory.data_mut(&mut caller);
                                let start = out_ptr as usize;
                                let end = start + val.len();
                                if end <= mem.len() {
                                    mem[start..end].copy_from_slice(&val);
                                }
                            }
                        }
                        return val_len;
                    }
                }

                -1
            },
        )?;

        linker.func_wrap(
            "env",
            "host_kv_delete",
            |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32| {
                let key = read_string(&caller, key_ptr, key_len);
                let Some(key) = key else { return };

                let state = caller.data_mut();
                state.kv_cache.remove(&key);

                if let Some(ref base) = state.kv_store_path {
                    let path = base.join(&key);
                    let _ = std::fs::remove_file(&path);
                }
            },
        )?;

        // ── HTTP Listener ─────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_http_listen",
            |mut caller: Caller<'_, HostState>, port: u32| -> u32 {
                let port = port as u16;

                let state = caller.data_mut();
                let listener_id = state.next_http_listener_id;
                state.next_http_listener_id += 1;

                let (request_tx, request_rx) = std::sync::mpsc::channel::<HttpInboundRequest>();
                let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
                let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();

                std::thread::spawn(move || {
                    http_listener_thread(port, request_tx, stop_rx, port_tx);
                });

                // Wait briefly for the actual bound port
                let actual_port = port_rx.recv_timeout(Duration::from_secs(2)).unwrap_or(port);

                state.http_listeners.insert(
                    listener_id,
                    ActiveHttpListener {
                        request_rx,
                        stop_tx,
                        port: actual_port,
                    },
                );

                listener_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_http_respond",
            |mut caller: Caller<'_, HostState>,
             request_id: u32,
             status: u32,
             headers_ptr: u32,
             headers_len: u32,
             body_ptr: u32,
             body_len: u32| {
                let headers = if headers_len > 0 {
                    read_string(&caller, headers_ptr, headers_len).unwrap_or_default()
                } else {
                    String::new()
                };
                let body = if body_len > 0 {
                    read_bytes(&caller, body_ptr, body_len).unwrap_or_default()
                } else {
                    Vec::new()
                };

                let state = caller.data_mut();
                if let Some(tx) = state.http_response_txs.remove(&request_id) {
                    let _ = tx.send(HttpListenerResponse {
                        status: status as u16,
                        headers,
                        body,
                    });
                }
            },
        )?;

        linker.func_wrap(
            "env",
            "host_http_close_listener",
            |mut caller: Caller<'_, HostState>, listener_id: u32| {
                let state = caller.data_mut();
                if let Some(listener) = state.http_listeners.remove(&listener_id) {
                    let _ = listener.stop_tx.send(());
                }
            },
        )?;

        linker.func_wrap(
            "env",
            "host_http_get_port",
            |caller: Caller<'_, HostState>, listener_id: u32| -> u32 {
                let state = caller.data();
                state
                    .http_listeners
                    .get(&listener_id)
                    .map_or(0, |l| u32::from(l.port))
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

        // host_format_date(timestamp, fmt_ptr, fmt_len, out_ptr, out_len) -> i32
        // Format a unix timestamp using a chrono strftime pattern.
        linker.func_wrap(
            "env",
            "host_format_date",
            |mut caller: Caller<'_, HostState>,
             timestamp: i64,
             fmt_ptr: u32,
             fmt_len: u32,
             out_ptr: u32,
             out_len: u32|
             -> i32 {
                let fmt = read_string(&caller, fmt_ptr, fmt_len);
                let Some(fmt) = fmt else { return -1 };
                let Some(dt) = DateTime::<Utc>::from_timestamp(timestamp, 0) else {
                    return -1;
                };
                let formatted = dt.format(&fmt).to_string();
                write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_json_free",
            |mut caller: Caller<'_, HostState>, doc_id: u32| {
                caller.data_mut().json_docs.remove(&doc_id);
            },
        )?;

        // ── XML ─────────────────────────────────────────────────────

        linker.func_wrap(
            "env",
            "host_xml_parse",
            |mut caller: Caller<'_, HostState>, body_ptr: u32, body_len: u32| -> u32 {
                let bytes = read_bytes(&caller, body_ptr, body_len);
                let Some(bytes) = bytes else { return 0 };

                let Ok(xml_str) = String::from_utf8(bytes) else {
                    return 0;
                };

                // Validate that it parses as XML
                if roxmltree::Document::parse(&xml_str).is_err() {
                    return 0;
                }

                let state = caller.data_mut();
                let doc_id = state.next_xml_id;
                state.next_xml_id += 1;
                state.xml_docs.insert(doc_id, xml_str);
                doc_id
            },
        )?;

        linker.func_wrap(
            "env",
            "host_xml_get_str",
            |mut caller: Caller<'_, HostState>,
             doc_id: u32,
             path_ptr: u32,
             path_len: u32,
             out_ptr: u32,
             out_len: u32|
             -> i32 {
                let path = read_string(&caller, path_ptr, path_len);
                let Some(path) = path else { return -1 };

                let result = {
                    let state = caller.data();
                    let Some(xml_str) = state.xml_docs.get(&doc_id) else {
                        return -1;
                    };
                    let Ok(doc) = roxmltree::Document::parse(xml_str) else {
                        return -1;
                    };
                    xml_query_text(&doc, &path)
                };

                let Some(text) = result else { return -1 };
                write_to_wasm(&mut caller, &text, out_ptr, out_len)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_xml_get_f64",
            |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> f64 {
                let path = read_string(&caller, path_ptr, path_len);
                let Some(path) = path else { return f64::NAN };
                let state = caller.data();
                let Some(xml_str) = state.xml_docs.get(&doc_id) else {
                    return f64::NAN;
                };
                let Ok(doc) = roxmltree::Document::parse(xml_str) else {
                    return f64::NAN;
                };
                xml_query_text(&doc, &path)
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(f64::NAN)
            },
        )?;

        linker.func_wrap(
            "env",
            "host_xml_free",
            |mut caller: Caller<'_, HostState>, doc_id: u32| {
                caller.data_mut().xml_docs.remove(&doc_id);
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

        // ── Calendar ──────────────────────────────────────────────────

        // host_expand_rrule(input_ptr, input_len, out_ptr, out_cap) -> i32
        // Two-call pattern: out_cap=0 returns required size, else fills buffer.
        // Input: binary packed (see sdk/src/calendar.rs for wire format)
        // Output: packed i64[] LE (UTC timestamps)
        linker.func_wrap(
            "env",
            "host_expand_rrule",
            |mut caller: Caller<'_, HostState>,
             input_ptr: u32,
             input_len: u32,
             out_ptr: u32,
             out_cap: u32|
             -> i32 {
                let input_bytes = read_bytes(&caller, input_ptr, input_len);
                let Some(input_bytes) = input_bytes else {
                    return -1;
                };

                let timestamps = expand_rrule_impl(&input_bytes);
                let needed = timestamps.len() * 8;
                let needed_i32 = i32::try_from(needed).unwrap_or(i32::MAX);

                if out_cap == 0 {
                    return needed_i32;
                }

                if (out_cap as usize) < needed {
                    return needed_i32;
                }

                // Write packed i64[] LE to WASM memory
                let memory = caller.get_export("memory").and_then(Extern::into_memory);
                if let Some(memory) = memory {
                    let data = memory.data_mut(&mut caller);
                    let start = out_ptr as usize;
                    for (i, &ts) in timestamps.iter().enumerate() {
                        let offset = start + i * 8;
                        if offset + 8 <= data.len() {
                            data[offset..offset + 8].copy_from_slice(&ts.to_le_bytes());
                        }
                    }
                }
                needed_i32
            },
        )?;

        // host_tz_convert(unix_secs, tz_ptr, tz_len, out_ptr) -> i32
        // Converts UTC timestamp to wall-clock time in a named IANA timezone.
        // Output: 20-byte SystemTime struct. Returns 0 on success, -1 on error.
        linker.func_wrap(
            "env",
            "host_tz_convert",
            |mut caller: Caller<'_, HostState>,
             unix_secs: i64,
             tz_ptr: u32,
             tz_len: u32,
             out_ptr: u32|
             -> i32 {
                let tz_name = read_string(&caller, tz_ptr, tz_len);
                let Some(tz_name) = tz_name else {
                    return -1;
                };

                let buf = tz_convert_impl(unix_secs, &tz_name);
                let Some(buf) = buf else { return -1 };

                let memory = caller.get_export("memory").and_then(Extern::into_memory);
                if let Some(memory) = memory {
                    let data = memory.data_mut(&mut caller);
                    let start = out_ptr as usize;
                    if start + 20 <= data.len() {
                        data[start..start + 20].copy_from_slice(&buf);
                    }
                }
                0
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
        let mut animation_only = state.animation_only_frame
            && !state.interaction.has_pending_events()
            && state.cached_tree.is_some();

        // Check wall-clock deadline for deferred WASM render (request_frame_after).
        // Uses Instant instead of delta_ms countdown because sub-millisecond
        // frames truncate delta_ms to 0 and stall countdown-based timers.
        if let Some(deadline) = state.deferred_wasm_render_at
            && Instant::now() >= deadline
        {
            state.deferred_wasm_render_at = None;
            animation_only = false;
        }

        state.interaction.begin_frame();
        state.begin_render_frame();
        state.delta_ms = delta_ms;

        if animation_only {
            Self::render_cached_tree(state, delta_ms);
            return Ok(RenderStatus::Ok);
        }

        // Full WASM frame: compute real elapsed time since last WASM render
        // (not just the animation frame's ~0-16ms delta).
        let now = Instant::now();
        let wasm_delta = now.duration_since(state.last_wasm_render_at).as_millis() as u32;
        state.last_wasm_render_at = now;

        // Full frame: run WASM with per-frame fuel budget.
        self.store.set_fuel(self.fuel_per_frame)?;
        let wasm_t0 = Instant::now();
        match self.render_func.call(&mut self.store, wasm_delta) {
            Ok(()) => {
                self.store.data_mut().last_timings.wasm_us = wasm_t0.elapsed().as_micros() as u32;
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
    ///
    /// Calls `layout_and_render` directly on the cached `TreeNode`, skipping
    /// deserialization entirely.
    fn render_cached_tree(state: &mut HostState, delta_ms: u32) {
        let Some((ref tree_node, width, height)) = state.cached_tree else {
            return;
        };
        let frame_counter = state.frame_counter;
        state.frame_counter += 1;
        let mut timings = FrameTimings::default();
        match tree::layout_and_render(
            tree_node,
            width,
            height,
            &mut state.renderer,
            &mut state.interaction,
            &mut state.modal_states,
            &mut state.scroll_states,
            &mut state.animation_states,
            &mut state.transition_states,
            frame_counter,
            delta_ms,
            &mut timings,
            &mut state.taffy,
        ) {
            Ok((result, has_active)) => {
                state.last_timings = timings;
                let had_interaction = !result.touch_drags.is_empty();
                // No WASM execution, no deserialization on cached frames
                state.tree_clicks = result.clicks;
                state.tree_touch_clicks = result.touch_clicks;
                state.tree_touch_drags = result.touch_drags;
                if has_active || had_interaction {
                    state.frame_requested = true;
                    state.animation_only_frame = !had_interaction;
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

    /// Per-component timing breakdown from the last rendered frame.
    #[must_use]
    pub fn last_timings(&self) -> FrameTimings {
        self.store.data().last_timings
    }

    /// Whether the widget needs another frame rendered.
    ///
    /// Returns `true` after the widget calls `request_frame()` or
    /// `request_frame_after(ms)`. The host **must not** call [`Self::render`]
    /// when this returns `false` — doing so wastes CPU and GPU for an
    /// identical frame.
    ///
    /// When this returns `true`, check [`Self::next_frame_delay`] to see if
    /// the frame should be rendered immediately or after a delay.
    #[must_use]
    pub fn wants_next_frame(&self) -> bool {
        self.store.data().frame_requested
    }

    /// Delay before the next frame, if the widget used `request_frame_after(ms)`.
    ///
    /// Returns `None` for immediate frames (`request_frame()`), or `Some(ms)`
    /// for delayed frames. The host should **sleep or schedule a timer** for the
    /// delay — not busy-wait or render immediately.
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
                ready.push((
                    df.method.clone(),
                    df.url.clone(),
                    df.headers.clone(),
                    df.body.clone(),
                    df.request_id,
                ));
                false
            } else {
                true
            }
        });
        for (method, url, headers, body, request_id) in ready {
            tracing::info!(request_id, %method, %url, "firing HTTP fetch");
            let tx = state.fetch_tx.clone();
            state.in_flight_fetches += 1;
            std::thread::spawn(move || {
                let (status, resp_body) = do_fetch(&method, &url, &headers, body.as_deref());
                tracing::info!(request_id, status, body_len = resp_body.len(), %url, "fetch completed");
                let _ = tx.send(CompletedFetch {
                    request_id,
                    status,
                    body: resp_body,
                });
            });
        }

        // Collect all completed responses
        let mut responses = Vec::new();
        let state = self.store.data_mut();
        while let Ok(resp) = state.fetch_rx.try_recv() {
            state.in_flight_fetches = state.in_flight_fetches.saturating_sub(1);
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

    /// Whether there are pending or in-flight fetches that need polling.
    #[must_use]
    pub fn has_pending_fetches(&self) -> bool {
        let state = self.store.data();
        !state.delayed_fetches.is_empty() || state.in_flight_fetches > 0
    }

    /// Drain WebSocket events from all active connections and deliver them
    /// to WASM by calling `__on_ws_event(ws_id, event_type, data_ptr, data_len)`.
    ///
    /// Event types: 0 = Open, 1 = Message, 2 = Close (data_ptr/data_len carry
    /// the close code as two little-endian bytes).
    ///
    /// Call this before `render()` each frame (alongside `deliver_fetch_responses`).
    pub fn deliver_ws_messages(&mut self) -> bool {
        // Collect events from all connections, noting which ones closed
        let mut events: Vec<(u32, WsEvent)> = Vec::new();
        let mut closed_ids: Vec<u32> = Vec::new();

        let state = self.store.data_mut();
        for (&ws_id, ws) in &state.websockets {
            while let Ok(event) = ws.event_rx.try_recv() {
                let is_close = matches!(event, WsEvent::Close(_));
                events.push((ws_id, event));
                if is_close {
                    closed_ids.push(ws_id);
                }
            }
        }
        for id in &closed_ids {
            state.websockets.remove(id);
        }

        if events.is_empty() {
            return false;
        }

        tracing::debug!("delivering {} WS event(s)", events.len());

        let on_ws_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_ws_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_ws_event), Ok(alloc_func)) = (on_ws_event, alloc_func) else {
            tracing::warn!("widget missing __on_ws_event or __alloc export");
            return false;
        };

        for (ws_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                WsEvent::Open => (0, &[]),
                WsEvent::Message(bytes) => (1, bytes),
                WsEvent::Close(code) => (2, &code.to_le_bytes()),
            };

            let data_len = data.len() as u32;

            let data_ptr = if data_len > 0 {
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    continue;
                }
                match alloc_func.call(&mut self.store, data_len) {
                    Ok(ptr) => {
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem_data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + data_len as usize;
                            if end <= mem_data.len() {
                                mem_data[start..end].copy_from_slice(data);
                            }
                        }
                        ptr
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed for WS event: {e}");
                        continue;
                    }
                }
            } else {
                0
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) =
                on_ws_event.call(&mut self.store, (ws_id, event_type, data_ptr, data_len))
            {
                tracing::error!("__on_ws_event failed: {e}");
            }
        }
        true
    }

    /// Whether there are any active WebSocket connections.
    #[must_use]
    pub fn has_active_websockets(&self) -> bool {
        !self.store.data().websockets.is_empty()
    }

    /// Drain TLS socket events from all active connections and deliver them
    /// to WASM by calling `__on_socket_event(socket_id, event_type, data_ptr, data_len)`.
    ///
    /// Event types: 0 = Connected, 1 = Data, 2 = Closed (data carries u32 LE reason code).
    ///
    /// Call this before `render()` each frame.
    pub fn deliver_socket_events(&mut self) -> bool {
        let mut events: Vec<(u32, SocketEvent)> = Vec::new();
        let mut closed_ids: Vec<u32> = Vec::new();

        let state = self.store.data_mut();
        for (&socket_id, sock) in &state.sockets {
            while let Ok(event) = sock.event_rx.try_recv() {
                let is_close = matches!(event, SocketEvent::Closed(_));
                events.push((socket_id, event));
                if is_close {
                    closed_ids.push(socket_id);
                }
            }
        }
        for id in &closed_ids {
            state.sockets.remove(id);
        }

        if events.is_empty() {
            return false;
        }

        let on_socket_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_socket_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_socket_event), Ok(alloc_func)) = (on_socket_event, alloc_func) else {
            tracing::warn!("widget missing __on_socket_event or __alloc export");
            return false;
        };

        for (socket_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                SocketEvent::Connected => (0, &[]),
                SocketEvent::Data(bytes) => (1, bytes),
                SocketEvent::Closed(code) => (2, &code.to_le_bytes()),
            };

            let data_len = data.len() as u32;

            let data_ptr = if data_len > 0 {
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    continue;
                }
                match alloc_func.call(&mut self.store, data_len) {
                    Ok(ptr) => {
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem_data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + data_len as usize;
                            if end <= mem_data.len() {
                                mem_data[start..end].copy_from_slice(data);
                            }
                        }
                        ptr
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed for socket event: {e}");
                        continue;
                    }
                }
            } else {
                0
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) =
                on_socket_event.call(&mut self.store, (socket_id, event_type, data_ptr, data_len))
            {
                tracing::error!("__on_socket_event failed: {e}");
            }
        }
        true
    }

    /// Whether there are any active TLS socket connections.
    #[must_use]
    pub fn has_active_sockets(&self) -> bool {
        !self.store.data().sockets.is_empty()
    }

    /// Drain mDNS events from all active browse sessions and deliver them
    /// to WASM by calling `__on_mdns_event(browse_id, event_type, data_ptr, data_len)`.
    ///
    /// Event types: 0 = Found (data = JSON), 1 = Removed (data = name).
    ///
    /// Call this before `render()` each frame.
    pub fn deliver_mdns_events(&mut self) -> bool {
        let mut events: Vec<(u32, MdnsEvent)> = Vec::new();

        let state = self.store.data_mut();
        for (&browse_id, browse) in &state.mdns_browses {
            while let Ok(event) = browse.event_rx.try_recv() {
                events.push((browse_id, event));
            }
        }

        if events.is_empty() {
            return false;
        }

        let on_mdns_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_mdns_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_mdns_event), Ok(alloc_func)) = (on_mdns_event, alloc_func) else {
            tracing::warn!("widget missing __on_mdns_event or __alloc export");
            return false;
        };

        for (browse_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                MdnsEvent::Found(json) => (0, json.as_bytes()),
                MdnsEvent::Removed(name) => (1, name.as_bytes()),
            };

            let data_len = data.len() as u32;
            let data_ptr = if data_len > 0 {
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    continue;
                }
                match alloc_func.call(&mut self.store, data_len) {
                    Ok(ptr) => {
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem_data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + data_len as usize;
                            if end <= mem_data.len() {
                                mem_data[start..end].copy_from_slice(data);
                            }
                        }
                        ptr
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed for mdns event: {e}");
                        continue;
                    }
                }
            } else {
                0
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) =
                on_mdns_event.call(&mut self.store, (browse_id, event_type, data_ptr, data_len))
            {
                tracing::error!("__on_mdns_event failed: {e}");
            }
        }
        true
    }

    /// Whether there are any active mDNS browse sessions.
    #[must_use]
    pub fn has_active_mdns_browses(&self) -> bool {
        !self.store.data().mdns_browses.is_empty()
    }

    /// Drain SSDP events from all active search sessions and deliver them
    /// to WASM by calling `__on_ssdp_event(search_id, event_type, data_ptr, data_len)`.
    ///
    /// Event types: 0 = Found (data = JSON), 1 = Removed (data = USN).
    ///
    /// Call this before `render()` each frame.
    pub fn deliver_ssdp_events(&mut self) -> bool {
        let mut events: Vec<(u32, SsdpEvent)> = Vec::new();

        let state = self.store.data_mut();
        for (&search_id, search) in &state.ssdp_searches {
            while let Ok(event) = search.event_rx.try_recv() {
                events.push((search_id, event));
            }
        }

        if events.is_empty() {
            return false;
        }

        let on_ssdp_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_ssdp_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_ssdp_event), Ok(alloc_func)) = (on_ssdp_event, alloc_func) else {
            tracing::warn!("widget missing __on_ssdp_event or __alloc export");
            return false;
        };

        for (search_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                SsdpEvent::Found(json) => (0, json.as_bytes()),
                SsdpEvent::Removed(usn) => (1, usn.as_bytes()),
            };

            let data_len = data.len() as u32;
            let data_ptr = if data_len > 0 {
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    continue;
                }
                match alloc_func.call(&mut self.store, data_len) {
                    Ok(ptr) => {
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem_data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + data_len as usize;
                            if end <= mem_data.len() {
                                mem_data[start..end].copy_from_slice(data);
                            }
                        }
                        ptr
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed for ssdp event: {e}");
                        continue;
                    }
                }
            } else {
                0
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) =
                on_ssdp_event.call(&mut self.store, (search_id, event_type, data_ptr, data_len))
            {
                tracing::error!("__on_ssdp_event failed: {e}");
            }
        }
        true
    }

    /// Whether there are any active SSDP search sessions.
    #[must_use]
    pub fn has_active_ssdp_searches(&self) -> bool {
        !self.store.data().ssdp_searches.is_empty()
    }

    /// Drain UDP broadcast events from all active sessions and deliver them
    /// to WASM by calling `__on_udp_broadcast_event(broadcast_id, data_ptr,
    /// data_len, source_ptr, source_len)`.
    ///
    /// Call this before `render()` each frame.
    pub fn deliver_udp_broadcast_events(&mut self) -> bool {
        let mut events: Vec<(u32, UdpBroadcastEvent)> = Vec::new();

        let state = self.store.data_mut();
        for (&broadcast_id, broadcast) in &state.udp_broadcasts {
            while let Ok(event) = broadcast.event_rx.try_recv() {
                events.push((broadcast_id, event));
            }
        }

        if events.is_empty() {
            return false;
        }

        let on_udp_broadcast_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32, u32), ()>(
                &self.store,
                "__on_udp_broadcast_event",
            );
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_udp_broadcast_event), Ok(alloc_func)) = (on_udp_broadcast_event, alloc_func)
        else {
            tracing::warn!("widget missing __on_udp_broadcast_event or __alloc export");
            return false;
        };

        for (broadcast_id, event) in events {
            let UdpBroadcastEvent::Response(ref data, ref source) = event;

            let data_bytes = data.as_bytes();
            let source_bytes = source.as_bytes();

            // Allocate and copy data string
            let data_len = data_bytes.len() as u32;
            let data_ptr = if data_len > 0 {
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    continue;
                }
                match alloc_func.call(&mut self.store, data_len) {
                    Ok(ptr) => {
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem_data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + data_len as usize;
                            if end <= mem_data.len() {
                                mem_data[start..end].copy_from_slice(data_bytes);
                            }
                        }
                        ptr
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed for udp broadcast data: {e}");
                        continue;
                    }
                }
            } else {
                0
            };

            // Allocate and copy source string
            let source_len = source_bytes.len() as u32;
            let source_ptr = if source_len > 0 {
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    continue;
                }
                match alloc_func.call(&mut self.store, source_len) {
                    Ok(ptr) => {
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem_data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + source_len as usize;
                            if end <= mem_data.len() {
                                mem_data[start..end].copy_from_slice(source_bytes);
                            }
                        }
                        ptr
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed for udp broadcast source: {e}");
                        continue;
                    }
                }
            } else {
                0
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_udp_broadcast_event.call(
                &mut self.store,
                (broadcast_id, data_ptr, data_len, source_ptr, source_len),
            ) {
                tracing::error!("__on_udp_broadcast_event failed: {e}");
            }
        }
        true
    }

    /// Whether there are any active UDP broadcast sessions.
    #[must_use]
    pub fn has_active_udp_broadcasts(&self) -> bool {
        !self.store.data().udp_broadcasts.is_empty()
    }

    /// Drain inbound HTTP requests from all active listeners and deliver them
    /// to WASM by calling `__on_http_request(listener_id, request_id, method_ptr,
    /// method_len, path_ptr, path_len, headers_ptr, headers_len, body_ptr, body_len)`.
    ///
    /// Call this before `render()` each frame.
    pub fn deliver_http_requests(&mut self) -> bool {
        let mut requests: Vec<(u32, HttpInboundRequest)> = Vec::new();

        let state = self.store.data_mut();
        for (&listener_id, listener) in &state.http_listeners {
            while let Ok(req) = listener.request_rx.try_recv() {
                requests.push((listener_id, req));
            }
        }

        if requests.is_empty() {
            return false;
        }

        let on_http_request = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32, u32, u32, u32, u32, u32, u32), ()>(
                &self.store,
                "__on_http_request",
            );
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_http_request), Ok(alloc_func)) = (on_http_request, alloc_func) else {
            tracing::warn!("widget missing __on_http_request or __alloc export");
            return false;
        };

        for (listener_id, req) in requests {
            // Store the response sender so host_http_respond can find it
            self.store
                .data_mut()
                .http_response_txs
                .insert(req.request_id, req.response_tx);

            // Helper: allocate + copy bytes into WASM memory
            let mut alloc_and_copy = |data: &[u8]| -> (u32, u32) {
                let len = data.len() as u32;
                if len == 0 {
                    return (0, 0);
                }
                if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                    tracing::error!("set_fuel failed: {e}");
                    return (0, 0);
                }
                match alloc_func.call(&mut self.store, len) {
                    Ok(ptr) => {
                        let memory = self
                            .instance
                            .get_export(&self.store, "memory")
                            .and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem_data = memory.data_mut(&mut self.store);
                            let start = ptr as usize;
                            let end = start + len as usize;
                            if end <= mem_data.len() {
                                mem_data[start..end].copy_from_slice(data);
                            }
                        }
                        (ptr, len)
                    }
                    Err(e) => {
                        tracing::error!("__alloc failed for http request: {e}");
                        (0, 0)
                    }
                }
            };

            let (method_ptr, method_len) = alloc_and_copy(req.method.as_bytes());
            let (path_ptr, path_len) = alloc_and_copy(req.path.as_bytes());
            let (headers_ptr, headers_len) = alloc_and_copy(req.headers.as_bytes());
            let (body_ptr, body_len) = alloc_and_copy(&req.body);

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            let request_id = req.request_id;
            if let Err(e) = on_http_request.call(
                &mut self.store,
                (
                    listener_id,
                    request_id,
                    method_ptr,
                    method_len,
                    path_ptr,
                    path_len,
                    headers_ptr,
                    headers_len,
                    body_ptr,
                    body_len,
                ),
            ) {
                tracing::error!("__on_http_request failed: {e}");
            }
        }
        true
    }

    /// Whether there are any active HTTP listeners.
    #[must_use]
    pub fn has_active_http_listeners(&self) -> bool {
        !self.store.data().http_listeners.is_empty()
    }

    /// Set the key-value storage path for this widget.
    pub fn set_kv_store_path(&mut self, path: std::path::PathBuf) {
        self.store.data_mut().kv_store_path = Some(path);
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

/// Read optional bytes from WASM memory (returns `None` if ptr is null / len is 0).
fn read_optional_bytes(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    if ptr == 0 || len == 0 {
        return None;
    }
    read_bytes(caller, ptr, len)
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

/// Query an XML document for a text value using a simplified path syntax.
///
/// Supported patterns:
/// - `//local_name` — text content of the first element with that local name
///   (namespace-agnostic, e.g. `//title` matches `<dc:title>`)
/// - `//local_name/@attr` — attribute value on the first matching element
///   (e.g. `//res/@duration`)
fn xml_query_text(doc: &roxmltree::Document<'_>, path: &str) -> Option<String> {
    let path = path.strip_prefix("//")?;

    // Check for attribute query: "element/@attr"
    if let Some((elem_part, attr_name)) = path.split_once("/@") {
        let local = elem_part.rsplit_once(':').map_or(elem_part, |(_, l)| l);
        for node in doc.descendants() {
            if node.is_element() && node.tag_name().name() == local {
                return node.attribute(attr_name).map(String::from);
            }
        }
        return None;
    }

    // Text content query
    let local = path.rsplit_once(':').map_or(path, |(_, l)| l);
    for node in doc.descendants() {
        if node.is_element() && node.tag_name().name() == local {
            // Collect all text children
            let text: String = node
                .children()
                .filter(roxmltree::Node::is_text)
                .filter_map(|n| n.text())
                .collect();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Background thread for a single WebSocket connection.
///
/// Connects to `url` with optional extra headers, then runs a loop that
/// interleaves reading inbound messages (with a 50 ms read timeout to avoid
/// blocking forever) and draining outbound messages from `msg_rx`.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
fn ws_background_thread(
    ws_id: u32,
    url: &str,
    headers: &[(String, String)],
    event_tx: std::sync::mpsc::Sender<WsEvent>,
    msg_rx: std::sync::mpsc::Receiver<WsOutbound>,
) {
    use tungstenite::http::Request;
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, connect};

    // Connect — use the plain URL when there are no custom headers so tungstenite
    // generates the required WebSocket handshake headers automatically. When extra
    // headers are needed, build a Request from the ClientRequestUri which adds them.
    let connect_result = if headers.is_empty() {
        connect(url)
    } else {
        let uri: tungstenite::http::Uri = match url.parse() {
            Ok(u) => u,
            Err(e) => {
                tracing::error!(ws_id, "WS bad URL: {e}");
                let _ = event_tx.send(WsEvent::Close(1002));
                return;
            }
        };
        let mut request = Request::builder()
            .uri(&uri)
            .header(
                "Host",
                uri.authority()
                    .map_or_else(|| "localhost".to_owned(), ToString::to_string),
            )
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            );
        for (k, v) in headers {
            request = request.header(k.as_str(), v.as_str());
        }
        let request = match request.body(()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(ws_id, "WS bad request: {e}");
                let _ = event_tx.send(WsEvent::Close(1002));
                return;
            }
        };
        connect(request)
    };

    let (mut socket, _response) = match connect_result {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(ws_id, "WS connect failed: {e}");
            let _ = event_tx.send(WsEvent::Close(1006));
            return;
        }
    };

    // Set a short read timeout so we can periodically check for outbound messages
    // instead of blocking forever on reads.
    if let MaybeTlsStream::Plain(tcp) = socket.get_ref() {
        let _ = tcp.set_read_timeout(Some(Duration::from_millis(50)));
    }

    let _ = event_tx.send(WsEvent::Open);
    tracing::info!(ws_id, %url, "WS connected");

    loop {
        // Drain all pending outbound messages
        loop {
            match msg_rx.try_recv() {
                Ok(WsOutbound::Text(text)) => {
                    if let Err(e) = socket.send(Message::Text(text)) {
                        tracing::warn!(ws_id, "WS send error: {e}");
                        let _ = event_tx.send(WsEvent::Close(1006));
                        return;
                    }
                }
                Ok(WsOutbound::Close) => {
                    let _ = socket.close(None);
                    let _ = event_tx.send(WsEvent::Close(1000));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(WsEvent::Close(1006));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        // Read one inbound message (blocks up to 50 ms due to read timeout)
        match socket.read() {
            Ok(Message::Text(text)) => {
                if event_tx.send(WsEvent::Message(text.into_bytes())).is_err() {
                    return;
                }
            }
            Ok(Message::Binary(data)) => {
                if event_tx.send(WsEvent::Message(data.clone())).is_err() {
                    return;
                }
            }
            Ok(Message::Close(frame)) => {
                let code = frame.map_or(1000, |f| f.code.into());
                let _ = event_tx.send(WsEvent::Close(code));
                tracing::info!(ws_id, code, "WS closed by server");
                return;
            }
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Read timeout expired — no data available, loop back to check outbound
            }
            Err(e) => {
                tracing::warn!(ws_id, "WS read error: {e}");
                break;
            }
        }
    }

    let _ = event_tx.send(WsEvent::Close(1006));
    tracing::info!(ws_id, "WS background thread exiting");
}

/// Background thread for a plain TCP socket connection.
///
/// Connects to `host:port`, then loops: drain outbound writes from
/// `write_rx`, read inbound data with a 50 ms timeout.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
fn tcp_background_thread(
    socket_id: u32,
    host: &str,
    port: u16,
    event_tx: std::sync::mpsc::Sender<SocketEvent>,
    write_rx: std::sync::mpsc::Receiver<SocketOutbound>,
) {
    use std::io::{Read as _, Write as _};

    let addr = format!("{host}:{port}");
    let mut tcp = match std::net::TcpStream::connect(&addr) {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::error!(socket_id, %addr, "TCP connect failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    if let Err(e) = tcp.set_read_timeout(Some(Duration::from_millis(50))) {
        tracing::warn!(socket_id, "failed to set read timeout: {e}");
    }

    let _ = event_tx.send(SocketEvent::Connected);
    tracing::info!(socket_id, %addr, "TCP connected");

    let mut read_buf = vec![0_u8; 16_384];

    loop {
        // Drain outbound writes
        loop {
            match write_rx.try_recv() {
                Ok(SocketOutbound::Data(data)) => {
                    if let Err(e) = tcp.write_all(&data) {
                        tracing::warn!(socket_id, "TCP write error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                    if let Err(e) = tcp.flush() {
                        tracing::warn!(socket_id, "TCP flush error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                }
                Ok(SocketOutbound::Close) => {
                    let _ = event_tx.send(SocketEvent::Closed(0));
                    tracing::info!(socket_id, "TCP socket closed by widget");
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(SocketEvent::Closed(1));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        // Read inbound data (blocks up to 50 ms due to read timeout)
        match tcp.read(&mut read_buf) {
            Ok(0) => {
                let _ = event_tx.send(SocketEvent::Closed(0));
                tracing::info!(socket_id, "TCP EOF");
                return;
            }
            Ok(n) => {
                if event_tx
                    .send(SocketEvent::Data(read_buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tracing::warn!(socket_id, "TCP read error: {e}");
                let _ = event_tx.send(SocketEvent::Closed(1));
                return;
            }
        }
    }
}

/// Background thread for a single TLS socket connection.
///
/// Connects to `host:port` with TLS (skipping certificate verification for
/// self-signed certs like Chromecast), then loops: drain outbound writes from
/// `write_rx`, read inbound data with a 50 ms timeout.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
fn tls_background_thread(
    socket_id: u32,
    host: &str,
    port: u16,
    event_tx: std::sync::mpsc::Sender<SocketEvent>,
    write_rx: std::sync::mpsc::Receiver<SocketOutbound>,
) {
    use std::io::{Read as _, Write as _};
    use std::sync::Arc;

    // Build a rustls ClientConfig that skips certificate verification
    // (needed for Chromecast self-signed certs)
    let crypto_provider = rustls::crypto::ring::default_provider();
    let config = match rustls::ClientConfig::builder_with_provider(Arc::new(crypto_provider))
        .with_safe_default_protocol_versions()
    {
        Ok(builder) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
            .with_no_client_auth(),
        Err(e) => {
            tracing::error!(socket_id, "TLS config error: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    // TCP connect
    let addr = format!("{host}:{port}");
    let tcp = match std::net::TcpStream::connect(&addr) {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::error!(socket_id, %addr, "TCP connect failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let server_name = match rustls::pki_types::ServerName::try_from(host.to_owned()) {
        Ok(name) => name,
        Err(e) => {
            tracing::error!(socket_id, "invalid server name '{host}': {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    // TLS handshake
    let conn = match rustls::ClientConnection::new(Arc::new(config), server_name) {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!(socket_id, "TLS handshake setup failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let mut tls = rustls::StreamOwned::new(conn, tcp);

    // Set a short read timeout for the underlying TCP stream so we can
    // periodically check for outbound writes.
    if let Err(e) = tls.sock.set_read_timeout(Some(Duration::from_millis(50))) {
        tracing::warn!(socket_id, "failed to set read timeout: {e}");
    }

    let _ = event_tx.send(SocketEvent::Connected);
    tracing::info!(socket_id, %addr, "TLS connected");

    let mut read_buf = vec![0_u8; 16_384];

    loop {
        // Drain outbound writes
        loop {
            match write_rx.try_recv() {
                Ok(SocketOutbound::Data(data)) => {
                    if let Err(e) = tls.write_all(&data) {
                        tracing::warn!(socket_id, "TLS write error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                    if let Err(e) = tls.flush() {
                        tracing::warn!(socket_id, "TLS flush error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                }
                Ok(SocketOutbound::Close) => {
                    let _ = event_tx.send(SocketEvent::Closed(0));
                    tracing::info!(socket_id, "TLS socket closed by widget");
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(SocketEvent::Closed(1));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        // Read inbound data (blocks up to 50 ms due to read timeout)
        match tls.read(&mut read_buf) {
            Ok(0) => {
                // EOF — remote closed
                let _ = event_tx.send(SocketEvent::Closed(0));
                tracing::info!(socket_id, "TLS EOF");
                return;
            }
            Ok(n) => {
                if event_tx
                    .send(SocketEvent::Data(read_buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tracing::warn!(socket_id, "TLS read error: {e}");
                let _ = event_tx.send(SocketEvent::Closed(1));
                return;
            }
        }
    }
}

/// Certificate verifier that accepts all certificates (for self-signed
/// Chromecast devices and similar LAN services).
#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Background thread for mDNS browse sessions.
///
/// Polls all registered service type receivers and forwards resolved
/// service events as JSON to the host state.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
fn mdns_browse_thread(
    service_types: Vec<String>,
    event_tx: std::sync::mpsc::Sender<MdnsEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("mDNS daemon creation failed: {e}");
            return;
        }
    };

    let receivers: Vec<_> = service_types
        .iter()
        .filter_map(|st| match daemon.browse(st) {
            Ok(rx) => Some((st.clone(), rx)),
            Err(e) => {
                tracing::error!("mDNS browse({st}) failed: {e}");
                None
            }
        })
        .collect();

    if receivers.is_empty() {
        let _ = daemon.shutdown();
        return;
    }

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        for (_, rx) in &receivers {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        // Build JSON with service details
                        let svc_type = info.ty_domain.clone();
                        let name = info.get_fullname().to_owned();
                        let port = info.get_port();
                        // Get first address (prefer IPv4)
                        let host = info
                            .get_addresses_v4()
                            .iter()
                            .next()
                            .map(ToString::to_string)
                            .unwrap_or_default();

                        // Build TXT as JSON object
                        let txt_pairs: Vec<String> = info
                            .get_properties()
                            .iter()
                            .map(|p| {
                                let k = p.key();
                                let v = p.val_str();
                                format!("\"{}\":\"{}\"", escape_json(k), escape_json(v))
                            })
                            .collect();
                        let txt_json = format!("{{{}}}", txt_pairs.join(","));

                        let json = format!(
                            "{{\"service_type\":\"{}\",\"name\":\"{}\",\"host\":\"{}\",\"port\":{},\"txt\":{}}}",
                            escape_json(&svc_type),
                            escape_json(&name),
                            escape_json(&host),
                            port,
                            txt_json,
                        );
                        if event_tx.send(MdnsEvent::Found(json)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        if event_tx.send(MdnsEvent::Removed(fullname)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::SearchStarted(_)
                    | ServiceEvent::ServiceFound(_, _)
                    | ServiceEvent::SearchStopped(_)
                    | _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = daemon.shutdown();
}

/// Escape a string for JSON output (quotes and backslashes).
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Background thread for SSDP M-SEARCH discovery.
///
/// Sends M-SEARCH multicast requests and listens for UPnP device responses.
/// For each responding device, fetches and parses the device description XML
/// to extract control URLs, then delivers pre-parsed JSON events.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
fn ssdp_search_thread(
    search_target: String,
    timeout_secs: u32,
    event_tx: std::sync::mpsc::Sender<SsdpEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::collections::HashSet;
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

    let multicast_group = Ipv4Addr::new(239, 255, 255, 250);
    let multicast_addr = SocketAddrV4::new(multicast_group, 1900);

    // Socket for M-SEARCH (ephemeral port, receives unicast responses)
    let search_socket = match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("SSDP: failed to bind search socket: {e}");
            return;
        }
    };
    if let Err(e) = search_socket.set_read_timeout(Some(Duration::from_millis(250))) {
        tracing::error!("SSDP: failed to set search socket timeout: {e}");
        return;
    }

    // Socket for NOTIFY listener (multicast group on port 1900, receives byebye/alive).
    // Port 1900 may already be in use — that's fine, NOTIFY listener is best-effort.
    let notify_socket: Option<UdpSocket> =
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 1900))
            .ok()
            .and_then(|sock| {
                if let Err(e) = sock.join_multicast_v4(&multicast_group, &Ipv4Addr::UNSPECIFIED) {
                    tracing::warn!("SSDP: failed to join multicast group: {e}");
                    return None;
                }
                let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
                Some(sock)
            });

    let mut seen_usns: HashSet<String> = HashSet::new();
    let overall_timeout = Duration::from_secs(u64::from(timeout_secs).max(3));
    let resend_interval = Duration::from_secs(30);
    let mut last_send = Instant::now()
        .checked_sub(resend_interval)
        .expect("BUG: system clock too close to epoch for SSDP interval");

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // Send M-SEARCH periodically
        if last_send.elapsed() >= resend_interval {
            let request = format!(
                "M-SEARCH * HTTP/1.1\r\n\
                 HOST: 239.255.255.250:1900\r\n\
                 MAN: \"ssdp:discover\"\r\n\
                 MX: {timeout_secs}\r\n\
                 ST: {search_target}\r\n\r\n"
            );
            if let Err(e) = search_socket.send_to(request.as_bytes(), multicast_addr) {
                tracing::warn!("SSDP: M-SEARCH send failed: {e}");
            } else {
                tracing::debug!("SSDP: sent M-SEARCH for {search_target}");
            }
            last_send = Instant::now();
        }

        // Listen for M-SEARCH responses within the search window
        let listen_deadline = Instant::now() + overall_timeout;
        let mut buf = [0_u8; 4096];
        while Instant::now() < listen_deadline {
            if stop_rx.try_recv().is_ok() {
                return;
            }

            // Poll search socket for M-SEARCH responses
            if let Ok((n, _addr)) = search_socket.recv_from(&mut buf) {
                let response = String::from_utf8_lossy(&buf[..n]);
                if let Some(event) = ssdp_handle_response(&response, &search_target, &mut seen_usns)
                    && event_tx.send(event).is_err()
                {
                    return;
                }
            }

            // Poll notify socket for NOTIFY messages (byebye / alive)
            if let Some(ref sock) = notify_socket
                && let Ok((n, _addr)) = sock.recv_from(&mut buf)
            {
                let msg = String::from_utf8_lossy(&buf[..n]);
                if let Some(event) = ssdp_handle_notify(&msg, &search_target, &mut seen_usns)
                    && event_tx.send(event).is_err()
                {
                    return;
                }
            }
        }
    }
}

/// Background thread for UDP broadcast: sends a broadcast message and collects responses.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
fn udp_broadcast_thread(
    port: u32,
    message: String,
    timeout_secs: u32,
    event_tx: std::sync::mpsc::Sender<UdpBroadcastEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

    let broadcast_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, port as u16);

    let socket = match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("UDP broadcast: failed to bind socket: {e}");
            return;
        }
    };
    if let Err(e) = socket.set_broadcast(true) {
        tracing::error!("UDP broadcast: failed to set broadcast: {e}");
        return;
    }
    if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(250))) {
        tracing::error!("UDP broadcast: failed to set read timeout: {e}");
        return;
    }

    let resend_interval = Duration::from_secs(30);
    let listen_window = Duration::from_secs(u64::from(timeout_secs).max(3));
    let mut last_send = Instant::now()
        .checked_sub(resend_interval)
        .expect("BUG: system clock too close to epoch for UDP broadcast interval");

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // Send broadcast periodically
        if last_send.elapsed() >= resend_interval {
            if let Err(e) = socket.send_to(message.as_bytes(), broadcast_addr) {
                tracing::warn!("UDP broadcast: send failed: {e}");
            } else {
                tracing::debug!("UDP broadcast: sent to port {port}");
            }
            last_send = Instant::now();
        }

        // Listen for responses
        let deadline = Instant::now() + listen_window;
        let mut buf = [0_u8; 4096];
        while Instant::now() < deadline {
            if stop_rx.try_recv().is_ok() {
                return;
            }
            if let Ok((n, addr)) = socket.recv_from(&mut buf)
                && let Ok(data) = std::str::from_utf8(&buf[..n])
            {
                let source = addr.to_string();
                if event_tx
                    .send(UdpBroadcastEvent::Response(data.to_owned(), source))
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

/// Handle an M-SEARCH response: extract LOCATION + USN, fetch description, return Found event.
fn ssdp_handle_response(
    response: &str,
    search_target: &str,
    seen_usns: &mut std::collections::HashSet<String>,
) -> Option<SsdpEvent> {
    // Verify the ST header matches our search target — devices may respond
    // with unrelated service types (e.g. upnp:rootdevice) to any M-SEARCH.
    let st = ssdp_extract_header(response, "ST")?;
    if st != search_target {
        return None;
    }

    let location = ssdp_extract_header(response, "LOCATION")?;
    let usn = ssdp_extract_header(response, "USN")?;

    if seen_usns.contains(&usn) {
        return None;
    }
    seen_usns.insert(usn.clone());

    tracing::debug!("SSDP: discovered USN={usn} at {location}");

    if let Some(json) = ssdp_fetch_description(&location) {
        return Some(SsdpEvent::Found(json));
    }
    tracing::warn!("SSDP: failed to parse description from {location}");
    None
}

/// Handle an SSDP NOTIFY message: detect `ssdp:byebye` for removal, `ssdp:alive` for discovery.
fn ssdp_handle_notify(
    msg: &str,
    search_target: &str,
    seen_usns: &mut std::collections::HashSet<String>,
) -> Option<SsdpEvent> {
    // Only process NOTIFY messages
    if !msg.starts_with("NOTIFY") {
        return None;
    }

    let nts = ssdp_extract_header(msg, "NTS")?;
    let usn = ssdp_extract_header(msg, "USN")?;
    let nt = ssdp_extract_header(msg, "NT").unwrap_or_default();

    // Only process events matching our search target
    if !nt.contains(search_target) && !usn.contains(search_target) {
        return None;
    }

    if nts == "ssdp:byebye" {
        tracing::debug!("SSDP: byebye USN={usn}");
        seen_usns.remove(&usn);
        Some(SsdpEvent::Removed(usn))
    } else if nts == "ssdp:alive" {
        // Treat as discovery if not already seen
        let location = ssdp_extract_header(msg, "LOCATION")?;
        if seen_usns.contains(&usn) {
            return None;
        }
        seen_usns.insert(usn.clone());
        tracing::debug!("SSDP: alive USN={usn} at {location}");
        if let Some(json) = ssdp_fetch_description(&location) {
            return Some(SsdpEvent::Found(json));
        }
        tracing::warn!("SSDP: failed to parse description from {location}");
        None
    } else {
        None
    }
}

/// Extract a header value from an SSDP HTTP-like response (case-insensitive).
fn ssdp_extract_header(response: &str, header_name: &str) -> Option<String> {
    let header_lower = header_name.to_ascii_lowercase();
    for line in response.lines() {
        if let Some((key, value)) = line.split_once(':')
            && key.trim().to_ascii_lowercase() == header_lower
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// Fetch a UPnP device description XML and extract relevant fields as JSON.
fn ssdp_fetch_description(location: &str) -> Option<String> {
    let response = ureq::get(location).call().ok()?;
    let body = response.into_body().read_to_string().ok()?;
    let doc = roxmltree::Document::parse(&body).ok()?;
    let root = doc.root_element();

    // Extract friendlyName from <device>
    let device_elem = root.descendants().find(|n| n.has_tag_name("device"))?;
    let friendly_name = device_elem
        .descendants()
        .find(|n| n.has_tag_name("friendlyName"))
        .and_then(|n| n.text())
        .unwrap_or("Unknown");

    // Extract control URLs from <serviceList>
    let mut av_transport_path = String::new();
    let mut rendering_control_path = String::new();

    for service in device_elem
        .descendants()
        .filter(|n| n.has_tag_name("service"))
    {
        let svc_type = service
            .descendants()
            .find(|n| n.has_tag_name("serviceType"))
            .and_then(|n| n.text())
            .unwrap_or("");
        let control_url = service
            .descendants()
            .find(|n| n.has_tag_name("controlURL"))
            .and_then(|n| n.text())
            .unwrap_or("");

        if svc_type.contains("AVTransport") {
            control_url.clone_into(&mut av_transport_path);
        } else if svc_type.contains("RenderingControl") {
            control_url.clone_into(&mut rendering_control_path);
        }
    }

    // Extract host and port from the LOCATION URL
    // Format: http://host:port/path
    let url_body = location.strip_prefix("http://")?;
    let host_port = url_body.split('/').next()?;
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse::<u16>().ok()?)
    } else {
        (host_port, 80)
    };

    let json = format!(
        "{{\"usn\":\"\",\"location\":\"{}\",\"name\":\"{}\",\"host\":\"{}\",\"port\":{},\"av_transport_path\":\"{}\",\"rendering_control_path\":\"{}\"}}",
        escape_json(location),
        escape_json(friendly_name),
        escape_json(host),
        port,
        escape_json(&av_transport_path),
        escape_json(&rendering_control_path),
    );

    Some(json)
}

/// Background thread for an HTTP listener.
///
/// Accepts connections, parses simple HTTP/1.1 requests, and sends them
/// to the WASM runtime for processing. Responses come back via a per-request
/// channel stored in `HostState::http_response_txs`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
fn http_listener_thread(
    port: u16,
    request_tx: std::sync::mpsc::Sender<HttpInboundRequest>,
    stop_rx: std::sync::mpsc::Receiver<()>,
    port_report_tx: std::sync::mpsc::Sender<u16>,
) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("HTTP listener bind failed on port {port}: {e}");
            let _ = port_report_tx.send(0);
            return;
        }
    };
    listener
        .set_nonblocking(true)
        .expect("BUG: set_nonblocking failed");

    let actual_port = listener.local_addr().map_or(port, |a| a.port());
    let _ = port_report_tx.send(actual_port);
    tracing::info!("HTTP listener started on port {actual_port}");

    let mut next_req_id: u32 = 1;

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((mut stream, addr)) => {
                tracing::debug!("HTTP connection from {addr}");
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

                // Parse HTTP/1.1 request (simple line-based)
                let mut reader = BufReader::new(&stream);

                // Request line: METHOD PATH HTTP/1.1
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
                if parts.len() < 2 {
                    continue;
                }
                let method = parts[0].to_owned();
                let path = parts[1].to_owned();

                // Headers
                let mut headers = String::new();
                let mut content_length: usize = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                        break;
                    }
                    if let Some(val) = line
                        .to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().to_owned())
                    {
                        content_length = val.parse().unwrap_or(0);
                    }
                    headers.push_str(&line);
                }

                // Body
                let mut body = vec![0_u8; content_length];
                if content_length > 0 {
                    let _ = reader.read_exact(&mut body);
                }

                let request_id = next_req_id;
                next_req_id += 1;

                // Create a response channel for this request. The sender goes
                // with the request to the WASM runtime; the receiver stays here
                // so we can block until WASM responds.
                let (resp_tx, resp_rx) = std::sync::mpsc::channel::<HttpListenerResponse>();

                let req = HttpInboundRequest {
                    request_id,
                    method,
                    path,
                    headers,
                    body,
                    response_tx: resp_tx,
                };

                if request_tx.send(req).is_err() {
                    break; // Listener was shut down
                }

                // Wait for WASM to send a response (with timeout)
                if let Ok(resp) = resp_rx.recv_timeout(Duration::from_secs(10)) {
                    let status_text = match resp.status {
                        204 => "No Content",
                        400 => "Bad Request",
                        404 => "Not Found",
                        500 => "Internal Server Error",
                        _ => "OK",
                    };
                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n{}\r\n",
                        resp.status,
                        status_text,
                        resp.body.len(),
                        resp.headers,
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&resp.body);
                    let _ = stream.flush();
                } else {
                    let response = "HTTP/1.1 504 Gateway Timeout\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                tracing::error!("HTTP listener accept error: {e}");
                break;
            }
        }
    }
    tracing::info!("HTTP listener stopped on port {actual_port}");
}

/// Perform an HTTP request, returning (status_code, body).
/// Returns (0, error_message) on network errors.
fn do_fetch(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> (u32, Vec<u8>) {
    // Methods that accept a body (POST, PUT, PATCH) vs. bodyless (GET, DELETE, HEAD)
    let result = match method {
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method {
                "POST" => ureq::post(url),
                "PUT" => ureq::put(url),
                _ => ureq::patch(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            match body {
                Some(bytes) => req.send(bytes),
                None => req.send_empty(),
            }
        }
        _ => {
            let mut req = match method {
                "DELETE" => ureq::delete(url),
                "HEAD" => ureq::head(url),
                _ => ureq::get(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.call()
        }
    };
    match result {
        Ok(response) => {
            let status = u32::from(response.status().as_u16());
            match response.into_body().read_to_vec() {
                Ok(body) => (status, body),
                Err(e) => (0, format!("body read error: {e}").into_bytes()),
            }
        }
        Err(ureq::Error::StatusCode(code)) => {
            // ureq 3 returns HTTP 4xx/5xx as Err(StatusCode) — pass through
            (u32::from(code), Vec::new())
        }
        Err(_) => (0, Vec::new()),
    }
}

// ── Calendar host functions ─────────────────────────────────────────

/// Expand an RRULE string into concrete UTC timestamps.
///
/// Input is a binary-packed buffer (see `sdk/src/calendar.rs` for wire format):
/// ```text
/// window_start: i64 LE, window_end: i64 LE, max_count: u16 LE,
/// tzid_len: u16 LE, tzid: [u8], dtstart_len: u16 LE, dtstart: [u8],
/// rrule_len: u16 LE, rrule: [u8]
/// ```
fn expand_rrule_impl(input: &[u8]) -> Vec<i64> {
    use rrule::RRuleSet;
    use std::fmt::Write;

    if input.len() < 18 {
        tracing::warn!("expand_rrule: input too short ({} bytes)", input.len());
        return Vec::new();
    }

    let window_start = i64::from_le_bytes(input[0..8].try_into().unwrap_or_default());
    let window_end = i64::from_le_bytes(input[8..16].try_into().unwrap_or_default());
    let max_count = u16::from_le_bytes(input[16..18].try_into().unwrap_or_default());

    let mut pos = 18;
    let read_str = |pos: &mut usize| -> Option<&str> {
        if *pos + 2 > input.len() {
            return None;
        }
        let len = u16::from_le_bytes(input[*pos..*pos + 2].try_into().ok()?) as usize;
        *pos += 2;
        if *pos + len > input.len() {
            return None;
        }
        let s = core::str::from_utf8(&input[*pos..*pos + len]).ok()?;
        *pos += len;
        Some(s)
    };

    let Some(tzid_raw) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read tzid");
        return Vec::new();
    };
    let tzid = if tzid_raw.is_empty() {
        None
    } else {
        Some(tzid_raw)
    };
    let Some(dtstart_str) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read dtstart");
        return Vec::new();
    };
    let Some(rrule_str) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read rrule");
        return Vec::new();
    };

    // Build an RFC-style RRULE string that the rrule crate can parse
    let mut rrule_input = String::with_capacity(256);

    // DTSTART line
    if let Some(tz) = tzid {
        let _ = writeln!(rrule_input, "DTSTART;TZID={tz}:{dtstart_str}");
    } else if dtstart_str.ends_with('Z') {
        let _ = writeln!(rrule_input, "DTSTART:{dtstart_str}");
    } else {
        // Assume UTC if no timezone
        let _ = writeln!(rrule_input, "DTSTART:{dtstart_str}Z");
    }

    // RRULE line
    let _ = write!(rrule_input, "RRULE:{rrule_str}");

    let rrule_set: RRuleSet = match rrule_input.parse() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("expand_rrule: failed to parse RRULE: {e}");
            return Vec::new();
        }
    };

    // Constrain to the window so the iterator skips directly past
    // occurrences before window_start instead of expanding from DTSTART
    // forward (which could be years of weekly events).
    let Some(after) = DateTime::from_timestamp(window_start, 0) else {
        return Vec::new();
    };
    let Some(before) = DateTime::from_timestamp(window_end, 0) else {
        return Vec::new();
    };
    let after = after.with_timezone(&rrule::Tz::UTC);
    let before = before.with_timezone(&rrule::Tz::UTC);

    let result = rrule_set.after(after).before(before).all(max_count);

    result.dates.into_iter().map(|dt| dt.timestamp()).collect()
}

/// Convert a UTC unix timestamp to wall-clock time in a named IANA timezone.
/// Returns the 20-byte SystemTime wire format, or `None` on error.
fn tz_convert_impl(unix_secs: i64, tz_name: &str) -> Option<[u8; 20]> {
    use chrono::{Datelike, TimeZone, Timelike};

    use chrono::Offset;

    let dt_utc = DateTime::from_timestamp(unix_secs, 0)?;

    // "Local" is a special case — use the system's local timezone
    let (year, month, day, hour, minute, second, weekday, utc_offset) = if tz_name == "Local" {
        let local = dt_utc.with_timezone(&Local);
        (
            local.year(),
            local.month(),
            local.day(),
            local.hour(),
            local.minute(),
            local.second(),
            local.weekday().num_days_from_monday(),
            local.offset().local_minus_utc(),
        )
    } else {
        let tz: chrono_tz::Tz = tz_name.parse().ok()?;
        let local = tz.from_utc_datetime(&dt_utc.naive_utc());
        (
            local.year(),
            local.month(),
            local.day(),
            local.hour(),
            local.minute(),
            local.second(),
            local.weekday().num_days_from_monday(),
            local.offset().fix().local_minus_utc(),
        )
    };

    let mut buf = [0_u8; 20];
    buf[0..8].copy_from_slice(&unix_secs.to_le_bytes());
    buf[8..12].copy_from_slice(&utc_offset.to_le_bytes());
    #[expect(clippy::cast_sign_loss)]
    let y = year as u16;
    buf[12..14].copy_from_slice(&y.to_le_bytes());
    buf[14] = month as u8;
    buf[15] = day as u8;
    buf[16] = hour as u8;
    buf[17] = minute as u8;
    buf[18] = second as u8;
    buf[19] = weekday as u8;

    Some(buf)
}
