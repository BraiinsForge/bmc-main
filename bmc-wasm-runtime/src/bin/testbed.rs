// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget development testbed with hot-reloading.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::integer_division
)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use bmc_wasm_runtime::WasmWidgetRuntime;
use bmc_wasm_runtime::interaction::TouchEvent;

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

    // Limit to ~60 FPS
    window.set_target_fps(60);

    let mut last_frame = Instant::now();
    let mut mouse_down = false;
    let mut frame_buffer: Vec<u32> = vec![0; (width * height) as usize];
    let mut frame_times: [f32; 30] = [16.67; 30];
    let mut frame_idx = 0_usize;

    // Draw TV test pattern as background
    draw_background(&mut frame_buffer, width, height);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Check for file changes (hot-reload) - drain all pending events
        let mut should_reload = false;
        while watcher_rx.try_recv().is_ok() {
            should_reload = true;
        }
        if should_reload {
            if let Some(new_runtime) = try_reload(&wasm_path, width, height) {
                runtime = new_runtime;
                // Reset mouse state so the next press emits a fresh Down event.
                mouse_down = false;
            }
        }

        let now = Instant::now();
        let delta_ms = now.duration_since(last_frame).as_millis() as u32;
        last_frame = now;

        // Handle mouse input -> touch events
        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Clamp) {
            let x = mx as i32;
            let y = my as i32;

            let is_down = window.get_mouse_down(MouseButton::Left);
            if is_down && !mouse_down {
                runtime.push_touch_event(TouchEvent::Down { x, y });
                mouse_down = true;
            } else if !is_down && mouse_down {
                runtime.push_touch_event(TouchEvent::Up { x, y });
                mouse_down = false;
            } else if is_down {
                runtime.push_touch_event(TouchEvent::Move { x, y });
            }
        }

        // Render frame
        if let Err(e) = runtime.render(delta_ms) {
            eprintln!("Render error: {e}");
        }

        // Composite overlay onto test pattern
        draw_background(&mut frame_buffer, width, height);
        composite_overlay(&mut frame_buffer, runtime.get_overlay(), width, height);

        // FPS counter (top-right corner)
        frame_times[frame_idx] = delta_ms.max(1) as f32;
        frame_idx = (frame_idx + 1) % frame_times.len();
        let avg_ms = frame_times.iter().sum::<f32>() / frame_times.len() as f32;
        let fps = (1000.0 / avg_ms) as u32;
        draw_fps(&mut frame_buffer, width, fps);

        window
            .update_with_buffer(&frame_buffer, width as usize, height as usize)
            .context("Failed to update window")?;

        // If widget doesn't want continuous frames, sleep a bit
        if !runtime.wants_next_frame() {
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    // Explicit cleanup to minimize Wayland warnings
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

/// Draw a smooth dark gradient background.
fn draw_background(buffer: &mut [u32], width: u32, height: u32) {
    for y in 0..height {
        for x in 0..width {
            // Diagonal gradient from dark gray to slightly lighter
            let t = (x + y) as f32 / (width + height) as f32;
            let gray = (0x16 as f32 + t * 0x10 as f32) as u8;
            buffer[(y * width + x) as usize] =
                0xFF_00_00_00 | (gray as u32) << 16 | (gray as u32) << 8 | gray as u32;
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

/// Draw FPS counter in top-right corner using 3x5 bitmap digits.
fn draw_fps(buffer: &mut [u32], width: u32, fps: u32) {
    // 3x5 bitmap font for digits 0-9 (each digit is 3 cols × 5 rows, packed as u16)
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

    let text: Vec<_> = fps.to_string().chars().collect();
    let scale = 2_u32;
    let char_w = 3 * scale + scale; // 3 pixels + 1 spacing
    let text_w = text.len() as u32 * char_w;
    let start_x = width.saturating_sub(text_w + 4);
    let start_y = 4_u32;

    for (i, ch) in text.iter().enumerate() {
        let digit = ch.to_digit(10).unwrap_or(0) as usize;
        let bits = DIGITS[digit];
        let ox = start_x + i as u32 * char_w;

        for row in 0..5_u32 {
            for col in 0..3_u32 {
                let bit_idx = (4 - row) * 3 + (2 - col);
                if (bits >> bit_idx) & 1 == 1 {
                    // Draw scaled pixel
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = ox + col * scale + sx;
                            let py = start_y + row * scale + sy;
                            if px < width {
                                let idx = (py * width + px) as usize;
                                if idx < buffer.len() {
                                    buffer[idx] = 0xFF_80_80_80; // gray
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
