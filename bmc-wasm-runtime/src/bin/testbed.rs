// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget development testbed with hot-reloading.
//!
//! Uses winit for windowing, glutin for OpenGL context, and FemtoVG for GPU rendering.
//! Renders all 4 widget sizes simultaneously.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::integer_division,
    clippy::wildcard_enum_match_arm
)]

use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::ContextAttributesBuilder;
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowButtons};

use bmc_wasm_protocol::FormatPreferences;
use bmc_wasm_runtime::components::{ButtonSize, ButtonStyle, draw_button};
use bmc_wasm_runtime::gpu::FemtoVgRenderer;
use bmc_wasm_runtime::interaction::{InteractionState, TouchEvent};
use bmc_wasm_runtime::perf_overlay::PerfOverlay;
use bmc_wasm_runtime::renderer::Renderer;
use bmc_wasm_runtime::{FrameTimings, RenderStatus, WasmWidgetRuntime};

// Layout constants
const PREVIEW_GAP: u32 = 8;
const PREVIEW_MARGIN: u32 = 8;
// Inner width = max(1280, 638+8+638) = 1284
const PREVIEW_WIDTH: u32 = PREVIEW_MARGIN + 1284 + PREVIEW_MARGIN; // 1300
// Inner height = 480+8+max(480, 238+8+238) = 480+8+484 = 972
const PREVIEW_HEIGHT: u32 = PREVIEW_MARGIN + 972 + PREVIEW_MARGIN; // 988

/// Scale a logical pixel value by the DPI factor to get physical pixels.
#[expect(clippy::cast_sign_loss)]
fn scaled(logical: u32, dpi: f32) -> u32 {
    (logical as f32 * dpi) as u32
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("WASM Widget Testbed");
        eprintln!("Usage: testbed <wasm_file> [--perf-report=<path>] [--perf-frames=<N>]");
        std::process::exit(1);
    }

    let wasm_path = PathBuf::from(&args[1]);

    // Parse optional flags from remaining args
    let mut perf_report_path: Option<PathBuf> = None;
    let mut perf_frames: u32 = 600;
    for arg in &args[2..] {
        if let Some(path) = arg.strip_prefix("--perf-report=") {
            perf_report_path = Some(PathBuf::from(path));
        } else if let Some(n) = arg.strip_prefix("--perf-frames=") {
            perf_frames = n.parse().unwrap_or(600);
        }
    }

    println!("Loading widget from: {}", wasm_path.display());
    println!("Display size: {PREVIEW_WIDTH}x{PREVIEW_HEIGHT} (4 sizes)");
    if let Some(ref path) = perf_report_path {
        println!("Perf report: {} ({perf_frames} frames)", path.display());
    }

    let event_loop = EventLoop::new()?;
    let mut app = App {
        wasm_path,
        state: None,
        rss_after_gl_kb: None,
        rss_after_runtime_kb: None,
        perf_report_path,
        perf_frames,
    };
    event_loop.run_app(&mut app)?;
    print_memory_stats(app.rss_after_gl_kb, app.rss_after_runtime_kb);
    Ok(())
}

/// Read current RSS from `/proc/self/status` in kB (Linux only).
fn current_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.trim().strip_suffix("kB")?.trim().parse().ok();
        }
    }
    None
}

/// Print peak memory usage from `/proc/self/status` (Linux only, zero overhead).
fn print_memory_stats(rss_after_gl_kb: Option<u64>, rss_after_runtime_kb: Option<u64>) {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return;
    };
    eprintln!("\n=== Memory ===");
    if let (Some(gl), Some(rt)) = (rss_after_gl_kb, rss_after_runtime_kb) {
        let rt_delta = rt.saturating_sub(gl);
        eprintln!("GL + windowing:    {gl:>6} kB");
        eprintln!("WASM runtime:      {rt_delta:>6} kB  (delta)");
    }
    for line in status.lines() {
        if line.starts_with("VmPeak:") || line.starts_with("VmRSS:") || line.starts_with("VmHWM:") {
            eprintln!("{}", line.trim());
        }
    }
}

// ── App ────────────────────────────────────────────────────────────

struct App {
    wasm_path: PathBuf,
    state: Option<PreviewState>,
    rss_after_gl_kb: Option<u64>,
    rss_after_runtime_kb: Option<u64>,
    /// If set, write a JSON perf report to this path after `perf_frames` frames.
    perf_report_path: Option<PathBuf>,
    perf_frames: u32,
}

// ── Preview mode ───────────────────────────────────────────────────

struct PreviewTile {
    runtime: WasmWidgetRuntime,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pw: u32,
    ph: u32,
    label: &'static str,
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    logged_dead: bool,
    ever_rendered: bool,
}

struct PreviewState {
    // Drop order: tiles (FemtoVG Canvases) → GL resources → window
    tiles: Vec<PreviewTile>,
    checker_fbo: glow::Framebuffer,
    _checker_texture: glow::Texture,
    stats_fbo: glow::Framebuffer,
    _stats_texture: glow::Texture,
    stats_renderer: FemtoVgRenderer,
    gl: glow::Context,
    gl_surface: glutin::surface::Surface<WindowSurface>,
    gl_context: glutin::context::PossiblyCurrentContext,
    gl_config: glutin::config::Config,
    window: Window,
    _watcher: RecommendedWatcher,
    watcher_rx: Receiver<()>,
    last_frame: Instant,
    needs_render: bool,
    pending_reload: bool,
    mouse_pos: (i32, i32),
    mouse_down: bool,
    perf_overlay: PerfOverlay,
    stats_interaction: InteractionState,
    /// Display DPI scale (for screen coordinate mapping only, not FemtoVG rendering).
    dpi_scale: f32,
    /// Physical pixel dimensions of the screen surface.
    phys_w: u32,
    phys_h: u32,
    /// Total frames rendered (for perf-report exit condition).
    frame_count: u32,
    /// Collected per-frame timings for perf report.
    perf_samples: Vec<FrameTimings>,
}

// Stats panel position: empty area right of SMALL tile
const STATS_X: u32 = M + 638 + G + 317 + G;
const STATS_Y: u32 = M + 480 + G + 238 + G;
const STATS_W: u32 = PREVIEW_WIDTH - M - STATS_X;
const STATS_H: u32 = PREVIEW_HEIGHT - M - STATS_Y;

/// Tile definitions: (x, y, w, h, label) — positioned with margin offset.
const M: u32 = PREVIEW_MARGIN;
const G: u32 = PREVIEW_GAP;
const TILE_DEFS: [(u32, u32, u32, u32, &str); 4] = [
    (M, M, 1280, 480, "FULL"),
    (M, M + 480 + G, 638, 480, "LARGE"),
    (M + 638 + G, M + 480 + G, 638, 238, "MEDIUM"),
    (M + 638 + G, M + 480 + G + 238 + G, 317, 238, "SMALL"),
];

impl App {
    #[expect(clippy::too_many_lines)]
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        // Use LogicalSize so the window adapts to display DPI — on HiDPI displays
        // the physical surface is larger, giving us higher-resolution text rendering.
        let win_size = LogicalSize::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
        let window_attrs = Window::default_attributes()
            .with_title("WASM Widget Testbed")
            .with_inner_size(win_size)
            .with_min_inner_size(win_size)
            .with_max_inner_size(win_size)
            .with_resizable(false)
            .with_enabled_buttons(WindowButtons::CLOSE | WindowButtons::MINIMIZE);

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_stencil_size(8);
        let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attrs));

        let (window, gl_config) = display_builder
            .build(event_loop, template, |configs| {
                configs
                    .reduce(|a, c| {
                        if c.num_samples() > a.num_samples() {
                            c
                        } else {
                            a
                        }
                    })
                    .unwrap_or_else(|| unreachable!())
            })
            .map_err(|e| anyhow::anyhow!("Failed to build display: {e}"))?;

        let window = window.context("Failed to create window")?;
        let gl_display = gl_config.display();
        let raw_handle = window.window_handle()?.as_raw();

        let context_attrs = ContextAttributesBuilder::new().build(Some(raw_handle));
        let gl_context = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .context("Failed to create GL context")?
        };

        let size = window.inner_size();
        let (nz_w, nz_h) = (
            NonZeroU32::new(size.width.max(1)).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(size.height.max(1)).unwrap_or(NonZeroU32::MIN),
        );
        let surface_attrs =
            SurfaceAttributesBuilder::<WindowSurface>::new().build(raw_handle, nz_w, nz_h);
        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .context("Failed to create GL surface")?
        };

        let gl_context = gl_context
            .make_current(&gl_surface)
            .context("Failed to make GL context current")?;

        if let Err(e) = gl_surface.set_swap_interval(
            &gl_context,
            SwapInterval::Wait(NonZeroU32::new(1).expect("BUG: 1 is non-zero")),
        ) {
            eprintln!("Warning: failed to enable vsync: {e}");
        }

        self.rss_after_gl_kb = current_rss_kb();

        let dpi_scale = window.scale_factor() as f32;
        let phys_w = scaled(PREVIEW_WIDTH, dpi_scale);
        let phys_h = scaled(PREVIEW_HEIGHT, dpi_scale);
        println!("Display DPI scale: {dpi_scale} ({phys_w}×{phys_h} physical)");

        let (watcher, watcher_rx) =
            setup_watcher(&self.wasm_path).context("Failed to set up file watcher")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_display
                    .get_proc_address(&CString::new(s).unwrap_or_default())
                    .cast()
            })
        };

        // KV storage directory: ./widget_data/<widget_name>/ next to the WASM file
        let widget_name = self
            .wasm_path
            .file_stem()
            .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
        let kv_base = self
            .wasm_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("widget_data")
            .join(&widget_name);

        let mut tiles = Vec::with_capacity(4);
        for &(x, y, w, h, label) in &TILE_DEFS {
            // FBOs at logical resolution — the blit to screen handles upscaling
            let (pw, ph) = (w, h);
            let (fbo, texture) = create_fbo(&gl, pw, ph, true)?;
            let fbo_id = fbo.0.get();
            let mut runtime = create_runtime(&self.wasm_path, &gl_config, w, h, fbo_id)
                .context("Failed to create runtime")?;
            runtime.set_kv_store_path(kv_base.clone());
            tiles.push(PreviewTile {
                runtime,
                x,
                y,
                w,
                h,
                pw,
                ph,
                label,
                fbo,
                texture,
                logged_dead: false,
                ever_rendered: false,
            });
        }

        let (checker_fbo, checker_texture) = create_fbo(&gl, phys_w, phys_h, false)?;
        render_checkerboard_to_fbo(&gl, checker_fbo, phys_w, phys_h);
        let (stats_fbo, stats_texture) = create_fbo(&gl, STATS_W, STATS_H, true)?;
        let stats_renderer = unsafe {
            let gl_display = gl_config.display();
            FemtoVgRenderer::new(
                |s| gl_display.get_proc_address(&CString::new(s).unwrap_or_default()),
                STATS_W,
                STATS_H,
                stats_fbo.0.get(),
            )
            .context("Failed to create stats renderer")?
        };

        self.rss_after_runtime_kb = current_rss_kb();

        let (major, minor, patch) = tiles[0].runtime.sdk_version();
        println!("Widget SDK version: {major}.{minor}.{patch}");
        let widget_name = self
            .wasm_path
            .file_stem()
            .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
        window.set_title(&format!("{widget_name} — SDK {major}.{minor}.{patch}"));

        self.state = Some(PreviewState {
            tiles,
            checker_fbo,
            _checker_texture: checker_texture,
            stats_fbo,
            _stats_texture: stats_texture,
            stats_renderer,
            gl,
            gl_surface,
            gl_context,
            gl_config,
            window,
            _watcher: watcher,
            watcher_rx,
            last_frame: Instant::now(),
            needs_render: true,
            pending_reload: false,
            mouse_pos: (0, 0),
            mouse_down: false,
            perf_overlay: PerfOverlay::new(),
            stats_interaction: InteractionState::new(),
            dpi_scale,
            phys_w,
            phys_h,
            frame_count: 0,
            perf_samples: Vec::new(),
        });
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        if let Err(e) = self.init(event_loop) {
            eprintln!("Fatal initialization error: {e:#}");
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Some(state) = &mut self.state {
            let was_redraw = matches!(event, WindowEvent::RedrawRequested);
            handle_preview_event(&self.wasm_path, state, event_loop, event);

            // Perf report: collect sample after each rendered frame
            if was_redraw && let Some(ref report_path) = self.perf_report_path {
                state
                    .perf_samples
                    .push(state.perf_overlay.last_sample_timings());
                state.frame_count += 1;

                if state.frame_count >= self.perf_frames {
                    write_perf_report(report_path, &state.perf_samples);
                    event_loop.exit();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(s) = &mut self.state else { return };

        if s.watcher_rx.try_recv().is_ok() {
            while s.watcher_rx.try_recv().is_ok() {}
            s.pending_reload = true;
        }

        let has_async_io = s.tiles.iter().any(|t| {
            t.runtime.has_pending_fetches()
                || t.runtime.has_active_websockets()
                || t.runtime.has_active_sockets()
                || t.runtime.has_active_mdns_browses()
                || t.runtime.has_active_http_listeners()
        });

        let wants_frame = s.tiles.iter().any(|t| t.runtime.wants_next_frame());

        if s.needs_render || s.pending_reload || wants_frame {
            event_loop.set_control_flow(ControlFlow::Poll);
            s.window.request_redraw();
        } else if let Some(delay_ms) = s
            .tiles
            .iter()
            .filter_map(|t| t.runtime.next_frame_delay())
            .min()
        {
            // Widget requested a delayed frame — wake after the delay and redraw
            if delay_ms == 0 {
                s.window.request_redraw();
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_millis(delay_ms.into()),
                ));
                s.window.request_redraw();
            }
        } else if has_async_io {
            // Active WebSocket/fetch — poll periodically to deliver messages
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(16),
            ));
            s.window.request_redraw();
        } else {
            // Fully idle — just poll for hot-reload
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
        }
    }
}

// ── Event handling ─────────────────────────────────────────────────

fn handle_preview_event(
    wasm_path: &Path,
    state: &mut PreviewState,
    event_loop: &ActiveEventLoop,
    event: WindowEvent,
) {
    match event {
        WindowEvent::CloseRequested => event_loop.exit(),

        WindowEvent::KeyboardInput { event, .. } => {
            if event.state == ElementState::Pressed
                && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
            {
                event_loop.exit();
            }
        }

        WindowEvent::CursorMoved { position, .. } => {
            // Convert physical cursor position to logical coordinates
            let s = f64::from(state.dpi_scale);
            state.mouse_pos = ((position.x / s) as i32, (position.y / s) as i32);
            if state.mouse_down
                && let Some((idx, lx, ly)) = hit_test_tile(&state.tiles, state.mouse_pos)
            {
                state.tiles[idx]
                    .runtime
                    .push_touch_event(TouchEvent::Move { x: lx, y: ly });
                state.needs_render = true;
            }
        }

        WindowEvent::MouseInput {
            state: btn_state,
            button: MouseButton::Left,
            ..
        } => {
            if let Some((idx, lx, ly)) = hit_test_tile(&state.tiles, state.mouse_pos) {
                match btn_state {
                    ElementState::Pressed => {
                        state.mouse_down = true;
                        state.tiles[idx]
                            .runtime
                            .push_touch_event(TouchEvent::Down { x: lx, y: ly });
                        state.needs_render = true;
                    }
                    ElementState::Released => {
                        state.mouse_down = false;
                        state.tiles[idx]
                            .runtime
                            .push_touch_event(TouchEvent::Up { x: lx, y: ly });
                        state.needs_render = true;
                    }
                }
            } else if let Some((lx, ly)) = hit_test_stats(state.mouse_pos) {
                match btn_state {
                    ElementState::Pressed => {
                        state.mouse_down = true;
                        state
                            .stats_interaction
                            .push_event(TouchEvent::Down { x: lx, y: ly });
                        state.needs_render = true;
                    }
                    ElementState::Released => {
                        state.mouse_down = false;
                        state
                            .stats_interaction
                            .push_event(TouchEvent::Up { x: lx, y: ly });
                        state.needs_render = true;
                    }
                }
            } else if btn_state == ElementState::Released {
                state.mouse_down = false;
            }
        }

        WindowEvent::MouseWheel { delta, .. } => {
            let delta_y = match delta {
                MouseScrollDelta::LineDelta(_, y) => (-y * 30.0) as i32,
                MouseScrollDelta::PixelDelta(pos) => -pos.y as i32,
            };
            if delta_y != 0
                && let Some((idx, lx, ly)) = hit_test_tile(&state.tiles, state.mouse_pos)
            {
                state.tiles[idx]
                    .runtime
                    .push_touch_event(TouchEvent::Scroll {
                        x: lx,
                        y: ly,
                        delta_y,
                    });
                state.needs_render = true;
            }
        }

        WindowEvent::Resized(size) => {
            // Undo WM-triggered maximize/resize (enabled_buttons not supported on X11/Wayland,
            // is_maximized unreliable — just force size back unconditionally).
            // Compare against physical dimensions since Resized gives PhysicalSize.
            if size.width != state.phys_w || size.height != state.phys_h {
                state.window.set_maximized(false);
                let _ = state
                    .window
                    .request_inner_size(LogicalSize::new(PREVIEW_WIDTH, PREVIEW_HEIGHT));
            }
        }

        WindowEvent::RedrawRequested => {
            render_preview(wasm_path, state);
        }

        _ => {}
    }
}

#[expect(clippy::too_many_lines)]
fn render_preview(wasm_path: &Path, state: &mut PreviewState) {
    // Hot-reload: recreate all 4 runtimes
    if state.pending_reload {
        state.pending_reload = false;
        let mut any_ok = false;
        for tile in &mut state.tiles {
            // FemtoVG's Framebuffer::Drop deletes the FBO passed to set_screen_target,
            // even though we created it externally. Recreate the FBO before the old
            // runtime is dropped so the new runtime gets a valid FBO.
            let Ok((fbo, texture)) = create_fbo(&state.gl, tile.pw, tile.ph, true) else {
                eprintln!("Reload failed ({}): could not create FBO", tile.label);
                continue;
            };
            let fbo_id = fbo.0.get();
            match create_runtime(wasm_path, &state.gl_config, tile.w, tile.h, fbo_id) {
                Ok(new_runtime) => {
                    tile.runtime = new_runtime; // drops old runtime → deletes old FBO
                    tile.fbo = fbo;
                    tile.texture = texture;
                    // Force a fresh render for the new runtime so constants
                    // or initial state changes show up immediately.
                    tile.ever_rendered = false;
                    tile.logged_dead = false;
                    any_ok = true;
                }
                Err(e) => {
                    // Clean up the FBO we just created since we won't use it
                    unsafe {
                        state.gl.delete_framebuffer(fbo);
                        state.gl.delete_texture(texture);
                    }
                    eprintln!("Reload failed ({}): {e}", tile.label);
                }
            }
        }
        if any_ok {
            let (major, minor, patch) = state.tiles[0].runtime.sdk_version();
            println!(
                "Reloaded: {} (SDK {major}.{minor}.{patch})",
                wasm_path.display()
            );
            let widget_name = wasm_path
                .file_stem()
                .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
            state
                .window
                .set_title(&format!("{widget_name} — SDK {major}.{minor}.{patch}"));
        }
    }

    let now = Instant::now();
    let delta_ms = now.duration_since(state.last_frame).as_millis() as u32;
    state.last_frame = now;

    let t0 = Instant::now();

    // Deliver async I/O to all tiles first (may trigger request_frame inside WASM)
    for tile in &mut state.tiles {
        tile.runtime.deliver_fetch_responses();
        tile.runtime.deliver_ws_messages();
        tile.runtime.deliver_socket_events();
        tile.runtime.deliver_mdns_events();
        tile.runtime.deliver_http_requests();
    }

    // Render stats panel into its dedicated FBO via its own FemtoVG renderer.
    let reload_clicked;
    {
        let renderer = &mut state.stats_renderer;
        let w = STATS_W as f32;
        let h = STATS_H as f32;
        renderer.begin_frame(STATS_W, STATS_H, 1.0);

        state.stats_interaction.begin_frame();
        reload_clicked = draw_stats_panel(
            renderer,
            &mut state.stats_interaction,
            w,
            h,
            &state.perf_overlay,
        );
        renderer.flush();
    }

    // Render each tile directly to its FBO (FemtoVG targets them via fbo_id)
    let interaction_pending = state.needs_render;
    let mut frame_timings = FrameTimings::default();
    let mut any_rendered = false;
    for (tile_idx, tile) in state.tiles.iter_mut().enumerate() {
        let needs_work = !tile.ever_rendered
            || tile.runtime.wants_next_frame()
            || tile.runtime.has_pending_fetches()
            || interaction_pending;
        if !needs_work && !state.pending_reload {
            continue; // FBO already holds the last good frame
        }
        tile.ever_rendered = true;

        any_rendered = true;
        let (tw, th) = (tile.w, tile.h);

        tile.runtime.renderer().begin_frame(tw, th, 1.0);
        match tile.runtime.render(delta_ms) {
            Ok(RenderStatus::Ok) => {
                tile.logged_dead = false;
            }
            Ok(RenderStatus::FuelExhausted) => {
                eprintln!("Fuel exhausted ({})", tile.label);
            }
            Ok(RenderStatus::Dead) => {
                if !tile.logged_dead {
                    eprintln!("Widget dead ({})", tile.label);
                    tile.logged_dead = true;
                }
            }
            Err(e) => {
                eprintln!("Render error ({}): {e}", tile.label);
            }
        }

        let flush_t0 = Instant::now();
        tile.runtime.renderer().flush();
        let flush_us = flush_t0.elapsed().as_micros() as u32;

        // Use FULL tile (index 0) as the representative for timings
        if tile_idx == 0 {
            frame_timings = tile.runtime.last_timings();
            frame_timings.flush_us = flush_us;
        }
    }

    // Blit cached checkerboard background (already at physical resolution)
    let sh = state.phys_h;
    blit_fbo_to_screen(
        &state.gl,
        state.checker_fbo,
        state.phys_w,
        state.phys_h,
        0,
        0,
        state.phys_w,
        state.phys_h,
        sh,
    );

    // Blit each tile FBO to screen — FBOs are logical, screen is physical
    let dpi = state.dpi_scale;
    for tile in &state.tiles {
        blit_fbo_to_screen(
            &state.gl,
            tile.fbo,
            tile.pw,
            tile.ph,
            scaled(tile.x, dpi),
            scaled(tile.y, dpi),
            scaled(tile.w, dpi),
            scaled(tile.h, dpi),
            sh,
        );
    }
    // Blit stats panel (logical FBO → physical screen position)
    blit_fbo_to_screen(
        &state.gl,
        state.stats_fbo,
        STATS_W,
        STATS_H,
        scaled(STATS_X, dpi),
        scaled(STATS_Y, dpi),
        scaled(STATS_W, dpi),
        scaled(STATS_H, dpi),
        sh,
    );

    // Reset = full WASM reload (same as hot-reload)
    if reload_clicked {
        state.pending_reload = true;
    }

    unsafe {
        state.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }

    let render_us = t0.elapsed().as_micros() as u32;
    state
        .perf_overlay
        .tick(render_us, any_rendered, frame_timings);

    if let Err(e) = state.gl_surface.swap_buffers(&state.gl_context) {
        eprintln!("Failed to swap buffers: {e}");
    }

    // Schedule next wake-up. `needs_render` is only set by user interactions
    // (mouse, scroll, keyboard). WebSocket/fetch activity is handled via the
    // I/O pre-pass — if messages arrive they trigger request_frame() in WASM.
    state.needs_render = false;
}

/// Blit an FBO to a position on the default framebuffer (screen).
///
/// Blit an FBO to a position on the default framebuffer (screen).
///
/// `src_w/src_h` = FBO dimensions (logical pixels).
/// `dst_x/dst_y/dst_w/dst_h` = destination on screen (physical pixels).
/// `screen_h` = physical height of the screen surface (for OpenGL Y-flip).
/// When src and dst sizes differ, `GL_LINEAR` provides upscale filtering.
#[expect(clippy::too_many_arguments, clippy::cast_possible_wrap)]
fn blit_fbo_to_screen(
    gl: &glow::Context,
    fbo: glow::Framebuffer,
    src_w: u32,
    src_h: u32,
    dst_x: u32,
    dst_y: u32,
    dst_w: u32,
    dst_h: u32,
    screen_h: u32,
) {
    unsafe {
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        // OpenGL Y is bottom-up, window Y is top-down
        let dy0 = screen_h as i32 - dst_y as i32 - dst_h as i32;
        let dy1 = screen_h as i32 - dst_y as i32;
        gl.blit_framebuffer(
            0,
            0,
            src_w as i32,
            src_h as i32,
            dst_x as i32,
            dy0,
            (dst_x + dst_w) as i32,
            dy1,
            glow::COLOR_BUFFER_BIT,
            glow::LINEAR,
        );
    }
}

/// Render a checkerboard pattern once into an FBO for later blitting.
#[expect(clippy::cast_possible_wrap)]
fn render_checkerboard_to_fbo(gl: &glow::Context, fbo: glow::Framebuffer, w: u32, h: u32) {
    const CELL: i32 = 16;
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.viewport(0, 0, w as i32, h as i32);

        // Fill with dark gray base
        gl.clear_color(0.10, 0.10, 0.10, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);

        // Draw lighter cells in checkerboard pattern
        gl.enable(glow::SCISSOR_TEST);
        gl.clear_color(0.16, 0.16, 0.16, 1.0);
        let cols = (w as i32 + CELL - 1) / CELL;
        let rows = (h as i32 + CELL - 1) / CELL;
        for row in 0..rows {
            for col in 0..cols {
                if (row + col) % 2 == 0 {
                    gl.scissor(col * CELL, row * CELL, CELL, CELL);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }
            }
        }
        gl.disable(glow::SCISSOR_TEST);

        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }
}

#[expect(clippy::cast_possible_wrap)]
/// Check if window coordinates are inside the stats panel, return local coords.
fn hit_test_stats(pos: (i32, i32)) -> Option<(i32, i32)> {
    let (mx, my) = pos;
    let sx = STATS_X as i32;
    let sy = STATS_Y as i32;
    if mx >= sx && mx < sx + STATS_W as i32 && my >= sy && my < sy + STATS_H as i32 {
        Some((mx - sx, my - sy))
    } else {
        None
    }
}

/// Find which tile contains the given window coordinates, returning (index, local_x, local_y).
#[expect(clippy::cast_possible_wrap)]
fn hit_test_tile(tiles: &[PreviewTile], pos: (i32, i32)) -> Option<(usize, i32, i32)> {
    let (mx, my) = pos;
    for (i, tile) in tiles.iter().enumerate() {
        let tx = tile.x as i32;
        let ty = tile.y as i32;
        if mx >= tx && mx < tx + tile.w as i32 && my >= ty && my < ty + tile.h as i32 {
            return Some((i, mx - tx, my - ty));
        }
    }
    None
}

/// Create an FBO with a color texture attachment and optional depth-stencil renderbuffer.
///
/// FemtoVG requires a stencil buffer for clipping and path rendering.  Tile FBOs
/// that serve as direct render targets must pass `stencil = true`.  Blit-only FBOs
/// (checkerboard, stats) can pass `false`.
#[expect(clippy::cast_possible_wrap)]
fn create_fbo(
    gl: &glow::Context,
    width: u32,
    height: u32,
    stencil: bool,
) -> Result<(glow::Framebuffer, glow::Texture)> {
    unsafe {
        let texture = gl
            .create_texture()
            .map_err(|e| anyhow::anyhow!("Failed to create texture: {e}"))?;
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
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );

        let fbo = gl
            .create_framebuffer()
            .map_err(|e| anyhow::anyhow!("Failed to create framebuffer: {e}"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );

        if stencil {
            let rbo = gl
                .create_renderbuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create renderbuffer: {e}"))?;
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
        }

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

// ── Helpers ────────────────────────────────────────────────────────

fn create_runtime(
    wasm_path: &Path,
    gl_config: &glutin::config::Config,
    width: u32,
    height: u32,
    fbo_id: u32,
) -> Result<WasmWidgetRuntime> {
    let wasm_bytes = std::fs::read(wasm_path).context("Failed to read WASM file")?;
    let gl_display = gl_config.display();
    unsafe {
        WasmWidgetRuntime::new(
            &wasm_bytes,
            |s| gl_display.get_proc_address(&CString::new(s).unwrap_or_default()),
            width,
            height,
            fbo_id,
            WasmWidgetRuntime::FUEL_PER_FRAME,
            FormatPreferences::default(),
        )
    }
    .context("Failed to create runtime")
}

fn setup_watcher(path: &Path) -> Result<(RecommendedWatcher, Receiver<()>)> {
    let (tx, rx) = channel();
    let target = path.canonicalize()?;
    let parent = target.parent().context("no parent directory")?.to_owned();
    let target_file_name = target.file_name().map(ToOwned::to_owned);

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, _>| {
            if let Ok(event) = res {
                let is_relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                );
                if !is_relevant {
                    return;
                }
                let targets_match = event.paths.iter().any(|p| {
                    if p == &target {
                        return true;
                    }
                    if let Some(name) = &target_file_name
                        && p.file_name() != Some(name.as_ref())
                    {
                        return false;
                    }
                    p.canonicalize().ok().as_ref() == Some(&target)
                });
                if targets_match {
                    let _ = tx.send(());
                }
            }
        },
        notify::Config::default(),
    )?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

// ── Stats overlay ──────────────────────────────────────────────────

/// Draw stats panel with reload button. Returns `true` if reload was clicked.
fn draw_stats_panel(
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    w: f32,
    h: f32,
    overlay: &PerfOverlay,
) -> bool {
    let pad = 8.0;
    let y = pad;

    // ── Row 1: reload button (testbed-only) ──
    let btn_sz = ButtonSize::Small;
    let btn_w = btn_sz.width(6, false);
    let btn_h = btn_sz.height();
    let reload_clicked = draw_button(
        renderer,
        interaction,
        "reload",
        "Reload WASM",
        w - pad - btn_w,
        y,
        btn_w,
        btn_h,
        ButtonStyle::Danger,
        btn_sz,
        0,
        false,
    );
    let y_offset = y + btn_h + 4.0;

    // ── Perf overlay (reusable) ──
    overlay.draw(renderer, w, h, y_offset);

    reload_clicked.0
}

// ── Perf report ───────────────────────────────────────────────────

fn write_perf_report(path: &Path, samples: &[FrameTimings]) {
    use std::io::Write;

    let n = samples.len() as f64;
    if n == 0.0 {
        eprintln!("No samples collected, skipping perf report");
        return;
    }

    let avg = |f: fn(&FrameTimings) -> u32| -> f64 {
        samples.iter().map(|s| f64::from(f(s))).sum::<f64>() / n
    };
    #[expect(clippy::cast_sign_loss)]
    let percentile = |f: fn(&FrameTimings) -> u32, pct: f64| -> u32 {
        let mut vals: Vec<u32> = samples.iter().map(f).collect();
        vals.sort_unstable();
        let idx = ((pct / 100.0) * (vals.len() - 1) as f64).round() as usize;
        vals[idx.min(vals.len() - 1)]
    };

    // Total frame time = wasm + flush (wasm includes tree+layout+render)
    let total_frame = |s: &FrameTimings| -> u32 { s.wasm_us + s.flush_us };
    let avg_frame = samples
        .iter()
        .map(|s| f64::from(total_frame(s)))
        .sum::<f64>()
        / n;
    let avg_fps_val = if avg_frame > 0.0 {
        1_000_000.0 / avg_frame
    } else {
        0.0
    };

    let anim_only_count = samples.iter().filter(|s| s.wasm_us == 0).count();
    let anim_only_pct = anim_only_count as f64 / n * 100.0;

    let json = format!(
        r#"{{
  "frames": {frames},
  "avg_fps": {avg_fps:.1},
  "avg_frame_us": {avg_frame:.0},
  "avg_wasm_us": {avg_wasm:.0},
  "avg_tree_us": {avg_tree:.0},
  "avg_layout_us": {avg_layout:.0},
  "avg_render_us": {avg_render:.0},
  "avg_flush_us": {avg_flush:.0},
  "p50_frame_us": {p50},
  "p95_frame_us": {p95},
  "p99_frame_us": {p99},
  "animation_only_pct": {anim_only_pct:.1},
  "samples": [{samples_json}
  ]
}}"#,
        frames = samples.len(),
        avg_fps = avg_fps_val,
        avg_frame = avg_frame,
        avg_wasm = avg(|s| s.wasm_us),
        avg_tree = avg(|s| s.deserialize_us),
        avg_layout = avg(|s| s.layout_us),
        avg_render = avg(|s| s.render_us),
        avg_flush = avg(|s| s.flush_us),
        p50 = percentile(total_frame, 50.0),
        p95 = percentile(total_frame, 95.0),
        p99 = percentile(total_frame, 99.0),
        anim_only_pct = anim_only_pct,
        samples_json = samples
            .iter()
            .map(|s| format!(
                "\n    {{\"wasm\":{},\"tree\":{},\"layout\":{},\"render\":{},\"flush\":{}}}",
                s.wasm_us, s.deserialize_us, s.layout_us, s.render_us, s.flush_us,
            ))
            .collect::<Vec<_>>()
            .join(","),
    );

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::File::create(path) {
        Ok(mut f) => {
            let _ = f.write_all(json.as_bytes());
            eprintln!("Perf report written to: {}", path.display());
        }
        Err(e) => eprintln!("Failed to write perf report: {e}"),
    }
}
