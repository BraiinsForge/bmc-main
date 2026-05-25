// Copyright (C) 2026  Braiins Systems s.r.o.

//! Rectangular analog dial — per-size dial SVGs stretched to fill the
//! widget viewport, with numerals and timezone label overlaid.

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

use crate::digital::date_pattern;
use crate::manifest_params::Params;
use crate::shared::{
    AlarmAnchor, ClockPalette, TzLabel, alarm_row_draws, f32_from_u32, font_weight,
    push_utc_offset, resolve_tz_for_label,
};

use super::{hour_angle, local_clock_components, minute_angle, second_angle};

// ── Assets ─────────────────────────────────────────────────────────────

const DIAL_RECT_FULL: Svg = include_svg!("assets/analog/dial-rect-full.svg");
const DIAL_RECT_LARGE: Svg = include_svg!("assets/analog/dial-rect-large.svg");
const DIAL_RECT_MEDIUM: Svg = include_svg!("assets/analog/dial-rect-medium.svg");
const DIAL_RECT_SMALL: Svg = include_svg!("assets/analog/dial-rect-small.svg");

// ── Per-size template parameters ───────────────────────────────────────
//
// The per-size variation lives in the dial SVG itself (one per size);
// the renderer stretches whatever it's handed to the full viewport.

#[derive(Clone, Copy)]
pub(crate) struct AnalogRectSizeParams {
    dial: &'static Svg,
    hand_scale: f32,
    numerals_font_size: u32,
    twelve_top_inset: f32,
    three_right_inset: f32,
    six_bottom_inset: f32,
    nine_left_inset: f32,
    timezone_y: f32,
    timezone_font_size: u32,
    show_date_row: bool,
    show_alarm: bool,
}

const ANALOG_RECT_FULL: AnalogRectSizeParams = AnalogRectSizeParams {
    dial: &DIAL_RECT_FULL,
    hand_scale: 1.0,
    numerals_font_size: 64,
    twelve_top_inset: 59.0,
    three_right_inset: 105.0,
    six_bottom_inset: 59.0,
    nine_left_inset: 105.0,
    timezone_y: 265.0,
    timezone_font_size: 24,
    show_date_row: true,
    show_alarm: true,
};

const ANALOG_RECT_LARGE: AnalogRectSizeParams = AnalogRectSizeParams {
    dial: &DIAL_RECT_LARGE,
    hand_scale: 1.0,
    numerals_font_size: 40,
    twelve_top_inset: 80.0,
    three_right_inset: 90.0,
    six_bottom_inset: 80.0,
    nine_left_inset: 90.0,
    timezone_y: 270.0,
    timezone_font_size: 24,
    show_date_row: false,
    show_alarm: false,
};

const ANALOG_RECT_MEDIUM: AnalogRectSizeParams = AnalogRectSizeParams {
    dial: &DIAL_RECT_MEDIUM,
    hand_scale: 0.5,
    numerals_font_size: 24,
    twelve_top_inset: 35.0,
    three_right_inset: 50.0,
    six_bottom_inset: 35.0,
    nine_left_inset: 50.0,
    timezone_y: 130.0,
    timezone_font_size: 16,
    show_date_row: false,
    show_alarm: false,
};

const ANALOG_RECT_SMALL: AnalogRectSizeParams = AnalogRectSizeParams {
    dial: &DIAL_RECT_SMALL,
    hand_scale: 0.5,
    numerals_font_size: 24,
    twelve_top_inset: 40.0,
    three_right_inset: 50.0,
    six_bottom_inset: 40.0,
    nine_left_inset: 50.0,
    timezone_y: 130.0,
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

// ── Render ─────────────────────────────────────────────────────────────

#[expect(
    clippy::too_many_lines,
    reason = "this renderer intentionally keeps one draw-order-sensitive template together"
)]
pub(crate) fn render(
    now: SystemTime,
    params: &Params,
    variant: SizeVariant,
    w: u32,
    h: u32,
    tz: Option<&Tz>,
    palette: &ClockPalette,
) -> Node {
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

    // Dial fills the entire widget viewport.
    // Two recolourable named paths — major ticks (12/3/6/9)
    // and minor ticks — pick up the active palette.
    draws.push(
        Draw::svg(0.0, 0.0, viewport_w, viewport_h, size.dial, TRANSPARENT)
            .with_anti_alias()
            .fill("major", palette.tick_large)
            .fill("minor", palette.tick_small),
    );

    // Numerals 12 / 3 / 6 / 9. All four anchor at the glyph centre
    // (`VerticalAlign::Center`); 12 and 6 use a half-font-height
    // offset so their visible glyphs end up the same distance
    // from top/bottom edges.
    //
    // Mixing Top/Bottom for 12/6 with Center for 3/9 produced
    // uneven cap-vs-descender leading and read as bottom-heavy.
    let numerals_half = f32_from_u32(size.numerals_font_size) / 2.0;
    let numerals_center = style!(
        size: size.numerals_font_size,
        weight: numerals_weight,
        color: palette.text,
        align: TextAlign::Center,
        valign: VerticalAlign::Center,
    );
    let numerals_left = style!(
        size: size.numerals_font_size,
        weight: numerals_weight,
        color: palette.text,
        align: TextAlign::Left,
        valign: VerticalAlign::Center,
    );
    let numerals_right = style!(
        size: size.numerals_font_size,
        weight: numerals_weight,
        color: palette.text,
        align: TextAlign::Right,
        valign: VerticalAlign::Center,
    );
    draws.push(Draw::text(
        centre_x,
        size.twelve_top_inset + numerals_half,
        "12",
        numerals_center,
    ));
    draws.push(Draw::text(
        viewport_w - size.three_right_inset,
        centre_y,
        "3",
        numerals_right,
    ));
    draws.push(Draw::text(
        centre_x,
        viewport_h - size.six_bottom_inset - numerals_half,
        "6",
        numerals_center,
    ));
    draws.push(Draw::text(
        size.nine_left_inset,
        centre_y,
        "9",
        numerals_left,
    ));

    // Date row (Full only when show_date) — weekday + day-of-month + month name.
    // Vertically centred at the viewport mid-line, left-anchored at a fixed inset
    // that puts the row to the right of the dial graphic.
    if size.show_date_row && params.show_date {
        let shifted = now.unix_secs + i64::from(offset_secs);
        let fmt = system::current().date_format().unwrap_or_default();
        let date_str = strftime(shifted, date_pattern(fmt, true, false));
        draws.push(Draw::text(
            870.0,
            centre_y,
            date_str,
            style!(
                size: 40,
                weight: FontWeight::REGULAR,
                color: palette.text,
                align: TextAlign::Left,
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
            size.timezone_y,
            line,
            style!(
                size: size.timezone_font_size,
                weight: FontWeight::REGULAR,
                color: tz_color,
                align: TextAlign::Center,
                valign: VerticalAlign::Top,
            ),
        ));
    }

    // Alarm row (Full only when an alarm is scheduled).
    if size.show_alarm {
        alarm_row_draws(
            AlarmAnchor::LeftX(250.0),
            centre_y,
            40.0,
            numerals_weight,
            palette.alarm_bell,
            &mut draws,
        );
    }

    let h_ang = hour_angle(hour12, minute);
    let m_ang = minute_angle(minute);
    let s_ang = params.show_seconds.then(|| second_angle(second));
    super::push_hands_and_centre(
        centre_x,
        centre_y,
        size.hand_scale,
        h_ang,
        m_ang,
        s_ang,
        palette,
        false,
        &mut draws,
    );

    canvas(props!(width: viewport_w, height: viewport_h), draws)
}
