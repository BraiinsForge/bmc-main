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
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowButtons};

use bmc_wasm_protocol::FormatPreferences;
use bmc_wasm_runtime::colors::*;
use bmc_wasm_runtime::components::{ButtonSize, ButtonStyle, draw_button};
use bmc_wasm_runtime::interaction::{InteractionState, TouchEvent};
use bmc_wasm_runtime::renderer::Renderer;
use bmc_wasm_runtime::{FrameTimings, RenderStatus, WasmWidgetRuntime};

// Layout constants
const PREVIEW_GAP: u32 = 8;
const PREVIEW_MARGIN: u32 = 8;
// Inner width = max(1280, 638+8+638) = 1284
const PREVIEW_WIDTH: u32 = PREVIEW_MARGIN + 1284 + PREVIEW_MARGIN; // 1300
// Inner height = 480+8+max(480, 238+8+238) = 480+8+484 = 972
const PREVIEW_HEIGHT: u32 = PREVIEW_MARGIN + 972 + PREVIEW_MARGIN; // 988

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
    label: &'static str,
    fbo: glow::Framebuffer,
    _texture: glow::Texture,
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
    fps: FpsTracker,
    stats_interaction: InteractionState,
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
        let win_size = PhysicalSize::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
        let window_attrs = Window::default_attributes()
            .with_title("WASM Widget Testbed")
            .with_inner_size(win_size)
            .with_min_inner_size(win_size)
            .with_max_inner_size(win_size)
            .with_resizable(false)
            .with_enabled_buttons(WindowButtons::CLOSE | WindowButtons::MINIMIZE);

        let template = ConfigTemplateBuilder::new().with_alpha_size(8);
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

        let (watcher, watcher_rx) =
            setup_watcher(&self.wasm_path).context("Failed to set up file watcher")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_display
                    .get_proc_address(&CString::new(s).unwrap_or_default())
                    .cast()
            })
        };

        let mut tiles = Vec::with_capacity(4);
        for &(x, y, w, h, label) in &TILE_DEFS {
            let runtime = create_runtime(&self.wasm_path, &gl_config, w, h)
                .context("Failed to create runtime")?;
            let (fbo, texture) = create_fbo(&gl, w, h)?;
            tiles.push(PreviewTile {
                runtime,
                x,
                y,
                w,
                h,
                label,
                fbo,
                _texture: texture,
                logged_dead: false,
                ever_rendered: false,
            });
        }

        let (checker_fbo, checker_texture) = create_fbo(&gl, PREVIEW_WIDTH, PREVIEW_HEIGHT)?;
        render_checkerboard_to_fbo(&gl, checker_fbo, PREVIEW_WIDTH, PREVIEW_HEIGHT);
        let (stats_fbo, stats_texture) = create_fbo(&gl, STATS_W, STATS_H)?;

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
            fps: FpsTracker::new(),
            stats_interaction: InteractionState::new(),
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
                let last = state.fps.history[(state.fps.history_idx + state.fps.history.len() - 1)
                    % state.fps.history.len()];
                state.perf_samples.push(last.timings);
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

        if s.needs_render || s.pending_reload {
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
            state.mouse_pos = (position.x as i32, position.y as i32);
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
            // is_maximized unreliable — just force size back unconditionally)
            if size.width != PREVIEW_WIDTH || size.height != PREVIEW_HEIGHT {
                state.window.set_maximized(false);
                let _ = state
                    .window
                    .request_inner_size(PhysicalSize::new(PREVIEW_WIDTH, PREVIEW_HEIGHT));
            }
        }

        WindowEvent::RedrawRequested => {
            render_preview(wasm_path, state);
        }

        _ => {}
    }
}

#[expect(clippy::too_many_lines, clippy::cast_possible_wrap)]
fn render_preview(wasm_path: &Path, state: &mut PreviewState) {
    // Hot-reload: recreate all 4 runtimes
    if state.pending_reload {
        state.pending_reload = false;
        let mut any_ok = false;
        for tile in &mut state.tiles {
            match create_runtime(wasm_path, &state.gl_config, tile.w, tile.h) {
                Ok(new_runtime) => {
                    tile.runtime = new_runtime;
                    any_ok = true;
                }
                Err(e) => eprintln!("Reload failed ({}): {e}", tile.label),
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

    // Render each tile to its FBO — skip tiles with no pending work
    let mut frame_timings = FrameTimings::default();
    let mut any_rendered = false;
    for (tile_idx, tile) in state.tiles.iter_mut().enumerate() {
        let needs_work = !tile.ever_rendered
            || tile.runtime.wants_next_frame()
            || tile.runtime.has_pending_fetches();
        if !needs_work && !state.pending_reload {
            continue; // FBO already holds the last good frame
        }
        tile.ever_rendered = true;

        any_rendered = true;
        let (tw, th) = (tile.w, tile.h);

        tile.runtime.renderer().begin_frame(tw, th);

        tile.runtime.deliver_fetch_responses();
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

        // Copy rendered content from default FB to tile's FBO
        let fbo = tile.fbo;
        unsafe {
            state.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            state.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fbo));
            state.gl.blit_framebuffer(
                0,
                0,
                tw as i32,
                th as i32,
                0,
                0,
                tw as i32,
                th as i32,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
        }
    }

    // Render stats panel into its FBO (reuse FULL tile's renderer)
    let reload_clicked;
    {
        let renderer = state.tiles[0].runtime.renderer();
        let w = STATS_W as f32;
        let h = STATS_H as f32;
        renderer.begin_frame(STATS_W, STATS_H);

        state.stats_interaction.begin_frame();
        reload_clicked = draw_stats_panel(renderer, &mut state.stats_interaction, w, h, &state.fps);
        renderer.flush();
        unsafe {
            state.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            state
                .gl
                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(state.stats_fbo));
            state.gl.blit_framebuffer(
                0,
                0,
                STATS_W as i32,
                STATS_H as i32,
                0,
                0,
                STATS_W as i32,
                STATS_H as i32,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
        }
    }

    // Blit cached checkerboard background
    blit_fbo_to_screen(
        &state.gl,
        state.checker_fbo,
        0,
        0,
        PREVIEW_WIDTH,
        PREVIEW_HEIGHT,
    );

    // Blit each tile FBO to its correct position on screen
    for tile in &state.tiles {
        blit_fbo_to_screen(&state.gl, tile.fbo, tile.x, tile.y, tile.w, tile.h);
    }
    // Blit stats panel
    blit_fbo_to_screen(
        &state.gl,
        state.stats_fbo,
        STATS_X,
        STATS_Y,
        STATS_W,
        STATS_H,
    );

    // Reset = full WASM reload (same as hot-reload)
    if reload_clicked {
        state.pending_reload = true;
    }

    unsafe {
        state.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }

    let render_us = t0.elapsed().as_micros() as u32;
    state.fps.tick(render_us, any_rendered, frame_timings);

    if let Err(e) = state.gl_surface.swap_buffers(&state.gl_context) {
        eprintln!("Failed to swap buffers: {e}");
    }

    // Request immediate redraw only for tiles that want a frame NOW (no delay)
    state.needs_render = state.tiles.iter().any(|t| {
        t.runtime.has_pending_fetches()
            || (t.runtime.wants_next_frame() && t.runtime.next_frame_delay().is_none())
    });
}

/// Blit an FBO to a position on the default framebuffer (screen).
#[expect(clippy::cast_possible_wrap)]
fn blit_fbo_to_screen(gl: &glow::Context, fbo: glow::Framebuffer, x: u32, y: u32, w: u32, h: u32) {
    unsafe {
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        // OpenGL Y is bottom-up, window Y is top-down
        let dst_y0 = PREVIEW_HEIGHT as i32 - y as i32 - h as i32;
        let dst_y1 = PREVIEW_HEIGHT as i32 - y as i32;
        gl.blit_framebuffer(
            0,
            0,
            w as i32,
            h as i32,
            x as i32,
            dst_y0,
            (x + w) as i32,
            dst_y1,
            glow::COLOR_BUFFER_BIT,
            glow::NEAREST,
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

/// Create an FBO with a color texture attachment.
#[expect(clippy::cast_possible_wrap)]
fn create_fbo(
    gl: &glow::Context,
    width: u32,
    height: u32,
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
) -> Result<WasmWidgetRuntime> {
    let wasm_bytes = std::fs::read(wasm_path).context("Failed to read WASM file")?;
    let gl_display = gl_config.display();
    unsafe {
        WasmWidgetRuntime::new(
            &wasm_bytes,
            |s| gl_display.get_proc_address(&CString::new(s).unwrap_or_default()),
            width,
            height,
            0,
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

// Component colors for timing breakdown
const COL_WASM: u32 = 0x6A_9F_D8_FF; // blue — wasmi interpreter
const COL_TREE: u32 = 0xE0_9A_50_FF; // orange — tree deserialization/parsing
const COL_LAYOUT: u32 = 0xCC_CC_50_FF; // yellow — Taffy layout
const COL_RENDER: u32 = 0x50_CC_50_FF; // green — tree render
const COL_FLUSH: u32 = 0xCC_50_CC_FF; // purple — GPU flush

/// Draw stats panel with reload button. Returns `true` if reload was clicked.
fn draw_stats_panel(
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    w: f32,
    h: f32,
    fps: &FpsTracker,
) -> bool {
    let pad = 8.0;
    let mut y = pad;

    // ── Row 1: reload button ──
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
    );
    y += btn_h + 4.0;

    // ── Row 2: avg ms + fps ──
    let avg_us = fps.avg_render_us();
    let avg_ms = avg_us as f32 / 1_000.0;
    let ms_color = if avg_us > 16_000 { RED_50 } else { GREEN_30 };
    let ms_text = format!("{avg_ms:.1}ms");
    renderer.draw_text(&ms_text, pad, y, 13.0, ms_color);
    if fps.display_render > 0 {
        let ms_w = renderer.measure_text(&ms_text, 13.0);
        let fps_text = format!("{}fps", fps.display_render);
        renderer.draw_text(&fps_text, pad + ms_w + 8.0, y, 13.0, GRAY_40);
    }
    y += 18.0;

    // ── Chart ──
    let chart_x = pad;
    let chart_top = y;
    let legend_font = 12.0;
    let legend_h = legend_font + 6.0;
    let chart_bottom = h - pad - legend_h;
    let chart_h = chart_bottom - chart_top;
    let chart_w = w - pad * 2.0;
    let bar_w = chart_w / fps.history.len() as f32;
    let scale_us = 20_000.0_f32;

    // Gridlines (drawn before bars so bars paint over them)
    let axis_font = 10.0;
    for &grid_us in &[4_000, 8_000, 16_000] {
        let gy = chart_top + chart_h - (grid_us as f32 * chart_h / scale_us);
        if gy > chart_top && gy < chart_bottom {
            renderer.fill_rect(chart_x, gy, chart_w, 1.0, GRAY_90);
        }
    }

    // Bars — snap to pixel grid to avoid subpixel gaps
    for (i, sample) in fps.samples().enumerate() {
        let bx = (chart_x + i as f32 * bar_w).floor();
        let bx_next = (chart_x + (i as f32 + 1.0) * bar_w)
            .floor()
            .min(chart_x + chart_w);
        let bw = bx_next - bx;
        if !sample.rendered {
            let bh = (sample.us as f32 * chart_h / scale_us).min(chart_h);
            renderer.fill_rect(bx, chart_top + chart_h - bh, bw, bh, GRAY_80);
            continue;
        }
        let t = &sample.timings;
        let segments: [(u32, u32); 5] = [
            (t.flush_us, COL_FLUSH),
            (t.render_us, COL_RENDER),
            (t.layout_us, COL_LAYOUT),
            (t.deserialize_us, COL_TREE),
            (
                t.wasm_us
                    .saturating_sub(t.deserialize_us + t.layout_us + t.render_us),
                COL_WASM,
            ),
        ];
        let mut y_off = 0.0_f32;
        for (us, col) in segments {
            let bh = (us as f32 * chart_h / scale_us).min(chart_h - y_off);
            if bh > 0.5 {
                renderer.fill_rect(bx, chart_top + chart_h - y_off - bh, bw, bh, col);
            }
            y_off += bh;
        }
    }

    // Axis tick labels (drawn on top of bars with black background)
    let tick_pad = 2.0;
    for &grid_us in &[4_000, 8_000, 16_000] {
        let gy = chart_top + chart_h - (grid_us as f32 * chart_h / scale_us);
        if gy > chart_top && gy < chart_bottom {
            let label = format!("{}", grid_us / 1_000);
            let lw = renderer.measure_text(&label, axis_font);
            let lx = chart_x + chart_w - lw - tick_pad;
            let ly = gy - axis_font - 1.0;
            renderer.fill_rect(
                lx - tick_pad,
                ly,
                lw + tick_pad * 2.0,
                axis_font + 2.0,
                BLACK,
            );
            renderer.draw_text(&label, lx, ly, axis_font, GRAY_60);
        }
    }

    // ── Legend (below chart) ──
    let avg = fps.avg_timings();
    let legend_y = chart_bottom + 4.0;
    let mut lx = pad;
    for (label, us, col) in [
        ("WASM", avg.wasm_us, COL_WASM),
        ("Tree", avg.deserialize_us, COL_TREE),
        ("Lay", avg.layout_us, COL_LAYOUT),
        ("RNDR", avg.render_us, COL_RENDER),
        ("GPU", avg.flush_us, COL_FLUSH),
    ] {
        let txt = format!("{label} {:.1}", us as f32 / 1_000.0);
        renderer.draw_text(&txt, lx, legend_y, legend_font, col);
        lx += renderer.measure_text(&txt, legend_font) + 6.0;
    }

    reload_clicked
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
        samples.iter().map(|s| f(s) as f64).sum::<f64>() / n
    };
    let percentile = |f: fn(&FrameTimings) -> u32, pct: f64| -> u32 {
        let mut vals: Vec<u32> = samples.iter().map(f).collect();
        vals.sort_unstable();
        let idx = ((pct / 100.0) * (vals.len() - 1) as f64).round() as usize;
        vals[idx.min(vals.len() - 1)]
    };

    // Total frame time = wasm + flush (wasm includes tree+layout+render)
    let total_frame = |s: &FrameTimings| -> u32 { s.wasm_us + s.flush_us };
    let avg_frame = samples.iter().map(|s| total_frame(s) as f64).sum::<f64>() / n;
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

// ── FPS tracking ───────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct FrameSample {
    us: u32,
    rendered: bool,
    timings: FrameTimings,
}

impl Default for FrameSample {
    fn default() -> Self {
        Self {
            us: 16_000,
            rendered: false,
            timings: FrameTimings::default(),
        }
    }
}

struct FpsTracker {
    last_update: Instant,
    loop_count: u32,
    render_count: u32,
    display_render: u32,
    history: [FrameSample; 120],
    history_idx: usize,
}

impl FpsTracker {
    fn new() -> Self {
        Self {
            last_update: Instant::now(),
            loop_count: 0,
            render_count: 0,
            display_render: 0,
            history: [FrameSample::default(); 120],
            history_idx: 0,
        }
    }

    fn tick(&mut self, us: u32, rendered: bool, timings: FrameTimings) {
        self.loop_count += 1;
        if rendered {
            self.render_count += 1;
        }
        if self.last_update.elapsed().as_secs() >= 1 {
            self.display_render = self.render_count;
            self.loop_count = 0;
            self.render_count = 0;
            self.last_update = Instant::now();
        }
        self.history[self.history_idx] = FrameSample {
            us,
            rendered,
            timings,
        };
        self.history_idx = (self.history_idx + 1) % self.history.len();
    }

    fn samples(&self) -> impl Iterator<Item = &FrameSample> {
        let (a, b) = self.history.split_at(self.history_idx);
        b.iter().chain(a.iter())
    }

    /// Average render time in microseconds.
    fn avg_render_us(&self) -> u32 {
        let (sum, count) = self
            .history
            .iter()
            .filter(|s| s.rendered)
            .fold((0_u32, 0_u32), |(sum, count), s| (sum + s.us, count + 1));
        if count > 0 { sum / count } else { 0 }
    }

    /// Average per-component timings across rendered frames.
    fn avg_timings(&self) -> FrameTimings {
        let rendered: Vec<_> = self.history.iter().filter(|s| s.rendered).collect();
        let n = rendered.len() as u32;
        if n == 0 {
            return FrameTimings::default();
        }
        FrameTimings {
            wasm_us: rendered.iter().map(|s| s.timings.wasm_us).sum::<u32>() / n,
            deserialize_us: rendered
                .iter()
                .map(|s| s.timings.deserialize_us)
                .sum::<u32>()
                / n,
            layout_us: rendered.iter().map(|s| s.timings.layout_us).sum::<u32>() / n,
            render_us: rendered.iter().map(|s| s.timings.render_us).sum::<u32>() / n,
            flush_us: rendered.iter().map(|s| s.timings.flush_us).sum::<u32>() / n,
        }
    }
}
