// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host-side animation computation: easing functions, color interpolation, animation value logic.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::integer_division,
    clippy::many_single_char_names
)]

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

/// Interpolate between two RGBA colors using the specified color space.
pub fn interpolate_color(from: u32, to: u32, t: f32, color_space: ColorSpace) -> u32 {
    match color_space {
        ColorSpace::Oklab => lerp_color_oklab(from, to, t),
        ColorSpace::Oklch => lerp_color_oklch(from, to, t),
        ColorSpace::LinearRgb => lerp_color_linear_rgb(from, to, t),
        ColorSpace::Srgb => lerp_color_srgb(from, to, t),
    }
}

/// Unpack RGBA u32 (big-endian: 0xRRGGBBAA) into (r, g, b, a) as 0.0..1.0.
fn unpack_rgba(c: u32) -> (f32, f32, f32, f32) {
    let bytes = c.to_be_bytes();
    (
        f32::from(bytes[0]) / 255.0,
        f32::from(bytes[1]) / 255.0,
        f32::from(bytes[2]) / 255.0,
        f32::from(bytes[3]) / 255.0,
    )
}

/// Pack (r, g, b, a) as 0.0..1.0 into RGBA u32.
fn pack_rgba(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let r = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let g = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let b = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let a = (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    u32::from_be_bytes([r, g, b, a])
}

// sRGB → linear
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

// linear → sRGB
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB → Oklab (L, a, b)
fn srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let lr = srgb_to_linear(r);
    let lg = srgb_to_linear(g);
    let lb = srgb_to_linear(b);

    let l = 0.412_221_5 * lr + 0.536_332_5 * lg + 0.051_445_8 * lb;
    let m = 0.211_903_5 * lr + 0.680_699_5 * lg + 0.107_396_8 * lb;
    let s = 0.088_302_46 * lr + 0.281_718_84 * lg + 0.629_978_7 * lb;

    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();

    (
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_8 * m - 0.808_675_8 * s,
    )
}

/// Oklab → sRGB
fn oklab_to_srgb(ol: f32, oa: f32, ob: f32) -> (f32, f32, f32) {
    let l = ol + 0.396_337_78 * oa + 0.215_803_76 * ob;
    let m = ol - 0.105_561_346 * oa - 0.063_854_17 * ob;
    let s = ol - 0.089_484_18 * oa - 1.291_485_5 * ob;

    let l = l * l * l;
    let m = m * m * m;
    let s = s * s * s;

    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;

    (linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
}

fn lerp_color_oklab(from: u32, to: u32, t: f32) -> u32 {
    let (r1, g1, b1, a1) = unpack_rgba(from);
    let (r2, g2, b2, a2) = unpack_rgba(to);

    let (l1, a1_ok, b1_ok) = srgb_to_oklab(r1, g1, b1);
    let (l2, a2_ok, b2_ok) = srgb_to_oklab(r2, g2, b2);

    let l = lerp(l1, l2, t);
    let a_ok = lerp(a1_ok, a2_ok, t);
    let b_ok = lerp(b1_ok, b2_ok, t);
    let alpha = lerp(a1, a2, t);

    let (r, g, b) = oklab_to_srgb(l, a_ok, b_ok);
    pack_rgba(r, g, b, alpha)
}

fn lerp_color_oklch(from: u32, to: u32, t: f32) -> u32 {
    let (r1, g1, b1, a1) = unpack_rgba(from);
    let (r2, g2, b2, a2) = unpack_rgba(to);

    let (l1, a1_ok, b1_ok) = srgb_to_oklab(r1, g1, b1);
    let (l2, a2_ok, b2_ok) = srgb_to_oklab(r2, g2, b2);

    // Convert to LCH
    let c1 = (a1_ok * a1_ok + b1_ok * b1_ok).sqrt();
    let h1 = b1_ok.atan2(a1_ok);
    let c2 = (a2_ok * a2_ok + b2_ok * b2_ok).sqrt();
    let h2 = b2_ok.atan2(a2_ok);

    // Interpolate with shortest-path hue
    let l = lerp(l1, l2, t);
    let c = lerp(c1, c2, t);

    let mut dh = h2 - h1;
    if dh > std::f32::consts::PI {
        dh -= 2.0 * std::f32::consts::PI;
    } else if dh < -std::f32::consts::PI {
        dh += 2.0 * std::f32::consts::PI;
    }
    let h = h1 + dh * t;

    let a_ok = c * h.cos();
    let b_ok = c * h.sin();
    let alpha = lerp(a1, a2, t);

    let (r, g, b) = oklab_to_srgb(l, a_ok, b_ok);
    pack_rgba(r, g, b, alpha)
}

fn lerp_color_linear_rgb(from: u32, to: u32, t: f32) -> u32 {
    let (r1, g1, b1, a1) = unpack_rgba(from);
    let (r2, g2, b2, a2) = unpack_rgba(to);

    let r = linear_to_srgb(lerp(srgb_to_linear(r1), srgb_to_linear(r2), t));
    let g = linear_to_srgb(lerp(srgb_to_linear(g1), srgb_to_linear(g2), t));
    let b = linear_to_srgb(lerp(srgb_to_linear(b1), srgb_to_linear(b2), t));
    let a = lerp(a1, a2, t);

    pack_rgba(r, g, b, a)
}

fn lerp_color_srgb(from: u32, to: u32, t: f32) -> u32 {
    let (r1, g1, b1, a1) = unpack_rgba(from);
    let (r2, g2, b2, a2) = unpack_rgba(to);

    pack_rgba(
        lerp(r1, r2, t),
        lerp(g1, g2, t),
        lerp(b1, b2, t),
        lerp(a1, a2, t),
    )
}

/// Multiply the alpha channel of an RGBA color.
pub fn multiply_alpha(color: u32, alpha: f32) -> u32 {
    let bytes = color.to_be_bytes();
    let new_alpha = (f32::from(bytes[3]) * alpha).clamp(0.0, 255.0) as u8;
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], new_alpha])
}
