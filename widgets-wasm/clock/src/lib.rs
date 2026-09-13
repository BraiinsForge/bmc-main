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
use bmc_wasm_sdk::{Draw, Easing, ViewportShape};

use manifest_params::ClockStyle;
#[cfg(target_arch = "wasm32")]
use manifest_params::Params;
#[cfg(target_arch = "wasm32")]
use shared::clock_palette;

/// Widgets get no entering/visible lifecycle hook, so a render gap this long
/// stands in for one: the hands snap to the current time
/// instead of sweeping from where the page left them.
/// Keyed on the render delta rather than wall-clock time,
/// so a DST or NTP step still sweeps the hands.
const MAX_ANIMATED_RENDER_GAP_MS: u32 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClockHandTransition {
    Animate,
    Snap,
}

impl ClockHandTransition {
    fn for_render_gap(delta_ms: u32) -> Self {
        if delta_ms > MAX_ANIMATED_RENDER_GAP_MS {
            Self::Snap
        } else {
            Self::Animate
        }
    }

    fn apply(self, draw: Draw, id: &str, duration_ms: u32) -> Draw {
        let duration_ms = match self {
            Self::Animate => duration_ms,
            Self::Snap => 0,
        };
        draw.transition(id, duration_ms, Easing::EaseOut)
    }
}

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
pub extern "C" fn render(delta_ms: u32) {
    let ws = widget_size();
    let now = SystemTime::now();
    let params = Params::current();
    let effective_tz = params.timezone_override.as_deref().map(Tz::from_runtime);
    let palette = clock_palette(system::current().night_mode().unwrap_or(false));
    let hand_transition = ClockHandTransition::for_render_gap(delta_ms);

    let viewport = widget_viewport();
    let style = effective_style(params.clock_style, viewport.shape);
    let root = match style {
        ClockStyle::AnalogRound => analog::round::render(
            now,
            &params,
            ws,
            effective_tz.as_ref(),
            &palette,
            hand_transition,
        ),
        ClockStyle::AnalogRect => analog::rect::render(
            now,
            &params,
            ws,
            effective_tz.as_ref(),
            &palette,
            hand_transition,
        ),
        ClockStyle::Digital => digital::render(now, &params, ws, effective_tz.as_ref(), &palette),
    };

    let _ = render_ui(ws.width, ws.height, root);
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
    use super::{ClockHandTransition, effective_style};
    use crate::manifest_params::ClockStyle;
    use bmc_wasm_sdk::{Draw, Easing, ViewportShape, WHITE};

    #[test]
    fn hand_transitions_animate_through_five_second_render_gaps() {
        for delta_ms in [0, 1_000, 4_999, 5_000] {
            assert_eq!(
                ClockHandTransition::for_render_gap(delta_ms),
                ClockHandTransition::Animate
            );
        }
    }

    #[test]
    fn hand_transitions_snap_after_longer_render_gaps() {
        for delta_ms in [5_001, 30_000, u32::MAX] {
            assert_eq!(
                ClockHandTransition::for_render_gap(delta_ms),
                ClockHandTransition::Snap
            );
        }
    }

    #[test]
    fn snapping_preserves_hand_identity_and_resumes_normal_duration() {
        for (id, duration_ms) in [
            ("hour-hand", 500),
            ("minute-hand", 500),
            ("second-hand", 200),
        ] {
            let mut identity = None;
            for (delta_ms, expected_duration) in
                [(1_000, duration_ms), (5_001, 0), (1_000, duration_ms)]
            {
                let draw = ClockHandTransition::for_render_gap(delta_ms).apply(
                    Draw::rect(0.0, 0.0, 1.0, 1.0, WHITE),
                    id,
                    duration_ms,
                );
                let Draw::Modified {
                    transition: Some(transition),
                    ..
                } = draw
                else {
                    panic!("each hand must retain its transition across a render gap");
                };
                assert_eq!(transition.duration_ms, expected_duration);
                assert_eq!(transition.easing, Easing::EaseOut);
                assert_eq!(
                    *identity.get_or_insert(transition.id_hash),
                    transition.id_hash
                );
            }
        }
    }

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
