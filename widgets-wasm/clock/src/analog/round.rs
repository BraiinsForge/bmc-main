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
use crate::shared::{
    AlarmAnchor, ClockPalette, alarm_row_draws, effective_tz, font_weight, local_or_system,
    push_utc_offset,
};

use super::{
    CENTER_BLACK, CENTER_ORANGE, CENTER_STROKE, CENTER_WHITE, HAND_HOUR, HAND_HOUR_PIVOT_X,
    HAND_HOUR_PIVOT_Y, HAND_HOUR_VIEWPORT, HAND_MINUTE, HAND_MINUTE_PIVOT_X, HAND_MINUTE_PIVOT_Y,
    HAND_MINUTE_VIEWPORT, HAND_SECOND, HAND_SECOND_PIVOT_X, HAND_SECOND_PIVOT_Y,
    HAND_SECOND_VIEWPORT, centre_icon, centre_shadow, hand_shadow, hour_angle,
    local_clock_components, minute_angle, place_hand_at_pivot, second_angle,
};

// ── Asset ──────────────────────────────────────────────────────────────

const DIAL_ROUND: Svg = include_svg!("assets/analog/dial-round.svg");

// ── Per-size template parameters ───────────────────────────────────────
//
// Full and Large share the full-scale dial; Small
// and Medium use the half-scale dial.

#[derive(Clone, Copy)]
pub(crate) struct AnalogRoundSizeParams {
    /// Side of the square dial canvas, in widget pixels.
    /// The single `DIAL_ROUND` asset (390-native viewBox) is rendered at this size;
    /// Small / Medium scale it down to 195.
    canvas: f32,
    /// Render-side scale applied to hand viewports and pivot offsets.
    /// 1.0 for Full/Large, 0.5 for Small/Medium.
    scale: f32,
    /// Show the date window (lower-half of the dial). Full / Large only.
    show_date_window: bool,
    /// Show the alarm row (left of dial centre).
    /// Full only when `next_alarm = Some(_)`.
    show_alarm: bool,
    /// Y position of the timezone label inside the dial canvas.
    timezone_y: f32,
    /// Timezone label font size.
    timezone_font_size: u32,
}

const ANALOG_ROUND_FULL: AnalogRoundSizeParams = AnalogRoundSizeParams {
    canvas: 390.0,
    scale: 1.0,
    show_date_window: true,
    show_alarm: true,
    timezone_y: 104.0,
    timezone_font_size: 24,
};

const ANALOG_ROUND_LARGE: AnalogRoundSizeParams = AnalogRoundSizeParams {
    canvas: 390.0,
    scale: 1.0,
    show_date_window: true,
    show_alarm: false,
    timezone_y: 112.0,
    timezone_font_size: 24,
};

const ANALOG_ROUND_MEDIUM: AnalogRoundSizeParams = AnalogRoundSizeParams {
    canvas: 195.0,
    scale: 0.5,
    show_date_window: false,
    show_alarm: false,
    timezone_y: 53.0,
    timezone_font_size: 16,
};

const ANALOG_ROUND_SMALL: AnalogRoundSizeParams = AnalogRoundSizeParams {
    canvas: 195.0,
    scale: 0.5,
    show_date_window: false,
    show_alarm: false,
    timezone_y: 53.0,
    timezone_font_size: 16,
};

fn pick_size(w: u32, h: u32) -> &'static AnalogRoundSizeParams {
    match (w, h) {
        (1280, 480) => &ANALOG_ROUND_FULL,
        (638, 480) => &ANALOG_ROUND_LARGE,
        (638, 238) => &ANALOG_ROUND_MEDIUM,
        _ => &ANALOG_ROUND_SMALL,
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
    w: u32,
    h: u32,
    tz: Option<&Tz>,
    palette: &ClockPalette,
    viewport_w: f32,
    viewport_h: f32,
) -> Node {
    let size = pick_size(w, h);
    // Single canvas at the widget viewport size — the SDK's `Draw::rotated`
    // pivots around canvas centre, so making the canvas match the widget
    // and centering the dial at the canvas centre lets every hand rotate
    // around the dial midpoint without nested layout.
    let centre_x = viewport_w / 2.0;
    let centre_y = viewport_h / 2.0;
    let dial_top_y = centre_y - size.canvas / 2.0;
    let effective = effective_tz(tz);
    let (hour12, minute, second) = local_clock_components(&now, &effective);

    let mut draws: Vec<Draw> = Vec::with_capacity(16);

    // Dial — single 390-native SVG, rendered at `size.canvas`
    // so it scales 1:1 (Full / Large) or down to 195 (Small / Medium).
    // Per-path `.fill()` overrides recolour the named paths
    // to the active palette; the SVG's stored colours act
    // only as fallback.
    let dial_x = centre_x - size.canvas / 2.0;
    let dial_y = centre_y - size.canvas / 2.0;
    draws.push(
        Draw::svg(
            dial_x,
            dial_y,
            size.canvas,
            size.canvas,
            &DIAL_ROUND,
            TRANSPARENT,
        )
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
        let city = effective.city();
        let offset = resolve_tz_offset(&effective, now.unix_secs);
        let offset_str = match offset {
            Some(secs) => {
                let mut s = String::new();
                push_utc_offset(&mut s, secs);
                s
            }
            None => "unknown".to_owned(),
        };
        let tz_color = if offset.is_some() {
            palette.text
        } else {
            RED_50
        };
        let city_size = size.timezone_font_size;
        let offset_size = city_size.saturating_mul(85) / 100;
        let line_h =
            f32::from(u16::try_from(city_size).expect("BUG: timezone font size fits in u16"))
                * 1.05;
        let group_centre_y = dial_top_y + size.timezone_y;
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
            &now,
            &effective,
            palette,
            numbers_weight,
            &mut draws,
        );
    }

    // Alarm row (Full only when an alarm is scheduled).
    // Right-anchored next to the dial's left edge so the group
    // reads as a satellite of the dial without overlapping it.
    if size.show_alarm {
        let dial_left_x = centre_x - size.canvas / 2.0;
        let margin_to_dial = 32.0_f32;
        alarm_row_draws(
            AlarmAnchor::RightX(dial_left_x - margin_to_dial),
            centre_y,
            24.0,
            numbers_weight,
            palette.alarm_bell,
            &mut draws,
        );
    }

    // Hour and minute hands always rendered; second hand gated on `show_seconds`.
    // Each `place_hand_at_pivot` returns an icon with its SVG-coordinate pivot
    // at canvas centre, ready for in-place rotation.
    //
    // Transitions match the cadence used by hello-widget (`200ms` for the fast-moving
    // second hand, `500ms` for hour / minute) so the host's frame-by-frame interpolation
    // gives a smooth sweep without a per-frame wasm wake-up.
    let hour_angle = hour_angle(hour12, minute);
    let minute_angle = minute_angle(minute);

    // No shadow on the hour hand — lowest hand layer, its shadow would only
    // fall on the dark dial. The minute hand's lands on the hour hand.
    draws.push(
        Draw::rotated(
            hour_angle,
            place_hand_at_pivot(
                centre_x,
                centre_y,
                size.scale,
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
                size.scale,
                HAND_MINUTE_VIEWPORT,
                HAND_MINUTE_PIVOT_X,
                HAND_MINUTE_PIVOT_Y,
                &HAND_MINUTE,
                palette.primary,
            ),
        )
        .with_drop_shadow(hand_shadow(size.scale))
        .transition("minute-hand", 500, Easing::EaseOut),
    );

    // Centre-circle stack — order matters (later draws cover
    // earlier ones): big white disc, optional orange
    // (with second hand active), small black hole, white stroke ring.
    // The white disc's shadow makes it sit visibly above the hands.
    draws.push(
        centre_icon(
            centre_x,
            centre_y,
            size.scale,
            54.0,
            &CENTER_WHITE,
            palette.centre_white,
        )
        .with_drop_shadow(centre_shadow(size.scale)),
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
                    size.scale,
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
            size.scale,
            16.0,
            &CENTER_ORANGE,
            palette.centre_orange,
        ));
    }
    // The CENTER_BLACK disc always stays black — it's
    // the "hole" in the centre stack regardless of palette.
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.scale,
        8.0,
        &CENTER_BLACK,
        TRANSPARENT,
    ));
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.scale,
        10.0,
        &CENTER_STROKE,
        palette.centre_stroke,
    ));

    canvas(props!(width: viewport_w, height: viewport_h), draws)
}

// ── Date window ────────────────────────────────────────────────────────

fn date_window(
    centre_x: f32,
    dial_top_y: f32,
    now: &SystemTime,
    tz: &Tz,
    palette: &ClockPalette,
    weight: FontWeight,
    draws: &mut Vec<Draw>,
) {
    // The date window is a 60×60 box anchored inside the dial inner-rect
    // (390×390 at Full/Large); top-left y=250 puts its centre at y=280.
    // `dial_top_y` translates from inner-rect to widget viewport coords.
    let cx = centre_x;
    let cy = dial_top_y + 250.0 + 30.0;
    let radius = 30.0;
    // 32-point closed smooth path → visually round 1px border.
    let mut ring_pts: Vec<(f32, f32)> = Vec::with_capacity(32);
    for i in 0_u16..32 {
        let theta = (f32::from(i) / 32.0) * std::f32::consts::TAU;
        ring_pts.push((cx + radius * theta.cos(), cy + radius * theta.sin()));
    }
    draws.push(Draw::path(
        ring_pts,
        1.0,
        palette.date_window,
        true,
        false,
        Interpolation::CatmullRom,
    ));
    // `VerticalAlign::Center` keeps the digit visually centred
    // on the ring instead of sitting half a font-size below it.
    let day_str = format!("{}", local_or_system(now, tz).day);
    draws.push(Draw::text(
        cx,
        cy,
        day_str,
        style!(
            size: 24,
            weight: weight,
            color: palette.date_window,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));
}
