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

#[cfg(target_arch = "wasm32")]
mod analog;
#[cfg(target_arch = "wasm32")]
mod digital;
mod manifest_params;
#[cfg(target_arch = "wasm32")]
mod shared;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(not(target_arch = "wasm32"))]
use bmc_wasm_sdk::ViewportShape;

use manifest_params::ClockStyle;
#[cfg(target_arch = "wasm32")]
use manifest_params::Params;
#[cfg(target_arch = "wasm32")]
use shared::clock_palette;

/// Choose the effective render mode. A round viewport forces the round analog
/// dial — a rectangular dial on round hardware clips at the corners — while a
/// rectangular viewport honors the operator's configured style.
#[must_use]
fn effective_style(configured: ClockStyle, shape: ViewportShape) -> ClockStyle {
    match shape {
        ViewportShape::Round => ClockStyle::AnalogRound,
        ViewportShape::Rectangular => configured,
    }
}

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

    let viewport = widget_viewport();
    let style = effective_style(params.clock_style, viewport.shape);
    let root = match style {
        ClockStyle::AnalogRound => {
            analog::round::render(now, &params, variant, w, h, effective_tz.as_ref(), &palette)
        }
        ClockStyle::AnalogRect => {
            analog::rect::render(now, &params, variant, w, h, effective_tz.as_ref(), &palette)
        }
        ClockStyle::Digital => {
            digital::render(now, &params, variant, effective_tz.as_ref(), &palette)
        }
    };

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

#[cfg(test)]
mod tests {
    use super::effective_style;
    use crate::manifest_params::ClockStyle;
    use bmc_wasm_sdk::ViewportShape;

    #[test]
    fn round_viewport_forces_round_analog() {
        assert_eq!(
            effective_style(ClockStyle::Digital, ViewportShape::Round),
            ClockStyle::AnalogRound
        );
        assert_eq!(
            effective_style(ClockStyle::AnalogRect, ViewportShape::Round),
            ClockStyle::AnalogRound
        );
    }

    #[test]
    fn rectangular_viewport_honors_configured_style() {
        assert_eq!(
            effective_style(ClockStyle::Digital, ViewportShape::Rectangular),
            ClockStyle::Digital
        );
        assert_eq!(
            effective_style(ClockStyle::AnalogRect, ViewportShape::Rectangular),
            ClockStyle::AnalogRect
        );
    }
}
