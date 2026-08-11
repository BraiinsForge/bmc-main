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

//! LED peripheral control.
//!
//! Widgets drive the device's LED strip via a single `set_effect` call
//! that takes an optional duration (endless when `None`, temporary
//! when `Some`). `stop()` cancels every outstanding LED request from
//! this widget — it is also the only way to turn the strip off.

pub use bmc_led::data::{LedEffectKind as LedEffect, LedScope};
pub use bmc_wasm_protocol::Color;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_led_set_endless(effect: u8, r: u8, g: u8, b: u8, period_ms: u32, scope: u32);
    fn host_led_set_temporary(
        effect: u8,
        r: u8,
        g: u8,
        b: u8,
        period_ms: u32,
        duration_ms: u32,
        scope: u32,
    );
    fn host_led_stop();
}

/// Set an LED effect.
///
/// `duration_ms = None` runs the effect until superseded or stopped;
/// `Some(n)` runs for `n` ms (including `Some(0)` — a zero-duration
/// temporary that the host fires and immediately expires).
pub fn set_effect(effect: LedEffect, color: Color, period_ms: u32, duration_ms: Option<u32>) {
    set_effect_scoped(effect, color, period_ms, duration_ms, LedScope::Local);
}

/// Set an LED effect on the global tier.
///
/// The global tier runs when no scene-local effect is active on the strip.
/// See `set_effect` for parameter semantics.
pub fn set_effect_global(
    effect: LedEffect,
    color: Color,
    period_ms: u32,
    duration_ms: Option<u32>,
) {
    set_effect_scoped(effect, color, period_ms, duration_ms, LedScope::Global);
}

fn set_effect_scoped(
    effect: LedEffect,
    color: Color,
    period_ms: u32,
    duration_ms: Option<u32>,
    scope: LedScope,
) {
    let (r, g, b) = (color.red(), color.green(), color.blue());
    let scope = scope as u32;
    match duration_ms {
        None => unsafe { host_led_set_endless(effect as u8, r, g, b, period_ms, scope) },
        Some(d) => unsafe { host_led_set_temporary(effect as u8, r, g, b, period_ms, d, scope) },
    }
}

/// Cancel every LED request this widget has outstanding.
pub fn stop() {
    unsafe { host_led_stop() }
}
