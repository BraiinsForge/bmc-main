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

//! Host-side animation computation: easing functions, color interpolation, animation value logic.

#![expect(clippy::cast_precision_loss, clippy::integer_division)]

use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{ColorSpace, Easing, LoopMode};

use crate::AnimationState;
use crate::tree::HostAnimationDef;

/// Apply an easing function to a normalized time value `t` in 0.0..=1.0.
#[must_use]
pub fn apply_easing(easing: Easing, t: f32) -> f32 {
    match easing {
        Easing::Linear => t,
        Easing::EaseIn => t * t,
        Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        Easing::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            }
        }
        Easing::EaseInCubic => t * t * t,
        Easing::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
        Easing::EaseInOutCubic => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        // Overshoot: goes past 1.0 then settles back
        Easing::EaseOutBack => {
            let c1 = 1.701_58;
            let c3 = c1 + 1.0;
            let t1 = t - 1.0;
            1.0 + c3 * t1 * t1 * t1 + c1 * t1 * t1
        }
        Easing::EaseInOutBack => {
            let c1 = 1.701_58;
            let c2 = c1 * 1.525;
            if t < 0.5 {
                let t2 = 2.0 * t;
                (t2 * t2 * ((c2 + 1.0) * t2 - c2)) / 2.0
            } else {
                let t2 = 2.0 * t - 2.0;
                t2.mul_add(t2 * ((c2 + 1.0) * t2 + c2), 2.0) / 2.0
            }
        }
        // Bounce: multiple decreasing bounces (like a ball landing)
        Easing::EaseOutBounce => ease_out_bounce(t),
        // Elastic: damped spring oscillation
        Easing::EaseOutElastic => {
            if t <= 0.0 {
                0.0
            } else if t >= 1.0 {
                1.0
            } else {
                let c4 = core::f32::consts::TAU / 3.0;
                2.0_f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
            }
        }
    }
}

/// Standard bounce-out curve: ball drop with 4 decreasing bounces.
fn ease_out_bounce(t: f32) -> f32 {
    let n1 = 7.5625;
    let d1 = 2.75;
    if t < 1.0 / d1 {
        n1 * t * t
    } else if t < 2.0 / d1 {
        let t = t - 1.5 / d1;
        n1 * t * t + 0.75
    } else if t < 2.5 / d1 {
        let t = t - 2.25 / d1;
        n1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / d1;
        n1 * t * t + 0.984_375
    }
}

/// Advance animation state by `delta_ms` and compute the current value.
///
/// Returns the interpolated value and whether the animation is still active.
pub fn compute_animation_value(
    def: &HostAnimationDef,
    state: &mut AnimationState,
    delta_ms: u32,
) -> (f32, bool) {
    state.elapsed_ms = state.elapsed_ms.saturating_add(delta_ms);

    // Handle delay
    if state.elapsed_ms < u32::from(def.delay_ms) {
        return (def.from, true);
    }

    let active_elapsed = state.elapsed_ms - u32::from(def.delay_ms);

    if def.duration_ms == 0 {
        return (def.to, false);
    }

    match def.loop_mode {
        LoopMode::Once => {
            if active_elapsed >= def.duration_ms {
                (def.to, false)
            } else {
                let t = active_elapsed as f32 / def.duration_ms as f32;
                let eased = apply_easing(def.easing, t);
                (lerp(def.from, def.to, eased), true)
            }
        }
        LoopMode::Forever => {
            let t = (active_elapsed % def.duration_ms) as f32 / def.duration_ms as f32;
            let eased = apply_easing(def.easing, t);
            (lerp(def.from, def.to, eased), true)
        }
        LoopMode::PingPong => {
            let cycle = active_elapsed / def.duration_ms;
            let within = (active_elapsed % def.duration_ms) as f32 / def.duration_ms as f32;
            let forward = cycle.is_multiple_of(2);
            let t = if forward { within } else { 1.0 - within };
            let eased = apply_easing(def.easing, t);
            (lerp(def.from, def.to, eased), true)
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The range `apply_easing` can return for `easing`, rounded outwards.
///
/// Not every easing stays inside `0.0..=1.0`: the Back and Elastic families
/// overshoot on purpose, so the eased lerp passes the endpoint it heads for.
/// Damage tracking has to cover the reach, not the endpoints.
#[must_use]
pub fn easing_extremes(easing: Easing) -> (f32, f32) {
    // Measured off `apply_easing` over a dense sweep, rounded outwards so the
    // pair always contains the real extreme.
    const BACK_OVERSHOOT: f32 = 1.101;
    const BACK_UNDERSHOOT: f32 = -0.101;
    const ELASTIC_OVERSHOOT: f32 = 1.374;

    match easing {
        Easing::EaseOutBack => (0.0, BACK_OVERSHOOT),
        Easing::EaseInOutBack => (BACK_UNDERSHOOT, BACK_OVERSHOOT),
        Easing::EaseOutElastic => (0.0, ELASTIC_OVERSHOOT),
        Easing::Linear
        | Easing::EaseIn
        | Easing::EaseOut
        | Easing::EaseInOut
        | Easing::EaseInCubic
        | Easing::EaseOutCubic
        | Easing::EaseInOutCubic
        | Easing::EaseOutBounce => (0.0, 1.0),
    }
}

// ============================================================================
// Color interpolation
// ============================================================================

/// Interpolate between two colors in Oklab perceptual space.
///
/// Delegates to [`Color::mix`]. The `color_space` parameter is accepted
/// for wire-format compatibility but ignored — Oklab is always used.
#[must_use]
pub fn interpolate_color(from: Color, to: Color, t: f32, _color_space: ColorSpace) -> Color {
    from.mix(to, t)
}

#[cfg(test)]
mod tests {
    use super::{apply_easing, easing_extremes};
    use bmc_wasm_protocol::Easing;

    const EVERY_EASING: [Easing; 11] = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::EaseInCubic,
        Easing::EaseOutCubic,
        Easing::EaseInOutCubic,
        Easing::EaseOutBack,
        Easing::EaseInOutBack,
        Easing::EaseOutBounce,
        Easing::EaseOutElastic,
    ];

    /// Damage tracking sizes its repaint from these bounds, so an easing
    /// reaching past them paints outside the rectangle and stays on screen.
    #[test]
    fn easing_never_leaves_its_declared_extremes() {
        const STEPS: u32 = 20_000;
        for easing in EVERY_EASING {
            let (min, max) = easing_extremes(easing);
            for step in 0..=STEPS {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "step and STEPS are far below f32's exact range"
                )]
                let t = step as f32 / STEPS as f32;
                let eased = apply_easing(easing, t);
                assert!(
                    (min..=max).contains(&eased),
                    "{easing:?} at t={t} eased to {eased}, outside {min}..={max}"
                );
            }
        }
    }

    /// A bound nobody has measured drifts wide, and a wide bound costs the full
    /// repaint per frame that damage tracking exists to avoid.
    #[test]
    fn declared_extremes_stay_close_to_what_the_easings_reach() {
        const STEPS: u32 = 20_000;
        const SLACK: f32 = 0.01;
        for easing in EVERY_EASING {
            let (min, max) = easing_extremes(easing);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for step in 0..=STEPS {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "step and STEPS are far below f32's exact range"
                )]
                let t = step as f32 / STEPS as f32;
                let eased = apply_easing(easing, t);
                lo = lo.min(eased);
                hi = hi.max(eased);
            }
            assert!(
                lo - min < SLACK && max - hi < SLACK,
                "{easing:?} declares {min}..={max} but reaches {lo}..={hi}"
            );
        }
    }
}
