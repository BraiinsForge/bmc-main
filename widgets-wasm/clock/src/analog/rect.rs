// Copyright (C) 2026  Braiins Systems s.r.o.

//! Rectangular analog dial — per-size dial SVGs stretched to fill the
//! widget viewport, with numerals and timezone label overlaid.

// ── Drop shadows disabled ──────────────────────────────────────────────
//
// The minute-hand and centre-disc drop shadows are commented out below
// (and the `centre_shadow` / `hand_shadow` import is too). Each shadow
// renders through an offscreen FBO + Gaussian blur; on the Deck's Vivante
// GC400 that costs ~400 ms/frame (device-measured 2026-05-22), which on
// its own pushes the clock past its 1 s second-hand budget.
//
// Re-enable by uncommenting the import and the two `.with_drop_shadow(...)`
// calls — but only once the blur is precomputed or otherwise cheap on
// GC400-class hardware.

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

use super::{
    CENTER_BLACK, CENTER_ORANGE, CENTER_STROKE, CENTER_WHITE, HAND_HOUR, HAND_HOUR_PIVOT_X,
    HAND_HOUR_PIVOT_Y, HAND_HOUR_VIEWPORT, HAND_MINUTE, HAND_MINUTE_PIVOT_X, HAND_MINUTE_PIVOT_Y,
    HAND_MINUTE_VIEWPORT, HAND_SECOND, HAND_SECOND_PIVOT_X, HAND_SECOND_PIVOT_Y,
    HAND_SECOND_VIEWPORT, centre_icon, hour_angle, local_clock_components, minute_angle,
    place_hand_at_pivot, second_angle,
};
// Shadow helpers — used by the disabled drop-shadow calls (see top-of-file note):
// use super::{centre_shadow, hand_shadow};

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

fn pick_size(w: u32, h: u32) -> &'static AnalogRectSizeParams {
    match (w, h) {
        (1280, 480) => &ANALOG_RECT_FULL,
        (638, 480) => &ANALOG_RECT_LARGE,
        (638, 238) => &ANALOG_RECT_MEDIUM,
        _ => &ANALOG_RECT_SMALL,
    }
}

// ── Render ─────────────────────────────────────────────────────────────

#[expect(
    clippy::too_many_lines,
    reason = "this renderer intentionally keeps one draw-order-sensitive template together"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "render collects all frame inputs; a context struct is a later refactor"
)]
pub(crate) fn render(
    now: SystemTime,
    params: &Params,
    w: u32,
    h: u32,
    tz: Option<&Tz>,
    palette: &ClockPalette,
    viewport_w: f32,
    viewport_h: f32,
) -> Node {
    let size = pick_size(w, h);
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

    // Hour and minute hands always rendered;
    // second hand gated on `show_seconds`.
    //
    // The hand SVGs are shared with round mode,
    // so the existing round pivots apply.
    let hour_angle = hour_angle(hour12, minute);
    let minute_angle = minute_angle(minute);
    // No shadow on the hour hand — lowest hand layer,
    // its shadow would only fall on the dark dial.
    // The minute hand's lands on the hour hand.
    draws.push(
        Draw::rotated(
            hour_angle,
            place_hand_at_pivot(
                centre_x,
                centre_y,
                size.hand_scale,
                HAND_HOUR_VIEWPORT,
                HAND_HOUR_PIVOT_X,
                HAND_HOUR_PIVOT_Y,
                &HAND_HOUR,
                palette.primary,
            ),
        )
        .transition("hour-hand", 500, Easing::EaseOut),
    );
    draws.push(
        Draw::rotated(
            minute_angle,
            place_hand_at_pivot(
                centre_x,
                centre_y,
                size.hand_scale,
                HAND_MINUTE_VIEWPORT,
                HAND_MINUTE_PIVOT_X,
                HAND_MINUTE_PIVOT_Y,
                &HAND_MINUTE,
                palette.primary,
            ),
        )
        // .with_drop_shadow(hand_shadow(size.hand_scale))
        .transition("minute-hand", 500, Easing::EaseOut),
    );

    // Centre-circle stack — draw order matters.
    // White goes under the second hand so the hand reads against it;
    // orange covers the second-hand root; black + stroke sit on top regardless.
    draws.push(
        centre_icon(
            centre_x,
            centre_y,
            size.hand_scale,
            54.0,
            &CENTER_WHITE,
            palette.centre_white,
        ), // .with_drop_shadow(centre_shadow(size.hand_scale))
    );
    if params.show_seconds {
        // No shadow on the seconds hand — barely visible, real perf cost.
        let second_angle = second_angle(second);
        draws.push(
            Draw::rotated(
                second_angle,
                place_hand_at_pivot(
                    centre_x,
                    centre_y,
                    size.hand_scale,
                    HAND_SECOND_VIEWPORT,
                    HAND_SECOND_PIVOT_X,
                    HAND_SECOND_PIVOT_Y,
                    &HAND_SECOND,
                    palette.second_hand,
                ),
            )
            .transition("second-hand", 200, Easing::EaseOut),
        );
        draws.push(centre_icon(
            centre_x,
            centre_y,
            size.hand_scale,
            16.0,
            &CENTER_ORANGE,
            palette.centre_orange,
        ));
    }
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.hand_scale,
        8.0,
        &CENTER_BLACK,
        TRANSPARENT,
    ));
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.hand_scale,
        10.0,
        &CENTER_STROKE,
        palette.centre_stroke,
    ));

    canvas(props!(width: viewport_w, height: viewport_h), draws)
}
