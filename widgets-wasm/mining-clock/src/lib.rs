// Copyright (C) 2026  Braiins Systems s.r.o.

//! Mining clock widget — round analog dial with live miner gauge rings.
//!
//! Module layout:
//! - `shared` — palette, tz helpers, alarm-row drawer, numeric utils
//! - `analog` — analog parent: hand assets, pivots, angle bookkeeping
//! - `analog::round` — round dial renderer

#[cfg(target_arch = "wasm32")]
mod analog;
mod manifest_params;
mod miner;
#[cfg(target_arch = "wasm32")]
mod shared;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
use manifest_params::Params;
#[cfg(target_arch = "wasm32")]
use shared::clock_palette;

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        variant,
    } = widget_size();
    let now = SystemTime::now();
    let params = Params::current();
    let effective_tz = params.timezone_override.as_deref().map(Tz::from_runtime);
    let palette = clock_palette(system::current().night_mode().unwrap_or(false));

    let root = analog::round::render(now, &params, variant, w, h, effective_tz.as_ref(), &palette);

    let _ = render_ui(w, h, root);
    // Re-render once per second so the displayed time advances.
    request_frame_after(1000);
}

/// Fires after every per-widget params delivery (operator change).
/// Trigger an immediate re-render so operator changes don't wait for
/// the next 1s tick.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    request_frame();
}

/// Fires after every deck-wide system snapshot delivery
/// (timezone, formats, next-alarm, night-mode, …).
///
/// Same reason for immediate re-render — night-mode flips
/// shouldn't sit on screen for up to a second before
/// the palette swap takes effect.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}
