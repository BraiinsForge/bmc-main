// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED peripheral control.
//!
//! Lets widgets drive the device's LED strip — set effects, brightness,
//! enable/disable.  The host maps these calls to real hardware (or testbed
//! visualization).

use bmc_wasm_protocol::Color;

pub use bmc_led::data::LedEffectKind as LedEffect;

unsafe extern "C" {
    fn host_led_set_effect(effect: u8, r: u8, g: u8, b: u8, period_ms: u32, duration_ms: u32);
    fn host_led_set_brightness(brightness_bits: u32);
    fn host_led_enable();
    fn host_led_disable();
}

/// Set an LED effect with color and timing.
///
/// `period_ms` controls animation speed (ignored for `Solid`/`None`).
/// `duration_ms` = 0 means persistent (stays until replaced).
pub fn set_effect(effect: LedEffect, color: Color, period_ms: u32, duration_ms: u32) {
    unsafe {
        host_led_set_effect(
            effect as u8,
            color.red(),
            color.green(),
            color.blue(),
            period_ms,
            duration_ms,
        );
    }
}

/// Set LED brightness (0.0–1.0).
///
/// Transmitted as `f32` bits to avoid float ABI issues in WASM FFI.
pub fn set_brightness(brightness: f32) {
    unsafe { host_led_set_brightness(brightness.to_bits()) }
}

/// Enable the LED strip.
pub fn enable() {
    unsafe { host_led_enable() }
}

/// Disable the LED strip.
pub fn disable() {
    unsafe { host_led_disable() }
}
