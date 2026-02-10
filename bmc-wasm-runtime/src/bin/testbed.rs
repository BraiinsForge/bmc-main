// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget development testbed with hot-reloading.
//!
//! Uses winit for windowing, glutin for OpenGL context, and FemtoVG for GPU rendering.

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
use winit::window::Window;

use bmc_wasm_runtime::WasmWidgetRuntime;
use bmc_wasm_runtime::interaction::TouchEvent;
use bmc_wasm_runtime::renderer::Renderer;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 480;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("WASM Widget Testbed");
        eprintln!("Usage: testbed <wasm_file> [--size WxH]");
        eprintln!();
        eprintln!("Example: testbed widget.wasm --size {DEFAULT_WIDTH}x{DEFAULT_HEIGHT}");
        std::process::exit(1);
    }

    let wasm_path = PathBuf::from(&args[1]);
    let (width, height) = parse_size(&args).unwrap_or((DEFAULT_WIDTH, DEFAULT_HEIGHT));

    println!("Loading widget from: {}", wasm_path.display());
    println!("Display size: {width}x{height}");

    let event_loop = EventLoop::new()?;
    let mut app = App {
        wasm_path,
        width,
        height,
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
    width: u32,
    height: u32,
    state: Option<AppState>,
    rss_after_gl_kb: Option<u64>,
    rss_after_runtime_kb: Option<u64>,
}

struct AppState {
    // runtime must drop before GL context (FemtoVG Canvas calls GL on drop)
    runtime: WasmWidgetRuntime,
    window: Window,
    gl_surface: glutin::surface::Surface<WindowSurface>,
    gl_context: glutin::context::PossiblyCurrentContext,
    gl_config: glutin::config::Config,
    _watcher: RecommendedWatcher,
    watcher_rx: Receiver<()>,
    last_frame: Instant,
    needs_render: bool,
    pending_reload: bool,
    mouse_pos: (i32, i32),
    mouse_down: bool,
    fps: FpsTracker,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title("WASM Widget Testbed")
            .with_inner_size(PhysicalSize::new(self.width, self.height));

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

        // No vsync — testbed measures uncapped GPU throughput.
        // Event-driven loop sleeps when idle, so no wasted CPU.
        let _ = gl_surface.set_swap_interval(&gl_context, SwapInterval::DontWait);

        self.rss_after_gl_kb = current_rss_kb();

        let runtime = create_runtime(&self.wasm_path, &gl_config, self.width, self.height)
            .expect("Failed to create runtime");

        self.rss_after_runtime_kb = current_rss_kb();

        let (major, minor, patch) = runtime.sdk_version();
        println!("Widget SDK version: {major}.{minor}.{patch}");
        let widget_name = self
            .wasm_path
            .file_stem()
            .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
        window.set_title(&format!("{widget_name} — SDK {major}.{minor}.{patch}"));

        let (watcher, watcher_rx) =
            setup_watcher(&self.wasm_path).expect("Failed to set up file watcher");

        self.state = Some(AppState {
            window,
            gl_surface,
            gl_context,
            gl_config,
            runtime,
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

    #[expect(clippy::too_many_lines)]
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };

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
                if state.mouse_down {
                    let (x, y) = state.mouse_pos;
                    state.runtime.push_touch_event(TouchEvent::Move { x, y });
                    state.needs_render = true;
                }
            }

            WindowEvent::MouseInput {
                state: btn_state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = state.mouse_pos;
                match btn_state {
                    ElementState::Pressed => {
                        state.mouse_down = true;
                        state.runtime.push_touch_event(TouchEvent::Down { x, y });
                        state.needs_render = true;
                    }
                    ElementState::Released => {
                        state.mouse_down = false;
                        state.runtime.push_touch_event(TouchEvent::Up { x, y });
                        state.needs_render = true;
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let delta_y = match delta {
                    // winit: positive y = content should move down = scroll up
                    // interaction: positive delta = scroll down
                    // Negate to match, and scale up for usable speed
                    MouseScrollDelta::LineDelta(_, y) => (-y * 30.0) as i32,
                    MouseScrollDelta::PixelDelta(pos) => -pos.y as i32,
                };
                if delta_y != 0 {
                    let (x, y) = state.mouse_pos;
                    state
                        .runtime
                        .push_touch_event(TouchEvent::Scroll { x, y, delta_y });
                    state.needs_render = true;
                }
            }

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    state.gl_surface.resize(
                        &state.gl_context,
                        NonZeroU32::new(size.width).unwrap(),
                        NonZeroU32::new(size.height).unwrap(),
                    );
                    state.needs_render = true;
                }
            }

            WindowEvent::RedrawRequested => {
                // Hot-reload
                if state.pending_reload {
                    state.pending_reload = false;
                    match create_runtime(&self.wasm_path, &state.gl_config, self.width, self.height)
                    {
                        Ok(new_runtime) => {
                            let (major, minor, patch) = new_runtime.sdk_version();
                            println!(
                                "Reloaded: {} (SDK {major}.{minor}.{patch})",
                                self.wasm_path.display()
                            );
                            let widget_name = self
                                .wasm_path
                                .file_stem()
                                .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
                            state
                                .window
                                .set_title(&format!("{widget_name} — SDK {major}.{minor}.{patch}"));
                            state.runtime = new_runtime;
                        }
                        Err(e) => eprintln!("Reload failed: {e}"),
                    }
                }

                let now = Instant::now();
                let delta_ms = now.duration_since(state.last_frame).as_millis() as u32;
                state.last_frame = now;

                let size = state.window.inner_size();
                let (w, h) = (size.width, size.height);
                if w == 0 || h == 0 {
                    return;
                }

                // Render
                let t0 = Instant::now();

                state.runtime.renderer().begin_frame(w, h);
                // Dark wine background
                state
                    .runtime
                    .renderer()
                    .fill_rect(0.0, 0.0, w as f32, h as f32, 0x50_18_38_FF);

                if let Err(e) = state.runtime.render(delta_ms) {
                    eprintln!("Render error: {e}");
                }

                let render_us = t0.elapsed().as_micros() as u32;
                state.fps.tick(render_us, state.needs_render);
                draw_stats(state.runtime.renderer(), w as f32, &state.fps);

                state.runtime.renderer().flush();
                state
                    .gl_surface
                    .swap_buffers(&state.gl_context)
                    .expect("Failed to swap buffers");

                state.needs_render = state.runtime.wants_next_frame();
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = &mut self.state else { return };

        // Check for hot-reload
        if state.watcher_rx.try_recv().is_ok() {
            while state.watcher_rx.try_recv().is_ok() {} // drain
            state.pending_reload = true;
        }

        if state.needs_render || state.pending_reload {
            event_loop.set_control_flow(ControlFlow::Poll);
            state.window.request_redraw();
        } else {
            // Poll periodically for hot-reload events
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
        }
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

fn parse_size(args: &[String]) -> Option<(u32, u32)> {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--size" {
            if let Some(size_str) = args.get(i + 1) {
                let parts: Vec<&str> = size_str.split('x').collect();
                if parts.len() == 2 {
                    let w: u32 = parts[0].parse().ok()?;
                    let h: u32 = parts[1].parse().ok()?;
                    return Some((w, h));
                }
            }
        }
    }
    None
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

fn draw_stats(renderer: &mut impl Renderer, w: f32, fps: &FpsTracker) {
    const CHART_H: f32 = 40.0;
    const BAR_W: f32 = 1.0;
    // Chart scale: 50ms full height (in microseconds)
    const SCALE_US: f32 = 50_000.0;
    let chart_w = fps.history.len() as f32 * BAR_W;
    let x0 = w - chart_w - 4.0;
    let y0 = 4.0_f32;

    // Background
    renderer.fill_rect(x0 - 2.0, y0, chart_w + 6.0, CHART_H + 4.0, 0x30_10_20_C0);

    // 16ms reference line (60fps target)
    let ref_y = y0 + CHART_H - (16_000.0 * CHART_H / SCALE_US);
    renderer.fill_rect(x0, ref_y, chart_w, 1.0, 0x50_30_40_FF);

    // Bars
    for (i, sample) in fps.samples().enumerate() {
        let bx = x0 + i as f32 * BAR_W;
        let bh = (sample.us as f32 * CHART_H / SCALE_US).min(CHART_H);
        let color = if sample.rendered {
            0x50_AA_50_FF
        } else {
            0x60_60_60_FF
        };
        renderer.fill_rect(bx, y0 + CHART_H - bh, BAR_W, bh, color);
    }

    // Text labels
    let avg_us = fps.avg_render_us();
    let avg_ms = avg_us as f32 / 1_000.0;
    let color = if avg_us > 16_000 {
        0xFF_60_60_FF
    } else {
        0xCC_CC_CC_FF
    };
    renderer.draw_text(&format!("{avg_ms:.1}ms"), x0 + 4.0, y0 + 2.0, 16.0, color);
    if fps.display_render > 0 {
        renderer.draw_text(
            &format!("{}fps", fps.display_render),
            x0 + 4.0,
            y0 + 20.0,
            16.0,
            0xAA_AA_AA_FF,
        );
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
