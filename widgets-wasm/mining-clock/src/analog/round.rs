// Copyright (C) 2026  Braiins Systems s.r.o.

//! Round analog dial — single canvas at the widget viewport size, with
//! all hands rotating around the canvas centre via `Draw::rotated`.
//!
//! The SDK rotation primitive pivots on canvas-centre only (no per-image
//! pivot), so each hand image is positioned such that its SVG-coordinate
//! pivot lands at the canvas centre — see `place_hand_at_pivot`.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::Params;
use crate::miner::{self, MinerData};
use crate::shared::{
    AlarmAnchor, ClockPalette, TzLabel, alarm_row_draws, f32_from_u32, font_weight,
    local_or_system, push_utc_offset, resolve_tz_for_label,
};
use mining::gauge;
use mining::style::{INACTIVE_TICK, ring_fill};

use super::{hour_angle, local_clock_components, minute_angle, second_angle};

// ── Asset ──────────────────────────────────────────────────────────────

const DIAL_ROUND: Svg = include_svg!("assets/analog/dial-round.svg");

// ── Dial sizing ─────────────────────────────────────────────────────────
//
// The dial fills a fraction of the viewport's shorter side, so it always
// fits — on a square round display as well as a landscape rectangular one —
// and downscales instead of overflowing. The single `DIAL_ROUND` asset has a
// 390-native viewBox; everything authored inside the dial (timezone label,
// date window, alarm row) is in those native coordinates and multiplied by
// `scale = dial / NATIVE_DIAL` to track the rendered dial.

const NATIVE_DIAL: f32 = 390.0;

// 364 / 480 — keeps about 10 px between the dial rim and the inner mining ring.
const DIAL_FRACTION: f32 = 0.758_333_3;

const HASHRATE_RADIUS: f32 = 216.0;
const POWER_RADIUS: f32 = 196.0;
const RING_WIDTH: f32 = 8.0;
const HASHRATE_SWEEP_END: f32 = 330.0_f32.to_radians();
// Gap between a gauge's leading edge and its curved-text label, so each label
// reads as attached to the segment or arc it follows. The flat inner segments
// sit tighter to their label than the continuous outer ring.
const HASHRATE_LABEL_OFFSET: f32 = 4.0_f32.to_radians();
const POWER_LABEL_OFFSET: f32 = 1.0_f32.to_radians();
// Duration of the gauge sweep transition; the host animates each ring's
// end angle toward its target whenever the lit fraction changes.
const GAUGE_TRANSITION_MS: u32 = 500;
const LABEL_GRAY: Color = Color::from_rgb(0xA7, 0xA7, 0xA7);

// ── Per-size template parameters ───────────────────────────────────────

#[derive(Clone, Copy)]
pub(crate) struct AnalogRoundSizeParams {
    /// Show the date window (lower-half of the dial). Full / Large only.
    show_date_window: bool,
    /// Show the alarm row (left of dial centre).
    /// Full only when `next_alarm = Some(_)`.
    show_alarm: bool,
    /// Timezone label baseline as a fraction of the dial side, from its top.
    timezone_y_frac: f32,
    /// Timezone label font size.
    timezone_font_size: u32,
}

const ANALOG_ROUND_FULL: AnalogRoundSizeParams = AnalogRoundSizeParams {
    show_date_window: true,
    show_alarm: true,
    timezone_y_frac: 0.2667,
    timezone_font_size: 24,
};

const ANALOG_ROUND_LARGE: AnalogRoundSizeParams = AnalogRoundSizeParams {
    show_date_window: true,
    show_alarm: false,
    timezone_y_frac: 0.2872,
    timezone_font_size: 24,
};

const ANALOG_ROUND_MEDIUM: AnalogRoundSizeParams = AnalogRoundSizeParams {
    show_date_window: false,
    show_alarm: false,
    timezone_y_frac: 0.2718,
    timezone_font_size: 16,
};

const ANALOG_ROUND_SMALL: AnalogRoundSizeParams = AnalogRoundSizeParams {
    show_date_window: false,
    show_alarm: false,
    timezone_y_frac: 0.2718,
    timezone_font_size: 16,
};

fn pick_size(variant: SizeVariant) -> &'static AnalogRoundSizeParams {
    match variant {
        SizeVariant::Full => &ANALOG_ROUND_FULL,
        SizeVariant::Large => &ANALOG_ROUND_LARGE,
        SizeVariant::Medium => &ANALOG_ROUND_MEDIUM,
        SizeVariant::Small => &ANALOG_ROUND_SMALL,
    }
}

// ── Render ─────────────────────────────────────────────────────────────

#[expect(
    clippy::too_many_arguments,
    reason = "render entry threads the existing clock context plus the miner snapshot"
)]
pub(crate) fn render(
    now: SystemTime,
    params: &Params,
    variant: SizeVariant,
    w: u32,
    h: u32,
    tz: Option<&Tz>,
    palette: &ClockPalette,
    miner: &MinerData,
    seed_gauges: bool,
) -> Node {
    let size = pick_size(variant);
    let viewport_w = f32_from_u32(w);
    let viewport_h = f32_from_u32(h);
    // Dial side as a fraction of the shorter viewport axis, so it always fits
    // and downscales rather than overflowing. `scale` maps the dial's native
    // 390 coordinates (hands, internal offsets) into rendered pixels.
    let dial = viewport_w.min(viewport_h) * DIAL_FRACTION;
    let scale = dial / NATIVE_DIAL;
    // Single canvas at the widget viewport size — the SDK's `Draw::rotated`
    // pivots around canvas centre, so making the canvas match the widget
    // and centering the dial at the canvas centre lets every hand rotate
    // around the dial midpoint without nested layout.
    let centre_x = viewport_w / 2.0;
    let centre_y = viewport_h / 2.0;
    let dial_top_y = centre_y - dial / 2.0;
    let label = resolve_tz_for_label(tz, now.unix_secs);
    let offset_secs = match &label {
        TzLabel::Resolved { offset_secs, .. } => *offset_secs,
        TzLabel::Unknown {
            system_offset_secs, ..
        } => *system_offset_secs,
    };
    let (hour12, minute, second) = local_clock_components(&now, offset_secs);

    let mut draws: Vec<Draw> = Vec::with_capacity(20);
    push_gauges_and_labels(&mut draws, centre_x, centre_y, miner, seed_gauges);

    // Dial — single 390-native SVG, rendered at the viewport-derived `dial`
    // side and centred. Per-path `.fill()` overrides recolour the named paths
    // to the active palette; the SVG's stored colours act only as fallback.
    let dial_x = centre_x - dial / 2.0;
    let dial_y = centre_y - dial / 2.0;
    draws.push(
        Draw::svg(dial_x, dial_y, dial, dial, &DIAL_ROUND, TRANSPARENT)
            .with_anti_alias()
            .fill("ticks-small", palette.tick_small)
            .fill("ticks-large", palette.tick_large)
            .fill("rim-outer", palette.dial_rim),
    );

    // Timezone label inside the dial: two stacked lines, city on top,
    // signed `±HH:MM` offset below. The IANA region prefix is dropped
    // so the city fits the dial inner-rect on Small/Medium; the offset
    // line disambiguates same-named cities across regions.
    if params.show_timezone {
        let (city, offset_str, tz_color) = match &label {
            TzLabel::Resolved { city, offset_secs } => {
                let mut s = String::new();
                push_utc_offset(&mut s, *offset_secs);
                (city.clone(), s, palette.text)
            }
            TzLabel::Unknown { city, .. } => (city.clone(), "unknown".to_owned(), RED_50),
        };
        let city_size = size.timezone_font_size;
        let offset_size = city_size.saturating_mul(85) / 100;
        let line_h =
            f32::from(u16::try_from(city_size).expect("BUG: timezone font size fits in u16"))
                * 1.05;
        let group_centre_y = dial_top_y + dial * size.timezone_y_frac;
        let weight = font_weight(params.numbers_font_style);
        draws.push(Draw::text(
            centre_x,
            group_centre_y - line_h / 2.0,
            city,
            style!(
                size: city_size,
                weight: weight,
                color: tz_color,
                align: TextAlign::Center,
                valign: VerticalAlign::Center,
            ),
        ));
        draws.push(Draw::text(
            centre_x,
            group_centre_y + line_h / 2.0,
            offset_str,
            style!(
                size: offset_size,
                weight: weight,
                color: tz_color,
                align: TextAlign::Center,
                valign: VerticalAlign::Center,
            ),
        ));
    }

    // Date window (Full / Large only, when `show_date` is set).
    // 60×60 hollow circle with the day-of-month text inside;
    // the border ring is a 32-point Catmull-Rom closed path stroked at 1px.
    //
    // Drawn before the hands so the hour/minute/second hands sweep
    // over it; the design has the date window living *under* the hands.
    let numbers_weight = font_weight(params.numbers_font_style);
    if size.show_date_window && params.show_date {
        date_window(
            centre_x,
            dial_top_y,
            scale,
            &now,
            offset_secs,
            palette,
            numbers_weight,
            &mut draws,
        );
    }

    // Alarm row (Full only when an alarm is scheduled).
    // Right-anchored next to the dial's left edge so the group
    // reads as a satellite of the dial without overlapping it.
    if size.show_alarm {
        let dial_left_x = centre_x - dial / 2.0;
        let margin_to_dial = 32.0 * scale;
        let _ = alarm_row_draws(
            AlarmAnchor::RightX(dial_left_x - margin_to_dial),
            centre_y,
            24.0 * scale,
            numbers_weight,
            palette.alarm_bell,
            &mut draws,
        );
    }

    let h_ang = hour_angle(hour12, minute);
    let m_ang = minute_angle(minute);
    let s_ang = params.show_seconds.then(|| second_angle(second));
    super::push_hands_and_centre(
        centre_x, centre_y, scale, h_ang, m_ang, s_ang, palette, true, &mut draws,
    );

    canvas(props!(width: viewport_w, height: viewport_h), draws)
}

fn push_gauges_and_labels(
    draws: &mut Vec<Draw>,
    centre_x: f32,
    centre_y: f32,
    miner: &MinerData,
    seed_gauges: bool,
) {
    let mcr = miner::mcr_percent(miner.hashrate_ths, miner.nominal_hashrate_ths);
    let g = gauge::gauge(miner.hashrate_ths, mcr);
    let hashrate_fraction =
        miner::hashrate_fraction(miner.hashrate_ths, miner.nominal_hashrate_ths);

    // Lit states color both rings with the state fill and draw only the lit
    // portion, capping the inner ring one slot short of full. NotAvailable has
    // no color: both rings fill with the neutral gray track — the inner ring as
    // a complete circle — and the inner power label is suppressed.
    let (fill, outer_fraction, lit, show_power_label) = match ring_fill(g.state) {
        Some(fill) => (
            fill,
            hashrate_fraction,
            g.lit_count.min(gauge::TICK_COUNT - 1),
            true,
        ),
        None => (ArcFill::Solid(INACTIVE_TICK), 1.0, gauge::TICK_COUNT, false),
    };
    let inner_end = gauge::lit_sweep_end(lit);

    let hashrate_label_angle = live_end_angle(HASHRATE_SWEEP_END, outer_fraction);
    push_gauge_arc(
        draws,
        centre_x,
        centre_y,
        HASHRATE_RADIUS,
        HASHRATE_SWEEP_END,
        seeded_fraction(outer_fraction, seed_gauges),
        fill,
        &ArcSegments::Continuous,
        ArcCap::Round,
        "hashrate-gauge",
    );

    let inner_segments = ArcSegments::Explicit(gauge::TICK_SPANS.to_vec());
    let power_label_angle = (inner_end + POWER_LABEL_OFFSET).rem_euclid(std::f32::consts::TAU);
    push_gauge_arc(
        draws,
        centre_x,
        centre_y,
        POWER_RADIUS,
        inner_end,
        seeded_fraction(1.0, seed_gauges),
        fill,
        &inner_segments,
        ArcCap::Butt,
        "power-gauge",
    );

    draws.push(Draw::curved_text(
        centre_x,
        centre_y,
        HASHRATE_RADIUS,
        hashrate_label_angle,
        text_anchor_for_angle(hashrate_label_angle),
        text_facing_for_angle(hashrate_label_angle),
        hashrate_label(miner.hashrate_ths),
        style!(size: 18, weight: FontWeight::REGULAR, color: LABEL_GRAY),
    ));
    if show_power_label {
        draws.push(Draw::curved_text(
            centre_x,
            centre_y,
            POWER_RADIUS,
            power_label_angle,
            text_anchor_for_angle(power_label_angle),
            text_facing_for_angle(power_label_angle),
            power_label(miner.power_w),
            style!(size: 16, weight: FontWeight::REGULAR, color: LABEL_GRAY),
        ));
    }
}

fn live_end_angle(sweep_end: f32, fraction: f32) -> f32 {
    (sweep_end * fraction.clamp(0.0, 1.0) + HASHRATE_LABEL_OFFSET).rem_euclid(std::f32::consts::TAU)
}

// On the seed frame the ring draws empty so the host's end-angle transition has a
// zero baseline to animate the real fill in from, even when miner data is already
// present on the first frame. Labels and gradients keep their real values so only
// the sweep grows into place.
fn seeded_fraction(fraction: f32, seed: bool) -> f32 {
    if seed { 0.0 } else { fraction }
}

fn text_anchor_for_angle(angle: f32) -> ArcAnchor {
    if text_facing_for_angle(angle) == ArcTextFacing::Inward {
        ArcAnchor::End
    } else {
        ArcAnchor::Start
    }
}

fn text_facing_for_angle(angle: f32) -> ArcTextFacing {
    let angle = angle.rem_euclid(std::f32::consts::TAU);
    if (90.0_f32.to_radians()..=270.0_f32.to_radians()).contains(&angle) {
        ArcTextFacing::Inward
    } else {
        ArcTextFacing::Outward
    }
}

fn hashrate_label(value: Option<f64>) -> String {
    match value {
        Some(ths) => bmc_wasm_sdk::fmt!("{} TH/s", format_number!(ths, 2)),
        None => "N/A TH/s".to_owned(),
    }
}

fn power_label(value: Option<f64>) -> String {
    match value {
        Some(watts) => bmc_wasm_sdk::fmt!("{} W", format_number!(watts, 0)),
        None => "N/A W".to_owned(),
    }
}

// The ring is always emitted, even at a zero sweep, so the host keeps a stable
// transition slot for it across data loads and animates the end angle in place.
// Explicit segments hold absolute positions; the renderer clips them to the
// (possibly interpolated) end angle, so the full span set is passed through.
#[expect(
    clippy::too_many_arguments,
    reason = "flat gauge geometry helper keeps render call sites readable"
)]
fn push_gauge_arc(
    draws: &mut Vec<Draw>,
    centre_x: f32,
    centre_y: f32,
    radius: f32,
    sweep_end: f32,
    fraction: f32,
    fill: ArcFill,
    segments: &ArcSegments,
    cap: ArcCap,
    transition_id: &str,
) {
    let live_end = sweep_end * fraction.clamp(0.0, 1.0);
    draws.push(
        Draw::arc(
            centre_x,
            centre_y,
            radius,
            0.0,
            live_end,
            RING_WIDTH,
            fill,
            segments.clone(),
            cap,
        )
        .transition(transition_id, GAUGE_TRANSITION_MS, Easing::EaseOutCubic),
    );
}

// ── Date window ────────────────────────────────────────────────────────

const FRAC_1_SQRT_2: f32 = std::f32::consts::FRAC_1_SQRT_2;
const RING_POINTS_UNIT: [(f32, f32); 32] = [
    (1.0, 0.0),
    (0.980_785_25, 0.195_090_32),
    (0.923_879_5, 0.382_683_43),
    (0.831_469_6, 0.555_570_24),
    (FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (0.555_570_24, 0.831_469_6),
    (0.382_683_43, 0.923_879_5),
    (0.195_090_32, 0.980_785_25),
    (0.0, 1.0),
    (-0.195_090_32, 0.980_785_25),
    (-0.382_683_43, 0.923_879_5),
    (-0.555_570_24, 0.831_469_6),
    (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (-0.831_469_6, 0.555_570_24),
    (-0.923_879_5, 0.382_683_43),
    (-0.980_785_25, 0.195_090_32),
    (-1.0, 0.0),
    (-0.980_785_25, -0.195_090_32),
    (-0.923_879_5, -0.382_683_43),
    (-0.831_469_6, -0.555_570_24),
    (-FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
    (-0.555_570_24, -0.831_469_6),
    (-0.382_683_43, -0.923_879_5),
    (-0.195_090_32, -0.980_785_25),
    (0.0, -1.0),
    (0.195_090_32, -0.980_785_25),
    (0.382_683_43, -0.923_879_5),
    (0.555_570_24, -0.831_469_6),
    (FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
    (0.831_469_6, -0.555_570_24),
    (0.923_879_5, -0.382_683_43),
    (0.980_785_25, -0.195_090_32),
];

#[expect(
    clippy::too_many_arguments,
    reason = "flat geometry helper that forwards explicit dial-space metrics"
)]
fn date_window(
    centre_x: f32,
    dial_top_y: f32,
    scale: f32,
    now: &SystemTime,
    offset_secs: i32,
    palette: &ClockPalette,
    weight: FontWeight,
    draws: &mut Vec<Draw>,
) {
    // The date window is a 60×60 box anchored inside the dial inner-rect
    // (390-native); top-left y=250 puts its centre at native y=280. All these
    // native offsets scale with the dial, and `dial_top_y` then translates
    // into widget viewport coords.
    let cx = centre_x;
    let cy = dial_top_y + 280.0 * scale;
    let radius = 30.0 * scale;
    // 32-point closed smooth path → visually round 1px border.
    let ring_pts: Vec<(f32, f32)> = RING_POINTS_UNIT
        .iter()
        .map(|&(ux, uy)| (cx + ux * radius, cy + uy * radius))
        .collect();
    draws.push(Draw::path(
        ring_pts,
        1.0,
        palette.date_window,
        true,
        Interpolation::CatmullRom,
    ));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "scaled day-font size is a small positive widget-pixel value"
    )]
    let day_font = (24.0 * scale) as u32;
    // `VerticalAlign::Center` keeps the digit visually centred
    // on the ring instead of sitting half a font-size below it.
    let day_str = bmc_wasm_sdk::fmt!("{}", local_or_system(now, offset_secs).day);
    draws.push(Draw::text(
        cx,
        cy,
        day_str,
        style!(
            size: day_font,
            weight: weight,
            color: palette.date_window,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));
}
