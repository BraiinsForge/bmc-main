// Copyright (C) 2026  Braiins Systems s.r.o.

//! Analog clock render modes — shared infrastructure (hand assets, hand
//! placement / angle bookkeeping, per-size parameter tables, centre-circle
//! stack) plus the two variant modules.

pub(crate) mod rect;
pub(crate) mod round;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::shared::local_or_system;

// ── Hand assets ────────────────────────────────────────────────────────

pub(crate) const HAND_HOUR: Svg = include_svg!("assets/analog/hand-hour.svg");
pub(crate) const HAND_MINUTE: Svg = include_svg!("assets/analog/hand-minute.svg");
pub(crate) const HAND_SECOND: Svg = include_svg!("assets/analog/hand-second.svg");
pub(crate) const CENTER_WHITE: Svg = include_svg!("assets/analog/center-circle-white.svg");
pub(crate) const CENTER_ORANGE: Svg = include_svg!("assets/analog/center-circle-orange.svg");
pub(crate) const CENTER_BLACK: Svg = include_svg!("assets/analog/center-circle-black.svg");
pub(crate) const CENTER_STROKE: Svg = include_svg!("assets/analog/center-circle-stroke.svg");

// ── Hand viewport / pivot metrics ──────────────────────────────────────

/// Uniform shrink applied to every hand viewport + its pivot so the hand
/// tips land inside the dial rim per the Figma reference.
/// The SVG assets bake in slint's drop-shadow padding which makes
/// the hands oversized relative to the visible dial; tune by eye.
const HAND_SHRINK: f32 = 0.85;

/// Hour-hand SVG dims: viewBox 0 0 63 290 rendered into
/// a 290×290 square via `xMidYMid meet` → content centered
/// horizontally with (290 − 63) / 2 = 113.5 px letterboxing
/// on each side.
///
/// Pivot is (31, 121) in viewBox coords; in render coords
/// that becomes (113.5 + 31, 121) = (144.5, 121).
pub(crate) const HAND_HOUR_VIEWPORT: f32 = 290.0 * HAND_SHRINK;
pub(crate) const HAND_HOUR_PIVOT_X: f32 = 144.5 * HAND_SHRINK;
pub(crate) const HAND_HOUR_PIVOT_Y: f32 = 121.0 * HAND_SHRINK;

/// Minute-hand: viewBox 0 0 50 400 → 400×400 viewport, letterbox 175
/// → pivot at (175 + 25, 200) = (200, 200) = viewport centre.
pub(crate) const HAND_MINUTE_VIEWPORT: f32 = 400.0 * HAND_SHRINK;
pub(crate) const HAND_MINUTE_PIVOT_X: f32 = 200.0 * HAND_SHRINK;
pub(crate) const HAND_MINUTE_PIVOT_Y: f32 = 200.0 * HAND_SHRINK;

/// Second-hand: viewBox 0 0 4 398 → 398×398 viewport, letterbox 197
/// → pivot at (197 + 2, 198) = (199, 198), basically viewport centre.
pub(crate) const HAND_SECOND_VIEWPORT: f32 = 398.0 * HAND_SHRINK;
pub(crate) const HAND_SECOND_PIVOT_X: f32 = 199.0 * HAND_SHRINK;
pub(crate) const HAND_SECOND_PIVOT_Y: f32 = 198.0 * HAND_SHRINK;

// ── Angle bookkeeping ──────────────────────────────────────────────────

// Per-hand unwrapped-angle state.
// The SDK's transition lerps linearly between consecutive submitted angles,
// so a wrap from ~2π back to 0 makes the host pick the long arc.
// We accumulate signed short-arc deltas into these cells so the submitted angle
// is monotonic and the lerp always travels the short way around.
thread_local! {
    static HOUR_ANGLE_STATE: core::cell::Cell<f32> = const { core::cell::Cell::new(0.0) };
    static MINUTE_ANGLE_STATE: core::cell::Cell<f32> = const { core::cell::Cell::new(0.0) };
    static SECOND_ANGLE_STATE: core::cell::Cell<f32> = const { core::cell::Cell::new(0.0) };
}

fn unwrap_angle(state: &core::cell::Cell<f32>, target_mod_tau: f32) -> f32 {
    let last = state.get();
    let last_mod = last.rem_euclid(std::f32::consts::TAU);
    let mut delta = target_mod_tau - last_mod;
    if delta > std::f32::consts::PI {
        delta -= std::f32::consts::TAU;
    } else if delta < -std::f32::consts::PI {
        delta += std::f32::consts::TAU;
    }
    let next = last + delta;
    state.set(next);
    next
}

pub(crate) fn hour_angle(hour: u8, minute: u8) -> f32 {
    let hour12 = f32::from(hour % 12);
    let m = f32::from(minute);
    let target = (hour12 * 30.0 + m * 0.5).to_radians();
    HOUR_ANGLE_STATE.with(|s| unwrap_angle(s, target))
}

pub(crate) fn minute_angle(minute: u8) -> f32 {
    let target = (f32::from(minute) * 6.0).to_radians();
    MINUTE_ANGLE_STATE.with(|s| unwrap_angle(s, target))
}

pub(crate) fn second_angle(second: u8) -> f32 {
    let target = (f32::from(second) * 6.0).to_radians();
    SECOND_ANGLE_STATE.with(|s| unwrap_angle(s, target))
}

pub(crate) fn local_clock_components(now: &SystemTime, tz: &Tz) -> (u8, u8, u8) {
    let local = local_or_system(now, tz);
    (local.hour, local.minute, local.second)
}

// ── Hand & centre placement ────────────────────────────────────────────

/// Position a hand icon at the canvas so its SVG-coordinate
/// pivot lands exactly on the canvas centre.
///
/// The SDK's rotation primitive then rotates the hand
/// around its own pivot (since the rotation axis is canvas-centre).
#[expect(
    clippy::too_many_arguments,
    reason = "hand placement is a flat geometry helper over explicit SVG metrics"
)]
pub(crate) fn place_hand_at_pivot(
    centre_x: f32,
    centre_y: f32,
    scale: f32,
    viewport: f32,
    pivot_x: f32,
    pivot_y: f32,
    icon: &'static Svg,
    tint: Color,
) -> Draw {
    let w = viewport * scale;
    let h = viewport * scale;
    let top_left_x = centre_x - pivot_x * scale;
    let top_left_y = centre_y - pivot_y * scale;
    Draw::svg(top_left_x, top_left_y, w, h, icon, tint).with_anti_alias()
}

/// Hand drop shadow — ~6 px Gaussian, 50 % black, scaled with the dial.
pub(crate) fn hand_shadow(scale: f32) -> DropShadow {
    DropShadow {
        dx: 0.0,
        dy: 0.0,
        blur: 6.0 * scale,
        color: Color::from_rgba(0, 0, 0, 0x80),
    }
}

/// Shadow the centre disc casts onto the hands beneath it — dark and slightly
/// offset, reading cleanly because it lands on the white hand surfaces.
pub(crate) fn centre_shadow(scale: f32) -> DropShadow {
    DropShadow {
        dx: 0.0,
        dy: 3.0 * scale,
        blur: 5.0 * scale,
        color: Color::from_rgba(0, 0, 0, 0xA0),
    }
}

pub(crate) fn centre_icon(
    centre_x: f32,
    centre_y: f32,
    scale: f32,
    native_side: f32,
    icon: &'static Svg,
    tint: Color,
) -> Draw {
    let side = native_side * scale;
    let top_left_x = centre_x - side / 2.0;
    let top_left_y = centre_y - side / 2.0;
    Draw::svg(top_left_x, top_left_y, side, side, icon, tint).with_anti_alias()
}
