// Copyright (C) 2026  Braiins Systems s.r.o.

//! Clock widget — three render modes (analog round / analog rectangular / digital)
//! and four sizes (Small / Medium / Large / Full).
//!
//! Module layout:
//! - `shared` — palette, tz helpers, alarm-row drawer, numeric utils
//! - `digital` — digital render mode (numerals + header + footer)
//! - `analog` — analog parent: hand assets, pivots, angle bookkeeping
//! - `analog::round` — round dial renderer
//! - `analog::rect` — rectangular dial renderer

mod analog;
mod digital;
mod manifest_params;
mod shared;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use manifest_params::{ClockStyle, Params};
use shared::{clock_palette, f32_from_u32};

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
    let viewport_w = f32_from_u32(w);
    let viewport_h = f32_from_u32(h);

    let root = match params.clock_style {
        ClockStyle::AnalogRound => analog::round::render(
            now,
            &params,
            variant,
            w,
            h,
            effective_tz.as_ref(),
            &palette,
            viewport_w,
            viewport_h,
        ),
        ClockStyle::AnalogRect => analog::rect::render(
            now,
            &params,
            variant,
            w,
            h,
            effective_tz.as_ref(),
            &palette,
            viewport_w,
            viewport_h,
        ),
        ClockStyle::Digital => {
            digital::render(now, &params, variant, w, h, effective_tz.as_ref(), &palette)
        }
    };

    let _ = render_ui(w, h, root);
    // Re-render once per second so the displayed time advances.
    request_frame_after(1000);
}

/// Fires after every per-widget params delivery (operator change).
/// Trigger an immediate re-render so operator changes don't wait for
/// the next 1s tick.
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
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}
