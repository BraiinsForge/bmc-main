// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

#![allow(clippy::cast_precision_loss)]

//! Host limit stress test — toggles between normal, CPU-burn, draw-spam
//! and stack-bomb modes to exercise the fuel budget and the stack reservation.
//!
//! The two limits fail differently on purpose: a fuel overrun is recoverable,
//! so `render` retries and the instance lives on, while a stack overflow traps
//! and the host tears the slot down.

use bmc_wasm_sdk::{
    FontWeight, GRAY_10, GRAY_50, GRAY_70, Node, ORANGE_50, RED_50, WidgetSize, button, col, props,
    render_ui, request_frame, row, spacer, style, text, widget_size,
};
use std::cell::Cell;

thread_local! {
    /// 0 = normal, 1 = CPU burn, 2 = draw spam, 3 = stack bomb
    static MODE: Cell<u32> = const { Cell::new(0) };
}

const MODE_NAMES: [&str; 4] = ["Normal", "CPU Burn", "Draw Spam", "Stack Bomb"];

/// Re-render in response to touch — the host no longer renders on touch by
/// itself, so an interactive widget must ask for the frame here.
#[unsafe(no_mangle)]
pub extern "C" fn on_touch() {
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        ..
    } = widget_size();
    let mode = MODE.get();

    let mode_color = match mode {
        1 | 3 => RED_50,
        2 => ORANGE_50,
        _ => GRAY_50,
    };

    // Always build a lightweight tree so the UI always submits successfully.
    // Expensive work (cpu_burn / draw_spam) happens AFTER render_ui returns.
    let small = w < 640;
    let children: Vec<Node> = vec![
        text("Host Limit Stress Test", style!(size: 24, color: GRAY_10)),
        row(
            props!(gap: 12.0),
            [
                text("Mode:", style!(size: 18, color: GRAY_70)),
                text(
                    MODE_NAMES[mode as usize],
                    style!(size: 18, weight: FontWeight::BOLD, color: mode_color),
                ),
            ],
        ),
        text(mode_description(mode), style!(size: 14, color: GRAY_70)),
        spacer(1.0),
        row(
            props!(gap: 8.0),
            if small {
                vec![
                    button!("normal", "Normal", style: Primary, size: Small),
                    button!("cpu_burn", "CPU Burn", style: Danger, size: Small),
                    button!("draw_spam", "Draw Spam", style: Secondary, size: Small),
                    button!("stack_bomb", "Stack Bomb", style: Danger, size: Small),
                ]
            } else {
                vec![
                    button!("normal", "Normal", style: Primary),
                    button!("cpu_burn", "CPU Burn", style: Danger),
                    button!("draw_spam", "Draw Spam", style: Secondary),
                    button!("stack_bomb", "Stack Bomb", style: Danger),
                ]
            },
        ),
    ];

    let root = col(props!(padding: 24.0, gap: 16.0, flex: 1.0), children);

    // Submit tree and process clicks BEFORE the expensive work.
    let result = render_ui(w, h, root);
    if result.clicks.contains_key("normal") {
        MODE.set(0);
    } else if result.clicks.contains_key("cpu_burn") {
        MODE.set(1);
    } else if result.clicks.contains_key("draw_spam") {
        MODE.set(2);
    } else if result.clicks.contains_key("stack_bomb") {
        MODE.set(3);
        // The mode is read when a render starts, and a tap schedules no render.
        request_frame();
    }

    // Expensive work runs AFTER the tree is submitted.
    // Fuel dies here — the UI is already committed and clicks processed.
    match mode {
        1 => cpu_burn(),
        2 => draw_spam(),
        3 => stack_bomb(),
        _ => {}
    }
}

fn mode_description(mode: u32) -> &'static str {
    match mode {
        1 => "Tight math loop that exceeds fuel budget every frame.",
        2 => "Heavy memory allocation that simulates complex tree building.",
        3 => "Recursion that runs the stack off the bottom of linear memory.",
        _ => "Well-behaved render, comfortably within budget.",
    }
}

/// Tight computation loop — burns fuel on pure WASM instructions.
fn cpu_burn() {
    let mut x: u64 = 1;
    for i in 0..100_000_000u64 {
        x = x.wrapping_mul(i | 1).wrapping_add(7);
    }
    // Prevent the optimizer from removing the loop.
    core::hint::black_box(x);
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
    // Prevent the optimizer from removing the loop.
    core::hint::black_box(&buf);
}

/// Stack claimed per recursion step. Large enough that the linker's reservation
/// gives out well before the interpreter's own call-depth limit.
const STACK_BOMB_FRAME_BYTES: usize = 4096;

/// Four times the 64 KiB reservation, so the trap still lands if it grows.
const STACK_BOMB_FRAMES: u32 = 64;

/// Recursion deep enough to run the stack off the bottom of linear memory,
/// where it traps as an ordinary out-of-bounds access.
fn stack_bomb() {
    core::hint::black_box(claim_stack(STACK_BOMB_FRAMES));
}

/// `#[inline(never)]` and the `black_box` keep every step a real stack frame
/// rather than a value the optimizer folds away.
#[inline(never)]
fn claim_stack(frames_left: u32) -> u32 {
    if frames_left == 0 {
        return 0;
    }
    let mut frame = [0xA5_u8; STACK_BOMB_FRAME_BYTES];
    core::hint::black_box(&mut frame);
    claim_stack(frames_left - 1).wrapping_add(u32::from(frame[0]))
}
