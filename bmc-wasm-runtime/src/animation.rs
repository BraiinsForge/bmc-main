// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host-side animation computation: easing functions, color interpolation, animation value logic.

#![expect(clippy::cast_precision_loss, clippy::integer_division)]

use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{ColorSpace, Easing, LoopMode};

use crate::host_api::AnimationState;
use crate::tree::HostAnimationDef;

/// Apply an easing function to a normalized time value `t` in 0.0..=1.0.
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

// ============================================================================
// Color interpolation
// ============================================================================

/// Interpolate between two colors in Oklab perceptual space.
///
/// Delegates to [`Color::mix`]. The `color_space` parameter is accepted
/// for wire-format compatibility but ignored — Oklab is always used.
pub fn interpolate_color(from: Color, to: Color, t: f32, _color_space: ColorSpace) -> Color {
    from.mix(to, t)
}
