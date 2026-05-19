// Copyright (C) 2026  Braiins Systems s.r.o.

//! Run subcommand — headless capture of a single widget at a given size.

use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::io::IsTerminal;
#[cfg(target_os = "linux")]
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use glow::HasContext;
#[cfg(target_os = "linux")]
use glutin::config::ConfigTemplateBuilder;
#[cfg(target_os = "linux")]
use glutin::context::{ContextApi, ContextAttributesBuilder};
#[cfg(target_os = "linux")]
use glutin::display::{Display, GetGlDisplay};
#[cfg(target_os = "linux")]
use glutin::prelude::*;
#[cfg(target_os = "linux")]
use glutin::surface::{PbufferSurface, SurfaceAttributesBuilder};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::interaction::TouchEvent;
use bmc_render::renderer::Renderer;
use bmc_wasm_protocol::{MdnsBrowseId, SocketId, SsdpSearchId, UdpBroadcastId, WebsocketId};
use bmc_wasm_runtime::capture_config::CaptureConfig;
use bmc_wasm_runtime::unified_fixture::{
    TimelineEvent, UnifiedEvent, UnifiedFixture, load_unified_fixture, validate_fixture,
};
use bmc_wasm_runtime::{
    FixtureEvent, FixtureEventKind, RenderStatus, RuntimeConfig, WasmWidgetRuntime,
};

/// Fixed timestep per frame (ms).
const DELTA_MS: u32 = 16;

/// Number of frames for a synthetic drag gesture.
const DRAG_FRAMES: u32 = 10;

// ── Public interface ────────────────────────────────────────────────

/// Arguments passed from the CLI `run` subcommand.
pub struct RunArgs {
    pub wasm_path: PathBuf,
    pub size: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub fixture: Option<PathBuf>,
    pub variant: Option<String>,
    pub list_variants: bool,
    /// Path to the `capture/` directory containing `config.toml` and fixtures.
    pub capture_dir: PathBuf,
}

/// Parsed capture parameters (size string → width/height/name).
struct CaptureCtx {
    wasm_path: PathBuf,
    width: u32,
    height: u32,
    size_name: String,
    output_dir: PathBuf,
    fixture: Option<PathBuf>,
    variant: Option<String>,
}

pub fn execute(args: RunArgs) -> Result<()> {
    let config = bmc_wasm_runtime::capture_config::load_from_capture_dir(&args.capture_dir)?;

    // --list-variants: print variant names and exit
    if args.list_variants {
        if config.variants.is_empty() {
            println!("_default");
        } else {
            for v in &config.variants {
                println!("{}", v.name);
            }
        }
        return Ok(());
    }

    // Parse size string (required for actual capture)
    let size_str = args.size.context("--size=<WxH> is required")?;
    let output_dir = args.output_dir.context("--output=<dir> is required")?;
    let (w, h) = size_str
        .split_once('x')
        .context("--size must be WxH (e.g. 1280x480)")?;
    let width: u32 = w.parse().context("invalid width")?;
    let height: u32 = h.parse().context("invalid height")?;

    let size_name =
        bmc_wasm_runtime::capture_config::size_name_from_dimensions(width, height).to_owned();

    // Check if this size is in the allowed list (empty = all sizes allowed)
    if !config.sizes.is_empty() && !config.sizes.contains(&size_name) {
        eprintln!("Skipping size '{size_name}' (not in capture.toml sizes)");
        return Ok(());
    }

    let ctx = CaptureCtx {
        wasm_path: args.wasm_path,
        width,
        height,
        size_name,
        output_dir,
        fixture: args.fixture,
        variant: args.variant,
    };

    run_capture(&ctx, &config)
}

// ── Capture entry point ─────────────────────────────────────────────

fn run_capture(ctx: &CaptureCtx, config: &CaptureConfig) -> Result<()> {
    // Look up fixture path from CLI or config
    let fixture_path = ctx
        .fixture
        .clone()
        .or_else(|| config.fixtures.get(&ctx.size_name).cloned())
        .with_context(|| {
            // Try to extract the example name from the WASM path for a helpful hint.
            let example_name = bmc_wasm_runtime::fixtures::find_widget_root(&ctx.wasm_path)
                .and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "<name>".into());
            format!(
                "no unified fixture for size '{}' — record one with: make record EXAMPLE={} SIZE={}",
                ctx.size_name, example_name, ctx.size_name
            )
        })?;
    run_unified_capture(ctx, config, &fixture_path)
}

// ── Unified fixture replay ──────────────────────────────────────────

/// Run capture using a unified fixture file — the new replay path.
///
/// Loads and validates the fixture, extracts fetch interceptors and network
/// events, seeds KV from the fixture header, then advances virtual time
/// frame-by-frame dispatching events at their `at_ms` timestamps.
#[expect(
    clippy::too_many_lines,
    clippy::integer_division,
    clippy::cast_precision_loss,
    reason = "single-flow replay routine; splitting purely for line count would obscure the \
              event-cursor / time-cursor coupling, and the precision-loss casts are \
              capture-step math on small bounded integers"
)]
fn run_unified_capture(
    ctx: &CaptureCtx,
    config: &CaptureConfig,
    fixture_path: &Path,
) -> Result<()> {
    let fixture = load_unified_fixture(fixture_path)
        .with_context(|| format!("failed to load fixture {}", fixture_path.display()))?;
    validate_fixture(&fixture)?;

    let widget_name = ctx
        .wasm_path
        .file_stem()
        .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
    eprintln!(
        "Unified replay: {} ({} events) for {widget_name} at {}x{}",
        fixture_path.display(),
        fixture.events.len(),
        ctx.width,
        ctx.height
    );

    // Parse start time from fixture header (overrides config/CLI).
    // Must include timezone offset (e.g. 2026-03-10T18:00:00+02:00).
    let mut system_time =
        chrono::DateTime::parse_from_str(&fixture.header.time, "%Y-%m-%dT%H:%M:%S%:z")
            .with_context(|| {
                format!(
                    "invalid time '{}' in fixture header — must include timezone (e.g. 2026-03-10T18:00:00+02:00)",
                    fixture.header.time
                )
            })?;

    // Prepare KV directory — seed from fixture header KV, not secrets.ini
    let kv_dir = prepare_unified_kv_dir(ctx, config, &widget_name, &fixture);

    // Extract fetch interceptors and network events from the unified timeline
    let (fetch_interceptor, network_events) = split_unified_events(&fixture);

    // Initial params snapshot — baked into the fixture header so replay is fully
    // self-contained (no `manifest.json` lookup at replay time, which used to walk up
    // from the wasm binary and silently returned an empty map in CI's nix-build
    // sandbox where the wasm artifact is divorced from its source tree).
    let initial_params = bmc_wasm_runtime::parse_params_json(&fixture.header.initial_params)
        .expect("BUG: capture fixture initial_params must be valid");

    // Build runtime config
    let mut rt_config = RuntimeConfig {
        kv_store_path: Some(kv_dir),
        mesh_msaa_samples: 4,
        rng_seed: Some(42),
        params: initial_params,
        ..RuntimeConfig::default()
    };
    if !fetch_interceptor.is_empty() {
        let fetches = std::sync::Arc::new(fetch_interceptor);
        rt_config.fetch_interceptor = Some(Box::new(move |method, url| {
            let key = format!("{method} {url}");
            fetches.get(&key).map(|fix| (fix.status, fix.body.clone()))
        }));
    }
    rt_config.event_fixtures = network_events;

    let (gl, fbo, _keep_alive, mut renderer, mut runtime) = setup_gl_and_runtime(ctx, rt_config)?;

    let (major, minor, patch) = runtime.sdk_version();
    eprintln!(
        "Capturing {widget_name} at {}x{} (SDK {major}.{minor}.{patch})",
        ctx.width, ctx.height
    );

    // ── Main replay loop ────────────────────────────────────────────
    //
    // Collect user-action events (Capture, Click, Scroll, Drag) with their
    // timestamps.  Network events and fetches are handled by the runtime's
    // inject_fixture_events and fetch_interceptor respectively.
    let user_events: Vec<&TimelineEvent> = fixture
        .events
        .iter()
        .filter(|e| {
            matches!(
                e.event,
                UnifiedEvent::Capture { .. }
                    | UnifiedEvent::Click { .. }
                    | UnifiedEvent::Scroll { .. }
                    | UnifiedEvent::Drag { .. }
                    | UnifiedEvent::ParamDelivery { .. }
            )
        })
        .collect();

    let is_tty = std::io::stderr().is_terminal();
    let mut monotonic_ms: u64 = 0;
    // Time in the original recording timeline — advances with monotonic_ms
    // but pauses during capture events (which consume real frames but shouldn't
    // shift the fixture's network event schedule).
    let mut fixture_ms: u64 = 0;
    let mut frame_count: u32 = 0;
    let mut captured_count: u32 = 0;
    let mut event_cursor: usize = 0;

    // Process all user events by advancing time to each one
    while event_cursor < user_events.len() {
        // User events fire at their recorded timestamp in the fixture timeline.
        // Convert to monotonic time: monotonic = recorded + (monotonic - fixture).
        let target_ms = user_events[event_cursor].at_ms + (monotonic_ms - fixture_ms);

        // Advance time frame-by-frame until we reach the event's timestamp
        while monotonic_ms < target_ms {
            runtime.set_time(system_time, monotonic_ms);
            runtime.inject_fixture_events(fixture_ms);
            deliver_all_io(&mut runtime, &mut renderer);
            if !render_frame(&mut runtime, &mut renderer, ctx, frame_count) {
                bail!("widget died at frame {frame_count}");
            }
            unsafe { gl.flush() };

            monotonic_ms += u64::from(DELTA_MS);
            fixture_ms += u64::from(DELTA_MS);
            system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
            frame_count += 1;
        }

        // Fire all events at this timestamp (may be multiple).
        // Compare in fixture-timeline space: the event's recorded at_ms
        // against the current fixture_ms.
        while event_cursor < user_events.len() && user_events[event_cursor].at_ms <= fixture_ms {
            let event = &user_events[event_cursor].event;
            match event {
                UnifiedEvent::Capture { duration_ms, fps } => {
                    // Run settle_delay extra frames so the widget can
                    // process network events and animate before capture.
                    // Both monotonic and fixture time advance so events
                    // continue to be delivered during the settle period.
                    for _ in 0..config.settle_delay {
                        runtime.set_time(system_time, monotonic_ms);
                        runtime.inject_fixture_events(fixture_ms);
                        deliver_all_io(&mut runtime, &mut renderer);
                        if !render_frame(&mut runtime, &mut renderer, ctx, frame_count) {
                            bail!("widget died during settle at frame {frame_count}");
                        }
                        unsafe { gl.flush() };
                        monotonic_ms += u64::from(DELTA_MS);
                        fixture_ms += u64::from(DELTA_MS);
                        system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
                        frame_count += 1;
                    }

                    // How many frames to capture and at what interval
                    let (total_frames, capture_interval_ms) = match (*duration_ms, *fps) {
                        (Some(dur), Some(f)) if f > 0 => {
                            #[expect(
                                clippy::cast_sign_loss,
                                clippy::cast_precision_loss,
                                reason = "duration_ms is small, result is always positive"
                            )]
                            let n = (dur as f64 * f64::from(f) / 1_000.0).ceil() as u32;
                            (n.max(1), 1_000 / u64::from(f))
                        }
                        (Some(dur), _) => {
                            // Duration but no fps: capture at render rate (DELTA_MS)
                            let n = (dur / u64::from(DELTA_MS)).max(1) as u32;
                            (n, u64::from(DELTA_MS))
                        }
                        _ => (1, 0), // single frame
                    };

                    tracing::debug!(
                        event = event_cursor,
                        frames = total_frames,
                        monotonic_ms,
                        fixture_ms,
                        recorded_ms = user_events[event_cursor].at_ms,
                        "capture"
                    );

                    let mut next_capture_at = monotonic_ms;
                    let mut frames_left = total_frames;

                    // During capture, only monotonic_ms advances — fixture_ms
                    // is frozen so network events stay aligned with the original
                    // recording timeline.
                    while frames_left > 0 {
                        // Advance to next capture point
                        while monotonic_ms < next_capture_at {
                            runtime.set_time(system_time, monotonic_ms);
                            runtime.inject_fixture_events(fixture_ms);
                            deliver_all_io(&mut runtime, &mut renderer);
                            if !render_frame(&mut runtime, &mut renderer, ctx, frame_count) {
                                bail!("widget died at frame {frame_count}");
                            }
                            unsafe { gl.flush() };
                            monotonic_ms += u64::from(DELTA_MS);
                            system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
                            frame_count += 1;
                        }

                        // Render and capture
                        runtime.set_time(system_time, monotonic_ms);
                        runtime.inject_fixture_events(fixture_ms);
                        deliver_all_io(&mut runtime, &mut renderer);
                        if !render_frame(&mut runtime, &mut renderer, ctx, frame_count) {
                            bail!("widget died at frame {frame_count}");
                        }
                        unsafe { gl.flush() };

                        let path = ctx
                            .output_dir
                            .join(format!("frame_{captured_count:04}.png"));
                        let pixels = read_fbo_pixels(&gl, fbo, ctx.width, ctx.height);
                        save_screenshot(&pixels, ctx.width, ctx.height, &path)?;
                        if !is_tty {
                            eprintln!("Captured frame {captured_count} → {}", path.display());
                        }
                        captured_count += 1;
                        frames_left -= 1;

                        monotonic_ms += u64::from(DELTA_MS);
                        system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
                        frame_count += 1;

                        if frames_left > 0 {
                            next_capture_at =
                                monotonic_ms + capture_interval_ms - u64::from(DELTA_MS);
                        }
                    }
                }
                UnifiedEvent::Click { element } => {
                    eprintln!("  [{event_cursor}] click(#{element})");
                    tracing::debug!(
                        event = event_cursor,
                        element,
                        monotonic_ms,
                        fixture_ms,
                        recorded_ms = user_events[event_cursor].at_ms,
                        frame_count,
                        "click"
                    );
                    let b = runtime.element_bounds(element).with_context(|| {
                        let available = runtime.element_ids().join(", ");
                        format!(
                            "element '#{element}' not found in hit regions at frame {frame_count} \
                             (monotonic={monotonic_ms}ms, fixture={fixture_ms}ms)\n\
                             available elements: [{available}]"
                        )
                    })?;
                    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                    runtime.push_touch_event(TouchEvent::Down { x: cx, y: cy });
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                    runtime.push_touch_event(TouchEvent::Up);
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                }
                UnifiedEvent::Scroll { element, delta } => {
                    eprintln!("  [{event_cursor}] scroll(#{element}, {delta})");
                    let b = runtime.element_bounds(element).with_context(|| {
                        let available = runtime.element_ids().join(", ");
                        format!(
                            "element '#{element}' not found in hit regions at frame {frame_count}\n\
                             available elements: [{available}]"
                        )
                    })?;
                    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                    let steps = 5;
                    let step_delta = *delta as f32 / 5.0;
                    runtime.push_touch_event(TouchEvent::Down { x: cx, y: cy });
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                    let mut current_y = cy;
                    for _ in 0..steps {
                        current_y += step_delta;
                        runtime.push_touch_event(TouchEvent::Move {
                            x: cx,
                            y: current_y,
                        });
                        tick_one_frame(
                            &mut runtime,
                            &mut renderer,
                            ctx,
                            &gl,
                            &mut monotonic_ms,
                            &mut fixture_ms,
                            &mut system_time,
                            &mut frame_count,
                        )?;
                    }
                    runtime.push_touch_event(TouchEvent::Up);
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                }
                UnifiedEvent::Drag { element, from, to } => {
                    eprintln!("  [{event_cursor}] drag(#{element}, {from}, {to})");
                    let b = runtime.element_bounds(element).with_context(|| {
                        let available = runtime.element_ids().join(", ");
                        format!(
                            "element '#{element}' not found in hit regions at frame {frame_count}\n\
                             available elements: [{available}]"
                        )
                    })?;
                    let cy = b.y + b.h / 2.0;
                    let start_x = b.x + from * b.w;
                    let end_x = b.x + to * b.w;
                    runtime.push_touch_event(TouchEvent::Down { x: start_x, y: cy });
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                    for i in 1..=DRAG_FRAMES {
                        let t = i as f32 / DRAG_FRAMES as f32;
                        let x = start_x + (end_x - start_x) * t;
                        runtime.push_touch_event(TouchEvent::Move { x, y: cy });
                        tick_one_frame(
                            &mut runtime,
                            &mut renderer,
                            ctx,
                            &gl,
                            &mut monotonic_ms,
                            &mut fixture_ms,
                            &mut system_time,
                            &mut frame_count,
                        )?;
                    }
                    runtime.push_touch_event(TouchEvent::Up);
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                }
                // Operator-driven params update — call `deliver_params_update` on the runtime
                // and let the widget's `on_params_update` hook fire. The version counter is
                // bumped by the runtime; we don't need to advance any timeline-side state.
                UnifiedEvent::ParamDelivery { params } => {
                    let table = bmc_wasm_runtime::parse_params_json(params)
                        .expect("BUG: capture ParamDelivery params must be valid");
                    runtime.deliver_params_update(table);
                }
                // Network events are handled by inject_fixture_events/fetch_interceptor
                UnifiedEvent::Fetch { .. }
                | UnifiedEvent::SsdpFound { .. }
                | UnifiedEvent::SsdpRemoved { .. }
                | UnifiedEvent::MdnsFound { .. }
                | UnifiedEvent::MdnsRemoved { .. }
                | UnifiedEvent::WsOpen { .. }
                | UnifiedEvent::WsMessage { .. }
                | UnifiedEvent::WsClose { .. }
                | UnifiedEvent::SocketConnected { .. }
                | UnifiedEvent::SocketData { .. }
                | UnifiedEvent::SocketClosed { .. }
                | UnifiedEvent::UdpResponse { .. }
                | UnifiedEvent::AudioPlay { .. }
                | UnifiedEvent::LedSetEffect { .. }
                | UnifiedEvent::LedSetBrightness { .. }
                | UnifiedEvent::LedEnable
                | UnifiedEvent::LedDisable => {}
            }

            if is_tty {
                eprint!(
                    "\r  frame {frame_count}  [{event_cursor}/{}]  ({captured_count} captured)   ",
                    user_events.len()
                );
            }
            event_cursor += 1;
        }
    }

    if is_tty {
        eprintln!();
    }
    eprintln!(
        "Done: {captured_count} frame(s) captured to {}",
        ctx.output_dir.display()
    );
    Ok(())
}

// ── Frame helpers ───────────────────────────────────────────────────

/// Tick a single frame: set time, inject fixtures, deliver I/O, render, advance.
///
/// Both `monotonic_ms` and `fixture_ms` are advanced by `DELTA_MS`.
#[expect(
    clippy::too_many_arguments,
    reason = "single-flow per-frame helper threading the replay loop's interlocked clocks; \
              splitting hurts readability"
)]
fn tick_one_frame(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
    ctx: &CaptureCtx,
    gl: &glow::Context,
    monotonic_ms: &mut u64,
    fixture_ms: &mut u64,
    system_time: &mut chrono::DateTime<chrono::FixedOffset>,
    frame_count: &mut u32,
) -> Result<()> {
    runtime.set_time(*system_time, *monotonic_ms);
    runtime.inject_fixture_events(*fixture_ms);
    deliver_all_io(runtime, renderer);
    if !render_frame(runtime, renderer, ctx, *frame_count) {
        bail!("widget died at frame {}", *frame_count);
    }
    unsafe { gl.flush() };
    *monotonic_ms += u64::from(DELTA_MS);
    *fixture_ms += u64::from(DELTA_MS);
    *system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
    *frame_count += 1;
    Ok(())
}

/// Render one frame. Returns false if the widget died or errored (caller should break).
fn render_frame(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
    ctx: &CaptureCtx,
    frame_count: u32,
) -> bool {
    renderer.begin_frame(ctx.width, ctx.height, 1.0);
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    let ptr = std::ptr::NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    match runtime.with_renderer(ptr, |rt| rt.render(DELTA_MS)) {
        Ok(RenderStatus::Dead) => {
            eprintln!("Widget died at frame {frame_count}");
            false
        }
        Ok(_) => {
            renderer.flush();
            true
        }
        Err(e) => {
            eprintln!("Render error at frame {frame_count}: {e}");
            false
        }
    }
}

/// Deliver all async I/O to the runtime. Returns true if any data arrived.
fn deliver_all_io(runtime: &mut WasmWidgetRuntime, renderer: &mut FemtoVgRenderer) -> bool {
    let had_pending_fetches = runtime.has_pending_fetches();
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    let ptr = std::ptr::NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    runtime.with_renderer(ptr, |runtime| {
        runtime.deliver_fetch_responses();
        let fetches_completed = had_pending_fetches && !runtime.has_pending_fetches();
        let had_ws = runtime.deliver_ws_messages();
        let had_socket = runtime.deliver_socket_events();
        let had_mdns = runtime.deliver_mdns_events();
        let had_ssdp = runtime.deliver_ssdp_events();
        let had_udp = runtime.deliver_udp_broadcast_events();
        runtime.deliver_http_requests();
        fetches_completed || had_ws || had_socket || had_mdns || had_ssdp || had_udp
    })
}

// ── KV directory setup ──────────────────────────────────────────────

/// Prepare a fresh KV directory for unified fixture replay.
///
/// Seeds from the fixture header's KV map (not from secrets.ini — the fixture
/// is self-contained). Config KV and variant KV are applied on top.
fn prepare_unified_kv_dir(
    ctx: &CaptureCtx,
    config: &CaptureConfig,
    widget_name: &str,
    fixture: &UnifiedFixture,
) -> PathBuf {
    let variant_suffix = ctx.variant.as_deref().unwrap_or("_default");
    let kv_dir = std::env::temp_dir()
        .join("bmc-wasm-capture")
        .join(widget_name)
        .join(variant_suffix)
        .join(&ctx.size_name);
    let _ = std::fs::remove_dir_all(&kv_dir);
    let _ = std::fs::create_dir_all(&kv_dir);
    // Fixture header KV (self-contained baseline)
    for (key, value) in &fixture.header.kv {
        let _ = std::fs::write(kv_dir.join(key), value.as_bytes());
    }
    // Config KV overrides
    for (key, value) in &config.kv {
        let _ = std::fs::write(kv_dir.join(key), value.as_bytes());
    }
    // Variant KV overrides
    if let Some(ref variant_name) = ctx.variant
        && let Some(variant) = config.variants.iter().find(|v| &v.name == variant_name)
    {
        for (key, value) in &variant.kv {
            let _ = std::fs::write(kv_dir.join(key), value.as_bytes());
        }
    }
    kv_dir
}

// ── Event splitting ─────────────────────────────────────────────────

/// A pre-recorded fetch response for the interceptor.
struct FetchEntry {
    status: u32,
    body: Vec<u8>,
}

/// Split a unified fixture timeline into fetch interceptors and network events.
///
/// - `Fetch` events → `HashMap<String, FetchEntry>` keyed by `"METHOD URL"`
/// - Network events (SSDP, mDNS, WS, Socket, UDP) → `Vec<FixtureEvent>`
/// - User actions (Capture, Click, Scroll, Drag) are skipped (handled in the
///   main replay loop)
#[expect(clippy::too_many_lines)]
fn split_unified_events(
    fixture: &UnifiedFixture,
) -> (HashMap<String, FetchEntry>, Vec<FixtureEvent>) {
    let mut fetches = HashMap::new();
    let mut network_events = Vec::new();

    for te in &fixture.events {
        match &te.event {
            UnifiedEvent::Fetch {
                method,
                url,
                status,
                body,
            } => {
                let key = format!("{method} {url}");
                fetches.insert(
                    key,
                    FetchEntry {
                        status: *status,
                        body: body.to_bytes(),
                    },
                );
            }

            // Convert network events to the runtime's FixtureEvent format
            UnifiedEvent::SsdpFound { search_id, data } => {
                let Some(search_id) = SsdpSearchId::from_wire(*search_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SsdpFound {
                        search_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::SsdpRemoved { search_id, data } => {
                let Some(search_id) = SsdpSearchId::from_wire(*search_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SsdpRemoved {
                        search_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::MdnsFound { browse_id, data } => {
                let Some(browse_id) = MdnsBrowseId::from_wire(*browse_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::MdnsFound {
                        browse_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::MdnsRemoved { browse_id, data } => {
                let Some(browse_id) = MdnsBrowseId::from_wire(*browse_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::MdnsRemoved {
                        browse_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::WsOpen { ws_id } => {
                let Some(ws_id) = WebsocketId::from_wire(*ws_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::WsOpen { ws_id },
                });
            }
            UnifiedEvent::WsMessage { ws_id, data } => {
                let Some(ws_id) = WebsocketId::from_wire(*ws_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::WsMessage {
                        ws_id,
                        data: data.to_bytes(),
                    },
                });
            }
            UnifiedEvent::WsClose { ws_id, code } => {
                let Some(ws_id) = WebsocketId::from_wire(*ws_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::WsClose { ws_id, code: *code },
                });
            }
            UnifiedEvent::SocketConnected { socket_id } => {
                let Some(socket_id) = SocketId::from_wire(*socket_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SocketConnected { socket_id },
                });
            }
            UnifiedEvent::SocketData { socket_id, data } => {
                let Some(socket_id) = SocketId::from_wire(*socket_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SocketData {
                        socket_id,
                        data: data.to_bytes(),
                    },
                });
            }
            UnifiedEvent::SocketClosed { socket_id, code } => {
                let Some(socket_id) = SocketId::from_wire(*socket_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SocketClosed {
                        socket_id,
                        code: *code,
                    },
                });
            }
            UnifiedEvent::UdpResponse {
                broadcast_id,
                data,
                source,
            } => {
                let Some(broadcast_id) = UdpBroadcastId::from_wire(*broadcast_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::UdpResponse {
                        broadcast_id,
                        data: data.clone(),
                        source: source.clone(),
                    },
                });
            }

            // User actions and informational events are handled in the main replay loop
            UnifiedEvent::Capture { .. }
            | UnifiedEvent::Click { .. }
            | UnifiedEvent::Scroll { .. }
            | UnifiedEvent::Drag { .. }
            | UnifiedEvent::ParamDelivery { .. }
            | UnifiedEvent::AudioPlay { .. }
            | UnifiedEvent::LedSetEffect { .. }
            | UnifiedEvent::LedSetBrightness { .. }
            | UnifiedEvent::LedEnable
            | UnifiedEvent::LedDisable => {}
        }
    }

    (fetches, network_events)
}

// ── GL helpers ──────────────────────────────────────────────────────

#[expect(clippy::cast_possible_wrap)]
fn create_fbo(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<(glow::Framebuffer, glow::Texture)> {
    unsafe {
        let texture = gl
            .create_texture()
            .map_err(|e| anyhow::anyhow!("create texture: {e}"))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );

        let fbo = gl
            .create_framebuffer()
            .map_err(|e| anyhow::anyhow!("create framebuffer: {e}"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );

        // Stencil buffer required by FemtoVG
        let rbo = gl
            .create_renderbuffer()
            .map_err(|e| anyhow::anyhow!("create renderbuffer: {e}"))?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH24_STENCIL8,
            width as i32,
            height as i32,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_STENCIL_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(rbo),
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);

        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        assert_eq!(
            status,
            glow::FRAMEBUFFER_COMPLETE,
            "FBO incomplete: {status:#x}"
        );

        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);

        Ok((fbo, texture))
    }
}

#[expect(clippy::cast_possible_wrap)]
fn read_fbo_pixels(gl: &glow::Context, fbo: glow::Framebuffer, w: u32, h: u32) -> Vec<u8> {
    unsafe {
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
        let mut pixels = vec![0_u8; (w * h * 4) as usize];
        gl.read_pixels(
            0,
            0,
            w as i32,
            h as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        pixels
    }
}

fn save_screenshot(pixels: &[u8], w: u32, h: u32, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // OpenGL gives us bottom-up rows — flip vertically
    let row_bytes = (w * 4) as usize;
    let mut flipped = vec![0_u8; pixels.len()];
    for y in 0..h as usize {
        let src_row = (h as usize - 1 - y) * row_bytes;
        let dst_row = y * row_bytes;
        flipped[dst_row..dst_row + row_bytes]
            .copy_from_slice(&pixels[src_row..src_row + row_bytes]);
    }
    image::save_buffer(path, &flipped, w, h, image::ColorType::Rgba8)?;
    Ok(())
}

// ── Headless GL setup ────────────────────────────────────────────────
//
// Platform-specific EGL context creation.  Everything after this point
// goes through `glow::Context` and is fully cross-platform.
//
// Linux  — glutin + Mesa EGL (llvmpipe for deterministic CI rendering)
// macOS  — khronos-egl + ANGLE (Metal backend, loaded at runtime)

// ── Linux: glutin + Mesa EGL ────────────────────────────────────────

#[cfg(target_os = "linux")]
fn create_headless_egl_display() -> Result<Display> {
    let devices: Vec<_> = glutin::api::egl::device::Device::query_devices()
        .context("EGL device enumeration not supported (missing EGL_EXT_device_query?)")?
        .collect();

    let device = devices
        .iter()
        .find(|d| d.extensions().contains("EGL_MESA_device_software"))
        .or_else(|| devices.first())
        .context("no EGL devices found (is Mesa/libGL in LD_LIBRARY_PATH?)")?;

    let display = unsafe { glutin::api::egl::display::Display::with_device(device, None) }
        .context("failed to create EGL display from device")?;

    Ok(Display::Egl(display))
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::type_complexity,
    reason = "headless EGL setup returns several owned handles whose lifetimes must \
              outlive the caller; pulling them out via a struct would force a public \
              `pub(super)` wrapper for a single call site"
)]
fn setup_gl_and_runtime(
    ctx: &CaptureCtx,
    rt_config: RuntimeConfig,
) -> Result<(
    glow::Context,
    glow::Framebuffer,
    Box<dyn std::any::Any>,
    FemtoVgRenderer,
    WasmWidgetRuntime,
)> {
    let egl_display =
        create_headless_egl_display().context("failed to create headless EGL display")?;

    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_stencil_size(8)
        .with_surface_type(glutin::config::ConfigSurfaceTypes::PBUFFER);
    let gl_config = unsafe { egl_display.find_configs(template.build()) }
        .map_err(|e| anyhow::anyhow!("failed to find GL configs: {e}"))?
        .reduce(|a, b| {
            if a.num_samples() > b.num_samples() {
                a
            } else {
                b
            }
        })
        .context("no suitable GL config found")?;

    let gl_display = gl_config.display();
    let context_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(glutin::context::Version::new(2, 0))))
        .build(None);
    let gl_context = unsafe {
        gl_display
            .create_context(&gl_config, &context_attrs)
            .context("failed to create GL context")?
    };

    // Pbuffer surface — fully offscreen, no window needed.
    let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
        NonZeroU32::new(ctx.width).expect("BUG: zero width"),
        NonZeroU32::new(ctx.height).expect("BUG: zero height"),
    );
    let surface = unsafe {
        gl_display
            .create_pbuffer_surface(&gl_config, &surface_attrs)
            .context("failed to create pbuffer surface")?
    };
    let gl_context = gl_context
        .make_current(&surface)
        .context("failed to make GL context current")?;

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            gl_display.get_proc_address(&CString::new(s).unwrap_or_default())
        })
    };
    let (fbo, texture) = create_fbo(&gl, ctx.width, ctx.height)?;
    let fbo_id = fbo.0.get();

    let wasm_bytes = std::fs::read(&ctx.wasm_path).context("failed to read WASM file")?;
    // SAFETY: GL context is current on this thread for the lifetime of the
    // returned `keep_alive` bundle, which holds `gl_context`.
    let renderer = unsafe {
        FemtoVgRenderer::new(
            |s| gl_display.get_proc_address(&CString::new(s).unwrap_or_default()),
            ctx.width,
            ctx.height,
            fbo_id,
            rt_config.mesh_msaa_samples,
        )
    }
    .context("failed to create renderer")?;
    let runtime = WasmWidgetRuntime::new(&wasm_bytes, ctx.width, ctx.height, rt_config)
        .context("failed to create WASM runtime")?;

    let keep_alive: Box<dyn std::any::Any> = Box::new((texture, surface, gl_context));
    Ok((gl, fbo, keep_alive, renderer, runtime))
}

// ── macOS: khronos-egl + ANGLE ──────────────────────────────────────

#[cfg(target_os = "macos")]
fn load_angle_egl() -> Result<khronos_egl::DynamicInstance<khronos_egl::EGL1_4>> {
    // Try default search path first (works when DYLD_LIBRARY_PATH is set,
    // e.g. inside `nix develop`), then fall back to common Homebrew prefixes.
    let candidates = [
        "libEGL.dylib",
        "/opt/homebrew/lib/libEGL.dylib", // Homebrew on Apple Silicon
        "/usr/local/lib/libEGL.dylib",    // Homebrew on Intel
    ];

    let mut last_err = None;
    for path in candidates {
        // SAFETY: loading a well-known library path.
        match unsafe {
            khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required_from_filename(path)
        } {
            Ok(instance) => return Ok(instance),
            Err(e) => last_err = Some(e),
        }
    }

    Err(anyhow::anyhow!(
        "failed to load libEGL.dylib: {}\n\n\
         ANGLE is required on macOS for headless GL rendering.\n\
         Install via one of:\n  \
         - nix develop\n  \
         - brew tap startergo/angle && brew install angle",
        last_err.expect("BUG: candidates list is empty"),
    ))
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::type_complexity,
    reason = "headless EGL setup returns several owned handles whose lifetimes must \
              outlive the caller; pulling them out via a struct would force a public \
              `pub(super)` wrapper for a single call site"
)]
fn setup_gl_and_runtime(
    ctx: &CaptureCtx,
    rt_config: RuntimeConfig,
) -> Result<(
    glow::Context,
    glow::Framebuffer,
    Box<dyn std::any::Any>,
    FemtoVgRenderer,
    WasmWidgetRuntime,
)> {
    use khronos_egl as egl;

    let instance = load_angle_egl()?;

    // SAFETY: DEFAULT_DISPLAY is a well-known constant — ANGLE handles it.
    let display =
        unsafe { instance.get_display(egl::DEFAULT_DISPLAY) }.context("eglGetDisplay failed")?;
    instance
        .initialize(display)
        .context("eglInitialize failed")?;

    let config = instance
        .choose_first_config(
            display,
            &[
                egl::RED_SIZE,
                8,
                egl::GREEN_SIZE,
                8,
                egl::BLUE_SIZE,
                8,
                egl::ALPHA_SIZE,
                8,
                egl::STENCIL_SIZE,
                8,
                egl::SURFACE_TYPE,
                egl::PBUFFER_BIT,
                egl::RENDERABLE_TYPE,
                egl::OPENGL_ES2_BIT,
                egl::NONE,
            ],
        )?
        .context("no suitable EGL config")?;

    let context = instance
        .create_context(
            display,
            config,
            None,
            &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
        )
        .context("eglCreateContext failed")?;

    let surface = instance
        .create_pbuffer_surface(
            display,
            config,
            &[
                egl::WIDTH,
                ctx.width.cast_signed(),
                egl::HEIGHT,
                ctx.height.cast_signed(),
                egl::NONE,
            ],
        )
        .context("eglCreatePbufferSurface failed")?;

    instance
        .make_current(display, Some(surface), Some(surface), Some(context))
        .context("eglMakeCurrent failed")?;

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            instance
                .get_proc_address(s)
                .map_or(std::ptr::null(), |f| f as *const _)
        })
    };

    let (fbo, texture) = create_fbo(&gl, ctx.width, ctx.height)?;
    let fbo_id = fbo.0.get();

    let wasm_bytes = std::fs::read(&ctx.wasm_path).context("failed to read WASM file")?;
    // SAFETY: ANGLE EGL context is current on this thread for the lifetime of
    // the returned `keep_alive` bundle.
    let renderer = unsafe {
        FemtoVgRenderer::new(
            |s| {
                instance
                    .get_proc_address(s)
                    .map_or(std::ptr::null(), |f| f as *const _)
            },
            ctx.width,
            ctx.height,
            fbo_id,
            rt_config.mesh_msaa_samples,
        )
    }
    .context("failed to create renderer")?;
    let runtime = WasmWidgetRuntime::new(&wasm_bytes, ctx.width, ctx.height, rt_config)
        .context("failed to create WASM runtime")?;

    // Keep EGL state alive — dropping tears down the GL context.
    let keep_alive: Box<dyn std::any::Any> =
        Box::new((instance, display, context, surface, texture));

    Ok((gl, fbo, keep_alive, renderer, runtime))
}

// ── Init subcommand ─────────────────────────────────────────────────

/// Write a default `capture/config.toml` template with commented-out options.
pub fn write_default_capture_config(dir: &Path) -> Result<()> {
    let capture_dir = dir.join("capture");
    let _ = std::fs::create_dir_all(&capture_dir);
    let path = capture_dir.join("config.toml");
    if path.exists() {
        bail!(
            "{} already exists — remove it first to regenerate",
            path.display()
        );
    }

    let template = r#"# Capture configuration for visual regression testing.
#
# This file controls how the headless capture binary renders and screenshots
# your widget. All fields are optional — sensible defaults are used when omitted.

# ── Timing ───────────────────────────────────────────────────────────

# Override the start time for deterministic rendering (default: 2026-01-01T12:00:00).
# time = "2026-06-15T09:30:00"

# Settlement timeout in frames before force-capturing (default: 300 = ~5s).
# Increase for widgets with slow network I/O.
# timeout = 300

# Extra frames to wait after I/O settles before capturing (default: 0).
# Useful when the widget animates after data arrives.
# settle_delay = 30

# ── Fixtures ─────────────────────────────────────────────────────────

# Paths to pre-recorded unified fixture files (one per size).
# Record fixtures with: make record EXAMPLE=<name> SIZE=<size>
#
# [fixtures]
# full   = "fixtures/full.jsonl.gz"
# large  = "fixtures/large.jsonl.gz"
# medium = "fixtures/medium.jsonl.gz"
# small  = "fixtures/small.jsonl.gz"

# ── Size filtering ───────────────────────────────────────────────────

# Only capture specific sizes (default: all sizes).
# Valid names: "full" (1280x480), "large" (638x480), "medium" (638x238), "small" (317x238).
# sizes = ["full", "large"]

# ── KV store defaults ────────────────────────────────────────────────

# Default key-value pairs injected into the widget's KV store.
# [kv]
# theme = "dark"
# language = "en"

# ── Named variants ───────────────────────────────────────────────────

# Each variant captures a separate set of screenshots with different KV values.
# Variant KV is merged on top of the base [kv] section.
#
# [[variants]]
# name = "dark"
# kv = { theme = "dark" }
#
# [[variants]]
# name = "light"
# kv = { theme = "light" }
"#;

    std::fs::write(&path, template)
        .with_context(|| format!("failed to write {}", path.display()))?;
    eprintln!("Created {}", path.display());
    Ok(())
}
