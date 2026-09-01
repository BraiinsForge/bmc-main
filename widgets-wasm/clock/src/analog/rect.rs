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

//! Rectangular analog dial — per-resolution dial SVGs stretched to fill
//! the widget viewport, with numerals and timezone label overlaid.

// ── Drop shadows disabled ──────────────────────────────────────────────
//
// The minute-hand and centre-disc drop shadows are suppressed via the
// `with_shadows = false` flag passed to `push_hands_and_centre`. Each
// shadow renders through an offscreen FBO + Gaussian blur; on the Deck's
// Vivante GC400 that costs ~400 ms/frame (device-measured 2026-05-22),
// which on its own pushes the clock past its 1 s second-hand budget.
//
// Re-enable by passing `true` — but only once the blur is precomputed or
// otherwise cheap on GC400-class hardware.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::Params;
use crate::shared::{
    AlarmAnchor, ClockPalette, DateOrder, TzLabel, alarm_row_draws, date_order, f32_from_u32,
    font_weight, push_utc_offset, resolve_tz_for_label,
};

use super::{hour_angle, local_clock_components, minute_angle, second_angle};

// ── Dials ──────────────────────────────────────────────────────────────
//
// Authored per exact viewport resolution and chosen independently of the
// size variant. A viewport with no matching dial simply draws no dial.

const DIAL_317X238: Svg = include_svg!("assets/analog/dial-rect-317x238.svg");
const DIAL_320X240: Svg = include_svg!("assets/analog/dial-rect-320x240.svg");
const DIAL_480X320: Svg = include_svg!("assets/analog/dial-rect-480x320.svg");
const DIAL_638X238: Svg = include_svg!("assets/analog/dial-rect-638x238.svg");
const DIAL_638X480: Svg = include_svg!("assets/analog/dial-rect-638x480.svg");
const DIAL_1280X480: Svg = include_svg!("assets/analog/dial-rect-1280x480.svg");

const DIALS_BY_RESOLUTION: &[(u32, u32, &Svg)] = &[
    (317, 238, &DIAL_317X238),
    (320, 240, &DIAL_320X240),
    (480, 320, &DIAL_480X320),
    (638, 238, &DIAL_638X238),
    (638, 480, &DIAL_638X480),
    (1280, 480, &DIAL_1280X480),
];

fn pick_dial(w: u32, h: u32) -> Option<&'static Svg> {
    DIALS_BY_RESOLUTION
        .iter()
        .find(|&&(rw, rh, _)| rw == w && rh == h)
        .map(|&(_, _, svg)| svg)
}

// ── Per-size template parameters ───────────────────────────────────────
//
// Layout metrics (numeral inset fractions, font sizes, hand scale) selected
// by the closest size variant, independent of the exact dial resolution.
// Insets are fractions of the viewport axis they sit against, so numerals
// stay proportionally placed when the actual viewport differs from the
// matched variant's canonical dimensions — e.g. BMM101's 480×320 under Large.

#[derive(Clone, Copy)]
pub(crate) struct AnalogRectSizeParams {
    numerals_font_size: u32,
    twelve_top_frac: f32,
    three_right_frac: f32,
    six_bottom_frac: f32,
    nine_left_frac: f32,
    timezone_font_size: u32,
    show_date_row: bool,
    show_alarm: bool,
}

const ANALOG_RECT_FULL: AnalogRectSizeParams = AnalogRectSizeParams {
    numerals_font_size: 64,
    twelve_top_frac: 0.175,
    three_right_frac: 0.1016,
    six_bottom_frac: 0.175,
    nine_left_frac: 0.1016,
    timezone_font_size: 24,
    show_date_row: true,
    show_alarm: true,
};

const ANALOG_RECT_LARGE: AnalogRectSizeParams = AnalogRectSizeParams {
    numerals_font_size: 40,
    twelve_top_frac: 0.1667,
    three_right_frac: 0.1254,
    six_bottom_frac: 0.1667,
    nine_left_frac: 0.1254,
    timezone_font_size: 24,
    show_date_row: false,
    show_alarm: false,
};

const ANALOG_RECT_MEDIUM: AnalogRectSizeParams = AnalogRectSizeParams {
    numerals_font_size: 24,
    twelve_top_frac: 0.1681,
    three_right_frac: 0.0940,
    six_bottom_frac: 0.1681,
    nine_left_frac: 0.0940,
    timezone_font_size: 16,
    show_date_row: false,
    show_alarm: false,
};

const ANALOG_RECT_SMALL: AnalogRectSizeParams = AnalogRectSizeParams {
    numerals_font_size: 24,
    twelve_top_frac: 0.1261,
    three_right_frac: 0.1262,
    six_bottom_frac: 0.1261,
    nine_left_frac: 0.1262,
    timezone_font_size: 16,
    show_date_row: false,
    show_alarm: false,
};

fn pick_size(variant: SizeVariant) -> &'static AnalogRectSizeParams {
    match variant {
        SizeVariant::Full => &ANALOG_RECT_FULL,
        SizeVariant::Large => &ANALOG_RECT_LARGE,
        SizeVariant::Medium => &ANALOG_RECT_MEDIUM,
        SizeVariant::Small => &ANALOG_RECT_SMALL,
    }
}

/// Abbreviates the month, unlike the digital date row,
/// to match the stable rectangular clock's date format.
fn analog_date_pattern(format: system::DateFormat) -> &'static str {
    match date_order(format) {
        DateOrder::MonthFirst => "%a, %b %-d",
        DateOrder::DayFirst => "%a %-d %b",
    }
}

// Timezone label baseline as a fraction of viewport height, measured from
// the top. A fraction (rather than a pixel inset) keeps the label in place
// when the actual viewport differs from the matched size variant's
// canonical dimensions — e.g. BMM101's 480×320 under Large params.
const TIMEZONE_Y_FRACTION: f32 = 0.55;

// Mirror the date and alarm anchors across the central dial.
const SIDE_ROW_X_INSET_FRACTION: f32 = 0.195;

// Hands scale with viewport height — the limiting axis on a landscape dial —
// so the hand tips keep the same margin inside the rim on every resolution.
// 480 px is the reference height at which the hand assets render at native
// scale; a shorter viewport (e.g. BMM101's 320) shrinks the hands to match.
const HAND_SCALE_REF_HEIGHT: f32 = 480.0;

// ── Render ─────────────────────────────────────────────────────────────

#[expect(
    clippy::too_many_lines,
    reason = "this renderer intentionally keeps one draw-order-sensitive template together"
)]
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
    let fit = ws.fit();
    let size = pick_size(variant);
    let viewport_w = f32_from_u32(w);
    let viewport_h = f32_from_u32(h);
    let centre_x = viewport_w / 2.0;
    let centre_y = viewport_h / 2.0;
    let label = resolve_tz_for_label(tz, now.unix_secs);
    let offset_secs = match &label {
        TzLabel::Resolved { offset_secs, .. } => *offset_secs,
        TzLabel::Unknown {
            system_offset_secs, ..
        } => *system_offset_secs,
    };
    let (hour12, minute, second) = local_clock_components(&now, offset_secs);
    let numerals_weight = font_weight(params.numbers_font_style);

    let mut draws: Vec<Draw> = Vec::with_capacity(20);

    // Dial fills the entire widget viewport — drawn only when a dial is
    // authored for this exact resolution. Two recolourable named paths —
    // major ticks (12/3/6/9) and minor ticks — pick up the active palette.
    if let Some(dial) = pick_dial(w, h) {
        draws.push(
            Draw::svg(0.0, 0.0, viewport_w, viewport_h, dial, TRANSPARENT)
                .with_anti_alias()
                .fill("major", palette.tick_large)
                .fill("minor", palette.tick_small),
        );
    }

    // Numerals 12 / 3 / 6 / 9 — all four anchor at the glyph centre
    // (`VerticalAlign::Center`); 12 and 6 offset by half-font-height
    // for a symmetric inset against the top/bottom edges.
    let numerals_size = scale_font(size.numerals_font_size, fit);
    let numerals_half = f32_from_u32(numerals_size) / 2.0;
    let numerals_center = style!(
        size: numerals_size,
        weight: numerals_weight,
        color: palette.tick_large,
        align: TextAlign::Center,
        valign: VerticalAlign::Center,
    );
    let numerals_left = style!(
        size: numerals_size,
        weight: numerals_weight,
        color: palette.tick_large,
        align: TextAlign::Left,
        valign: VerticalAlign::Center,
    );
    let numerals_right = style!(
        size: numerals_size,
        weight: numerals_weight,
        color: palette.tick_large,
        align: TextAlign::Right,
        valign: VerticalAlign::Center,
    );
    draws.push(Draw::text(
        centre_x,
        viewport_h * size.twelve_top_frac + numerals_half,
        "12",
        numerals_center,
    ));
    draws.push(Draw::text(
        viewport_w - viewport_w * size.three_right_frac,
        centre_y,
        "3",
        numerals_right,
    ));
    draws.push(Draw::text(
        centre_x,
        viewport_h - viewport_h * size.six_bottom_frac - numerals_half,
        "6",
        numerals_center,
    ));
    draws.push(Draw::text(
        viewport_w * size.nine_left_frac,
        centre_y,
        "9",
        numerals_left,
    ));

    // Date row (Full only when show_date) — weekday + day-of-month + month name.
    if size.show_date_row && params.show_date {
        let shifted = now.unix_secs + i64::from(offset_secs);
        let fmt = system::current().date_format().unwrap_or_default();
        let date_str = strftime(shifted, analog_date_pattern(fmt));
        draws.push(Draw::text(
            viewport_w * (1.0 - SIDE_ROW_X_INSET_FRACTION),
            centre_y,
            date_str,
            style!(
                size: scale_font(40, fit),
                weight: FontWeight::REGULAR,
                color: palette.text,
                align: TextAlign::Right,
                valign: VerticalAlign::Center,
            ),
        ));
    }

    // Timezone label — single line "City (±HH:MM)".
    // Unresolvable tz falls back to "(unknown)" and switches
    // to the night-red colour so an operator typo is visible at a glance.
    if params.show_timezone {
        let (line, tz_color) = match &label {
            TzLabel::Resolved { city, offset_secs } => {
                let mut s = String::with_capacity(city.len() + 10);
                s.push_str(city);
                s.push_str(" (");
                push_utc_offset(&mut s, *offset_secs);
                s.push(')');
                (s, palette.text)
            }
            TzLabel::Unknown { city, .. } => {
                let mut s = String::with_capacity(city.len() + 10);
                s.push_str(city);
                s.push_str(" (unknown)");
                (s, RED_50)
            }
        };
        draws.push(Draw::text(
            centre_x,
            viewport_h * TIMEZONE_Y_FRACTION,
            line,
            style!(
                size: scale_font(size.timezone_font_size, fit),
                weight: FontWeight::REGULAR,
                color: tz_color,
                align: TextAlign::Center,
                valign: VerticalAlign::Top,
            ),
        ));
    }

    // Alarm row (Full only when an alarm is scheduled).
    if size.show_alarm {
        let _ = alarm_row_draws(
            AlarmAnchor::LeftX(viewport_w * SIDE_ROW_X_INSET_FRACTION),
            centre_y,
            (40.0 * fit).round(),
            numerals_weight,
            palette.alarm_bell,
            &mut draws,
        );
    }

    let h_ang = hour_angle(hour12, minute);
    let m_ang = minute_angle(minute);
    let s_ang = params.show_seconds.then(|| second_angle(second));
    let hand_scale = viewport_h / HAND_SCALE_REF_HEIGHT;
    super::push_hands_and_centre(
        centre_x, centre_y, hand_scale, h_ang, m_ang, s_ang, palette, false, &mut draws,
    );

    canvas(props!(width: viewport_w, height: viewport_h), draws)
}
