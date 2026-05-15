// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED peripheral control.
//!
//! Widgets drive the device's LED strip via a single `set_effect` call
//! that takes an optional duration (endless when `None`, temporary
//! when `Some`). `stop()` cancels every outstanding LED request from
//! this widget — it is also the only way to turn the strip off.

pub use bmc_wasm_protocol::{Color, LedEffect};

unsafe extern "C" {
    fn host_led_set_endless(effect: u8, r: u8, g: u8, b: u8, period_ms: u32);
    fn host_led_set_temporary(effect: u8, r: u8, g: u8, b: u8, period_ms: u32, duration_ms: u32);
    fn host_led_stop();
}

/// Set an LED effect.
///
/// `duration_ms = None` runs the effect until superseded or stopped;
/// `Some(n)` runs for `n` ms (including `Some(0)` — a zero-duration
/// temporary that the host fires and immediately expires).
pub fn set_effect(effect: LedEffect, color: Color, period_ms: u32, duration_ms: Option<u32>) {
    let (r, g, b) = (color.red(), color.green(), color.blue());
    match duration_ms {
        None => unsafe { host_led_set_endless(effect as u8, r, g, b, period_ms) },
        Some(d) => unsafe { host_led_set_temporary(effect as u8, r, g, b, period_ms, d) },
    }
}

/// Cancel every LED request this widget has outstanding.
pub fn stop() {
    unsafe { host_led_stop() }
}
