// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget development testbed with hot-reloading.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::integer_division
)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use anyhow::{Context, Result};
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use bmc_wasm_runtime::WasmWidgetRuntime;
use bmc_wasm_runtime::interaction::TouchEvent;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 480;

/// Tracks mouse state and converts to touch events.
struct InputState {
    mouse_down: bool,
    width: u32,
}

impl InputState {
    fn new(width: u32) -> Self {
        Self {
            mouse_down: false,
            width,
        }
    }

    /// Check if position is in the stats/chart overlay area (top-right).
    fn in_stats_area(&self, x: i32, y: i32) -> bool {
        x >= (self.width as i32 - 130) && y < 50
    }

    /// Process window input, push touch events to runtime. Returns true if there was input.
    fn handle(&mut self, window: &Window, runtime: &mut WasmWidgetRuntime) -> bool {
        let mut has_input = false;

        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Clamp) {
            let x = mx as i32;
            let y = my as i32;

            // Ignore interactions in stats overlay area
            if self.in_stats_area(x, y) {
                // Still track mouse state to avoid stuck states
                self.mouse_down = window.get_mouse_down(MouseButton::Left);
                return false;
            }

            let is_down = window.get_mouse_down(MouseButton::Left);
            if is_down && !self.mouse_down {
                runtime.push_touch_event(TouchEvent::Down { x, y });
                self.mouse_down = true;
                has_input = true;
            } else if !is_down && self.mouse_down {
                runtime.push_touch_event(TouchEvent::Up { x, y });
                self.mouse_down = false;
                has_input = true;
            } else if is_down {
                runtime.push_touch_event(TouchEvent::Move { x, y });
                has_input = true;
            }

            if let Some((_, scroll_y)) = window.get_scroll_wheel() {
                let delta_y = (scroll_y * 3.0) as i32;
                if delta_y != 0 {
                    runtime.push_touch_event(TouchEvent::Scroll { x, y, delta_y });
                    has_input = true;
                }
            }
        }

        has_input
    }
}

/// Frame timing sample for visualization.
#[derive(Clone, Copy)]
struct FrameSample {
    ms: u32,
    rendered: bool,
}

impl Default for FrameSample {
    fn default() -> Self {
        Self {
            ms: 16,
            rendered: false,
        }
    }
}

/// Tracks render/loop FPS and frame history for display.
struct FpsTracker {
    last_update: Instant,
    loop_count: u32,
    render_count: u32,
    pub display_loop: u32,
    pub display_render: u32,
    /// Rolling buffer of frame samples for chart
    pub history: [FrameSample; 120],
    history_idx: usize,
}

impl FpsTracker {
    fn new() -> Self {
        Self {
            last_update: Instant::now(),
            loop_count: 0,
            render_count: 0,
            display_loop: 0,
            display_render: 0,
            history: [FrameSample::default(); 120],
            history_idx: 0,
        }
    }

    fn tick(&mut self, delta_ms: u32, rendered: bool) {
        self.loop_count += 1;
        if rendered {
            self.render_count += 1;
        }
        if self.last_update.elapsed().as_secs() >= 1 {
            self.display_loop = self.loop_count;
            self.display_render = self.render_count;
            self.loop_count = 0;
            self.render_count = 0;
            self.last_update = Instant::now();
        }

        // Record frame sample
        self.history[self.history_idx] = FrameSample {
            ms: delta_ms,
            rendered,
        };
        self.history_idx = (self.history_idx + 1) % self.history.len();
    }

    /// Iterate samples from oldest to newest.
    fn samples(&self) -> impl Iterator<Item = &FrameSample> {
        let (a, b) = self.history.split_at(self.history_idx);
        b.iter().chain(a.iter())
    }

    /// Average frame time of rendered frames (in ms). Returns 0 if no renders.
    fn avg_render_ms(&self) -> u32 {
        let (sum, count) = self
            .history
            .iter()
            .filter(|s| s.rendered)
            .fold((0_u32, 0_u32), |(sum, count), s| (sum + s.ms, count + 1));
        if count > 0 { sum / count } else { 0 }
    }
}

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

    let wasm_bytes = std::fs::read(&wasm_path).context("Failed to read WASM file")?;
    let mut runtime =
        WasmWidgetRuntime::new(&wasm_bytes, width, height).context("Failed to create runtime")?;

    let (_watcher, watcher_rx) = setup_watcher(&wasm_path)?;

    let mut window = Window::new(
        "WASM Widget Testbed",
        width as usize,
        height as usize,
        WindowOptions::default(),
    )
    .context("Failed to create window")?;
    window.set_target_fps(60);

    let mut last_frame = Instant::now();
    let mut frame_buffer: Vec<u32> = vec![0; (width * height) as usize];
    let mut background: Vec<u32> = vec![0; (width * height) as usize];
    let mut needs_render = true;
    let mut input = InputState::new(width);
    let mut fps = FpsTracker::new();

    // Pre-render background once
    draw_background(&mut background, width, height);
    frame_buffer.copy_from_slice(&background);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Hot-reload check
        if watcher_rx.try_recv().is_ok() {
            while watcher_rx.try_recv().is_ok() {} // drain
            if let Some(new_runtime) = try_reload(&wasm_path, width, height) {
                runtime = new_runtime;
                input = InputState::new(width);
                needs_render = true;
            }
        }

        let now = Instant::now();
        let delta_ms = now.duration_since(last_frame).as_millis() as u32;
        last_frame = now;

        let has_input = input.handle(&window, &mut runtime);
        let rendered = needs_render || has_input;

        let render_ms = if rendered {
            let t0 = Instant::now();
            if let Err(e) = runtime.render(delta_ms) {
                eprintln!("Render error: {e}");
            }
            frame_buffer.copy_from_slice(&background);
            composite_overlay(&mut frame_buffer, runtime.get_overlay(), width, height);
            t0.elapsed().as_millis() as u32
        } else {
            0
        };

        fps.tick(render_ms, rendered);
        draw_frame_chart(&mut frame_buffer, width, &fps);

        window
            .update_with_buffer(&frame_buffer, width as usize, height as usize)
            .context("Failed to update window")?;

        needs_render = runtime.wants_next_frame();
    }

    drop(window);
    Ok(())
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

/// Draw a smooth diagonal gradient background in wine colors.
fn draw_background(buffer: &mut [u32], width: u32, height: u32) {
    // Wine color base: #662347 (R=0x66, G=0x23, B=0x47)
    // Gradient from darker to base wine
    const R_START: f32 = 0x50 as f32;
    const G_START: f32 = 0x18 as f32;
    const B_START: f32 = 0x38 as f32;
    const R_END: f32 = 0x70 as f32;
    const G_END: f32 = 0x28 as f32;
    const B_END: f32 = 0x50 as f32;

    for y in 0..height {
        for x in 0..width {
            let t = (x + y) as f32 / (width + height) as f32;
            let r = (R_START + t * (R_END - R_START)) as u8;
            let g = (G_START + t * (G_END - G_START)) as u8;
            let b = (B_START + t * (B_END - B_START)) as u8;
            buffer[(y * width + x) as usize] =
                0xFF_00_00_00 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
        }
    }
}

/// Set up file watcher for hot-reload.
/// Watches parent directory to handle file replacement (cargo deletes + renames).
fn setup_watcher(path: &Path) -> Result<(RecommendedWatcher, Receiver<()>)> {
    let (tx, rx) = channel();
    let target = path.canonicalize()?;
    let parent = target.parent().context("no parent directory")?.to_owned();
    let target_file_name = target.file_name().map(ToOwned::to_owned);

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, _>| {
            if let Ok(event) = res {
                let is_relevant_kind = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                );
                if !is_relevant_kind {
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

/// Try to reload WASM module, returning new runtime on success.
fn try_reload(path: &Path, width: u32, height: u32) -> Option<WasmWidgetRuntime> {
    match std::fs::read(path) {
        Ok(bytes) => match WasmWidgetRuntime::new(&bytes, width, height) {
            Ok(rt) => {
                println!("Reloaded: {}", path.display());
                Some(rt)
            }
            Err(e) => {
                eprintln!("Reload failed: {e}");
                None
            }
        },
        Err(e) => {
            eprintln!("Read failed: {e}");
            None
        }
    }
}

/// Composite RGBA overlay onto ARGB buffer.
fn composite_overlay(buffer: &mut [u32], overlay: &tiny_skia::Pixmap, width: u32, height: u32) {
    let overlay_data = overlay.data();

    for y in 0..height.min(overlay.height()) {
        for x in 0..width.min(overlay.width()) {
            let src_idx = ((y * overlay.width() + x) * 4) as usize;
            let dst_idx = (y * width + x) as usize;

            // tiny-skia uses premultiplied RGBA
            let src_r = overlay_data[src_idx];
            let src_g = overlay_data[src_idx + 1];
            let src_b = overlay_data[src_idx + 2];
            let src_a = overlay_data[src_idx + 3];

            if src_a == 0 {
                continue;
            }

            if src_a == 255 {
                // Fully opaque - just copy
                buffer[dst_idx] =
                    0xFF_00_00_00 | ((src_r as u32) << 16) | ((src_g as u32) << 8) | (src_b as u32);
            } else {
                // Alpha blend (source is premultiplied)
                let dst = buffer[dst_idx];
                let dst_r = ((dst >> 16) & 0xFF) as u8;
                let dst_g = ((dst >> 8) & 0xFF) as u8;
                let dst_b = (dst & 0xFF) as u8;

                let inv_alpha = 255 - src_a;
                let out_r = src_r.saturating_add(((dst_r as u16 * inv_alpha as u16) / 255) as u8);
                let out_g = src_g.saturating_add(((dst_g as u16 * inv_alpha as u16) / 255) as u8);
                let out_b = src_b.saturating_add(((dst_b as u16 * inv_alpha as u16) / 255) as u8);

                buffer[dst_idx] =
                    0xFF_00_00_00 | ((out_r as u32) << 16) | ((out_g as u32) << 8) | (out_b as u32);
            }
        }
    }
}

/// Draw frame time chart with avg ms overlay.
fn draw_frame_chart(buffer: &mut [u32], width: u32, fps: &FpsTracker) {
    const CHART_HEIGHT: u32 = 40;
    const BAR_WIDTH: u32 = 1;
    let chart_width = fps.history.len() as u32 * BAR_WIDTH;
    let start_x = width.saturating_sub(chart_width + 4);
    let start_y = 4_u32;

    // Clear chart area
    for y in 0..CHART_HEIGHT + 4 {
        for x in start_x.saturating_sub(2)..width {
            let idx = ((start_y + y) * width + x) as usize;
            if idx < buffer.len() {
                buffer[idx] = 0xFF_30_10_20; // darker wine
            }
        }
    }

    // Draw 16ms reference line (60fps target)
    let ref_y = start_y + CHART_HEIGHT - (16 * CHART_HEIGHT / 50).min(CHART_HEIGHT);
    for x in start_x..width.saturating_sub(4) {
        let idx = (ref_y * width + x) as usize;
        if idx < buffer.len() {
            buffer[idx] = 0xFF_50_30_40; // dim line
        }
    }

    // Draw bars
    for (i, sample) in fps.samples().enumerate() {
        let x = start_x + i as u32 * BAR_WIDTH;
        let bar_h = (sample.ms * CHART_HEIGHT / 50).min(CHART_HEIGHT);
        let color = if sample.rendered {
            0xFF_50_AA_50 // green for rendered
        } else {
            0xFF_60_60_60 // gray for idle
        };

        for dy in 0..bar_h {
            let y = start_y + CHART_HEIGHT - 1 - dy;
            let idx = (y * width + x) as usize;
            if idx < buffer.len() {
                buffer[idx] = color;
            }
        }
    }

    // Draw avg render ms on top of chart
    let avg_ms = fps.avg_render_ms();
    if avg_ms > 0 {
        draw_number(buffer, width, start_x + 4, start_y + 2, avg_ms, avg_ms > 16);
    }
}

/// Draw a number using 3x5 bitmap font scaled 3x. Red if `warning` is true.
fn draw_number(buffer: &mut [u32], width: u32, x: u32, y: u32, value: u32, warning: bool) {
    const DIGITS: [u16; 10] = [
        0b111_101_101_101_111, // 0
        0b010_110_010_010_111, // 1
        0b111_001_111_100_111, // 2
        0b111_001_111_001_111, // 3
        0b101_101_111_001_001, // 4
        0b111_100_111_001_111, // 5
        0b111_100_111_101_111, // 6
        0b111_001_001_001_001, // 7
        0b111_101_111_101_111, // 8
        0b111_101_111_001_111, // 9
    ];

    const SCALE: u32 = 3;
    let text = format!("{value}");
    let color = if warning {
        0xFF_FF_60_60
    } else {
        0xFF_AA_AA_AA
    };

    for (i, ch) in text.chars().enumerate() {
        let bits = DIGITS.get(ch as usize - '0' as usize).copied().unwrap_or(0);
        let ox = x + i as u32 * (3 * SCALE + SCALE);

        for row in 0..5_u32 {
            for col in 0..3_u32 {
                let bit_idx = (4 - row) * 3 + (2 - col);
                if (bits >> bit_idx) & 1 == 1 {
                    for sy in 0..SCALE {
                        for sx in 0..SCALE {
                            let px = ox + col * SCALE + sx;
                            let py = y + row * SCALE + sy;
                            let idx = (py * width + px) as usize;
                            if idx < buffer.len() {
                                buffer[idx] = color;
                            }
                        }
                    }
                }
            }
        }
    }
}
