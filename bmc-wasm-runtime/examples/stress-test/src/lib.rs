// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(clippy::cast_precision_loss)]

//! Fuel limiter stress test — toggles between normal, CPU-burn, and draw-spam
//! modes to exercise the host fuel budget enforcement.

use bmc_wasm_sdk::{
    GRAY_10, GRAY_50, GRAY_70, Node, ORANGE_50, RED_50, button, col, props, render_ui, row, spacer,
    style, text,
};
use std::cell::Cell;

thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(1_280) };
    static HEIGHT: Cell<u32> = const { Cell::new(480) };
    /// 0 = normal, 1 = CPU burn, 2 = draw spam
    static MODE: Cell<u32> = const { Cell::new(0) };
}

const MODE_NAMES: [&str; 3] = ["Normal", "CPU Burn", "Draw Spam"];

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    WIDTH.set(width);
    HEIGHT.set(height);
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let w = WIDTH.get();
    let h = HEIGHT.get();
    let mode = MODE.get();

    let mode_color = match mode {
        1 => RED_50,
        2 => ORANGE_50,
        _ => GRAY_50,
    };

    // Always build a lightweight tree so the UI always submits successfully.
    // Expensive work (cpu_burn / draw_spam) happens AFTER render_ui returns.
    let small = w < 640;
    let children: Vec<Node> = vec![
        text("Fuel Limiter Stress Test", style!(size: 24, color: GRAY_10)),
        row(
            props!(gap: 12.0),
            [
                text("Mode:", style!(size: 18, color: GRAY_70)),
                text(
                    MODE_NAMES[mode as usize],
                    style!(size: 18, weight: 700, color: mode_color),
                ),
            ],
        ),
        text(mode_description(mode), style!(size: 14, color: GRAY_70)),
        spacer(1.0),
        row(
            props!(gap: 8.0),
            if small {
                vec![
                    button!("Normal", style: Primary, size: Small),
                    button!("CPU Burn", style: Danger, size: Small),
                    button!("Draw Spam", style: Secondary, size: Small),
                ]
            } else {
                vec![
                    button!("Normal", style: Primary),
                    button!("CPU Burn", style: Danger),
                    button!("Draw Spam", style: Secondary),
                ]
            },
        ),
    ];

    let root = col(props!(padding: 24.0, gap: 16.0, flex: 1.0), children);

    // Submit tree and process clicks BEFORE the expensive work.
    let result = render_ui(w, h, root);
    for (i, &clicked) in result.clicks.iter().enumerate() {
        if clicked {
            MODE.set(i as u32);
        }
    }

    // Expensive work runs AFTER the tree is submitted.
    // Fuel dies here — the UI is already committed and clicks processed.
    match mode {
        1 => cpu_burn(),
        2 => draw_spam(),
        _ => {}
    }
}

fn mode_description(mode: u32) -> &'static str {
    match mode {
        1 => "Tight math loop that exceeds fuel budget every frame.",
        2 => "Heavy memory allocation that simulates complex tree building.",
        _ => "Well-behaved render, comfortably within budget.",
    }
}

/// Tight computation loop — burns fuel on pure WASM instructions.
fn cpu_burn() {
    let mut x: u64 = 1;
    for i in 0..100_000_000u64 {
        x = x.wrapping_mul(i | 1).wrapping_add(7);
    }
    // Prevent the optimizer from removing the loop
    if x == 0 {
        WIDTH.set(0);
    }
}

/// Heavy WASM-side memory work — simulates what happens when a widget
/// builds a complex tree with thousands of draw commands.  Burns fuel
/// on allocation + byte writes, same as real tree serialization.
fn draw_spam() {
    let count = 200_000u32;
    let mut buf: Vec<u8> = Vec::with_capacity(count as usize * 20);
    for i in 0..count {
        buf.extend_from_slice(&(i as f32).to_le_bytes());
        buf.extend_from_slice(&(i as f32).to_le_bytes());
        buf.extend_from_slice(&1.0_f32.to_le_bytes());
        buf.extend_from_slice(&1.0_f32.to_le_bytes());
        buf.extend_from_slice(&0xFF_00_00_FFu32.to_le_bytes());
    }
    // Prevent the optimizer from removing the loop
    if buf.is_empty() {
        WIDTH.set(0);
    }
}
