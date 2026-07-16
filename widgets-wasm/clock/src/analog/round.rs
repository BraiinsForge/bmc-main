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
use crate::shared::{
    AlarmAnchor, ClockPalette, TzLabel, alarm_row_draws, f32_from_u32, font_weight,
    local_or_system, push_utc_offset, resolve_tz_for_label,
};

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

// 390 / 480 — reproduces the canonical 390 px dial on a 480 px-tall viewport,
// so Deck viewports render unchanged.
const DIAL_FRACTION: f32 = 0.812_5;

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
    /// Timezone label font size at the variant's canonical viewport; scaled
    /// down by `WidgetSize::fit` when the actual viewport is smaller.
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

pub(crate) fn render(
    now: SystemTime,
    params: &Params,
    ws: WidgetSize,
    tz: Option<&Tz>,
    palette: &ClockPalette,
) -> Node {
    let variant = ws.variant;
    let w = ws.width;
    let h = ws.height;
    let size = pick_size(variant);
    let viewport_w = f32_from_u32(w);
    let viewport_h = f32_from_u32(h);
    // Dial side as a fraction of the shorter viewport axis, so it always fits
    // and downscales rather than overflowing. `scale` maps the dial's 390
    // native geometry (hands, date ring) onto the rendered dial. The timezone
    // label is a per-variant text annotation and scales by `ws.fit()` instead.
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

    let mut draws: Vec<Draw> = Vec::with_capacity(16);

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
        // Per-variant authored size scaled by `fit`, so the label keeps its
        // legible size at each variant's canonical viewport and only shrinks
        // when the viewport is smaller — unlike the dial geometry above.
        let city_size = scale_font(size.timezone_font_size, ws.fit());
        let offset_size = city_size.saturating_mul(85) / 100;
        let line_h = f32_from_u32(city_size) * 1.05;
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
    let day_font = scale_font(24, scale);
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
