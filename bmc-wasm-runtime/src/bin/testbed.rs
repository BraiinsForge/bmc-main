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

use bmc_wasm_runtime::WasmWidgetRuntime;
use bmc_wasm_runtime::interaction::TouchEvent;
use bmc_wasm_runtime::renderer::Renderer;

// Layout constants
const PREVIEW_GAP: u32 = 8;
const PREVIEW_MARGIN: u32 = 8;
// Inner width = max(1280, 638+8+638) = 1284
const PREVIEW_WIDTH: u32 = PREVIEW_MARGIN + 1284 + PREVIEW_MARGIN; // 1300
// Inner height = 480+8+max(480, 238+8+238) = 480+8+484 = 972
const PREVIEW_HEIGHT: u32 = PREVIEW_MARGIN + 972 + PREVIEW_MARGIN; // 988

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("WASM Widget Testbed");
        eprintln!("Usage: testbed <wasm_file>");
        std::process::exit(1);
    }

    let wasm_path = PathBuf::from(&args[1]);

    println!("Loading widget from: {}", wasm_path.display());
    println!("Display size: {PREVIEW_WIDTH}x{PREVIEW_HEIGHT} (4 sizes)");

    let event_loop = EventLoop::new()?;
    let mut app = App {
        wasm_path,
        state: None,
        rss_after_gl_kb: None,
        rss_after_runtime_kb: None,
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

impl ApplicationHandler for App {
    #[expect(clippy::too_many_lines)]
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

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
                    .unwrap()
            })
            .expect("Failed to build display");

        let window = window.expect("Failed to create window");
        let gl_display = gl_config.display();
        let raw_handle = window.window_handle().unwrap().as_raw();

        let context_attrs = ContextAttributesBuilder::new().build(Some(raw_handle));
        let gl_context = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .expect("Failed to create GL context")
        };

        let size = window.inner_size();
        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw_handle,
            NonZeroU32::new(size.width.max(1)).unwrap(),
            NonZeroU32::new(size.height.max(1)).unwrap(),
        );
        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .expect("Failed to create GL surface")
        };

        let gl_context = gl_context
            .make_current(&gl_surface)
            .expect("Failed to make GL context current");

        if let Err(e) = gl_surface.set_swap_interval(&gl_context, SwapInterval::DontWait) {
            eprintln!("Warning: failed to disable vsync: {e}");
        }

        self.rss_after_gl_kb = current_rss_kb();

        let (watcher, watcher_rx) =
            setup_watcher(&self.wasm_path).expect("Failed to set up file watcher");

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                let c = CString::new(s).unwrap();
                gl_display.get_proc_address(&c).cast()
            })
        };

        let mut tiles = Vec::with_capacity(4);
        for &(x, y, w, h, label) in &TILE_DEFS {
            let runtime = create_runtime(&self.wasm_path, &gl_config, w, h)
                .expect("Failed to create runtime");
            let (fbo, texture) = create_fbo(&gl, w, h);
            tiles.push(PreviewTile {
                runtime,
                x,
                y,
                w,
                h,
                label,
                fbo,
                _texture: texture,
            });
        }

        let (checker_fbo, checker_texture) = create_fbo(&gl, PREVIEW_WIDTH, PREVIEW_HEIGHT);
        render_checkerboard_to_fbo(&gl, checker_fbo, PREVIEW_WIDTH, PREVIEW_HEIGHT);
        let (stats_fbo, stats_texture) = create_fbo(&gl, STATS_W, STATS_H);

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
        });
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Some(state) = &mut self.state {
            handle_preview_event(&self.wasm_path, state, event_loop, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(s) = &mut self.state else { return };

        if s.watcher_rx.try_recv().is_ok() {
            while s.watcher_rx.try_recv().is_ok() {}
            s.pending_reload = true;
        }

        let (needs_render, pending_reload, window) = (s.needs_render, s.pending_reload, &s.window);

        if needs_render || pending_reload {
            event_loop.set_control_flow(ControlFlow::Poll);
            window.request_redraw();
        } else {
            // Still redraw periodically for overlay updates and hot-reload checks
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
            window.request_redraw();
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

    // Render each tile to its FBO via the default framebuffer
    for tile in &mut state.tiles {
        let (tw, th) = (tile.w, tile.h);

        // FemtoVG renders to the default framebuffer at (0, 0, tw, th)
        tile.runtime.renderer().begin_frame(tw, th);

        tile.runtime.deliver_fetch_responses();
        if let Err(e) = tile.runtime.render(delta_ms) {
            eprintln!("Render error ({}): {e}", tile.label);
        }

        tile.runtime.renderer().flush();

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
    {
        let renderer = state.tiles[0].runtime.renderer();
        let w = STATS_W as f32;
        let h = STATS_H as f32;
        renderer.begin_frame(STATS_W, STATS_H);

        // Subtle dark background — begin_frame already clears to black,
        // fill with slightly lighter to distinguish from widget backgrounds
        renderer.fill_rect(0.0, 0.0, w, h, 0x12_12_12_FF);
        // Inset rounded panel
        let pad = 12.0;
        renderer.fill_rounded_rect(pad, pad, w - pad * 2.0, h - pad * 2.0, 6.0, 0x1E_1E_1E_FF);

        draw_preview_stats(renderer, w, h, &state.fps);
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

    unsafe {
        state.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }

    let render_us = t0.elapsed().as_micros() as u32;
    // Always mark as rendered — we draw all 4 tiles every frame
    state.fps.tick(render_us, true);

    state
        .gl_surface
        .swap_buffers(&state.gl_context)
        .expect("Failed to swap buffers");

    state.needs_render = state
        .tiles
        .iter()
        .any(|t| t.runtime.wants_next_frame() || t.runtime.has_pending_fetches());
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
fn create_fbo(gl: &glow::Context, width: u32, height: u32) -> (glow::Framebuffer, glow::Texture) {
    unsafe {
        let texture = gl.create_texture().unwrap();
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

        let fbo = gl.create_framebuffer().unwrap();
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

        (fbo, texture)
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
            |s| {
                let c = CString::new(s).unwrap();
                gl_display.get_proc_address(&c)
            },
            width,
            height,
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

/// Draw a stats panel for the empty area right of the SMALL tile.
/// Content is inset to fit inside the rounded panel (outer=12, inner=8 → 20px from edge).
fn draw_preview_stats(renderer: &mut impl Renderer, w: f32, h: f32, fps: &FpsTracker) {
    // Content area inside the inset rounded panel (12px outer + 8px inner padding)
    let inset = 20.0;
    let cw = w - inset * 2.0;
    let ch = h - inset * 2.0;

    // Header
    renderer.draw_text("Performance", inset, inset, 14.0, 0x88_88_88_FF);

    // Chart fills available width, leaves room for header and text
    let bar_w = 2.0;
    let chart_w = (fps.history.len() as f32 * bar_w).min(cw);
    let chart_top = inset + 24.0;
    let chart_h = ch - 24.0 - 36.0; // header + bottom text
    let scale_us = 50_000.0_f32;
    let x0 = inset + (cw - chart_w) / 2.0;

    // 16ms reference line
    let ref_y = chart_top + chart_h - (16_000.0 * chart_h / scale_us);
    renderer.fill_rect(x0, ref_y, chart_w, 1.0, 0x44_44_44_FF);
    renderer.draw_text("16ms", x0 + chart_w + 4.0, ref_y - 6.0, 10.0, 0x66_66_66_FF);

    // Bars
    for (i, sample) in fps.samples().enumerate() {
        let bx = x0 + i as f32 * bar_w;
        let bh = (sample.us as f32 * chart_h / scale_us).min(chart_h);
        let color = if sample.rendered {
            0x50_AA_50_FF
        } else {
            0x44_44_44_FF
        };
        renderer.fill_rect(bx, chart_top + chart_h - bh, bar_w - 0.5, bh, color);
    }

    // Stats text below chart
    let text_y = chart_top + chart_h + 12.0;
    let avg_us = fps.avg_render_us();
    let avg_ms = avg_us as f32 / 1_000.0;
    let ms_color = if avg_us > 16_000 {
        0xFF_60_60_FF
    } else {
        0xAA_CC_AA_FF
    };
    renderer.draw_text(&format!("{avg_ms:.1}ms avg"), inset, text_y, 18.0, ms_color);
    if fps.display_render > 0 {
        let fps_text = format!("{}fps", fps.display_render);
        let fps_w = renderer.measure_text(&fps_text, 18.0);
        renderer.draw_text(&fps_text, w - inset - fps_w, text_y, 18.0, 0xAA_AA_AA_FF);
    }
}

// ── FPS tracking ───────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct FrameSample {
    us: u32,
    rendered: bool,
}

impl Default for FrameSample {
    fn default() -> Self {
        Self {
            us: 16_000,
            rendered: false,
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

    fn tick(&mut self, us: u32, rendered: bool) {
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
        self.history[self.history_idx] = FrameSample { us, rendered };
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
}
