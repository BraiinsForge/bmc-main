// Copyright (C) 2026  Braiins Systems s.r.o.

//! Clock widget — three render modes (analog round / analog rectangular / digital)
//! and four sizes (Small / Medium / Large / Full).

mod manifest_params;

use bmc_wasm_sdk::host::{SystemTime, request_frame, request_frame_after};
use bmc_wasm_sdk::system::TimeFormat;
use bmc_wasm_sdk::{
    Draw, Easing, FontWeight, FormatTimeOpts, GRAY_60, Node, Svg, TRANSPARENT, Tz, WHITE,
    WidgetSize, canvas, center, col, format_time, include_svg, local_unix_secs, props, render_ui,
    resolve_tz_offset, row, spacer, strftime, style, system, text, widget_size,
};

use manifest_params::{ClockStyle, NumbersFontStyle, Params};

// ── AnalogRound assets ─────────────────────────────────────────────────

const DIAL_ROUND: Svg = include_svg!("assets/round-dial.svg");
const HAND_HOUR_ROUND: Svg = include_svg!("assets/hand-hour.svg");
const HAND_MINUTE_ROUND: Svg = include_svg!("assets/hand-minute.svg");
const HAND_SECOND_ROUND: Svg = include_svg!("assets/hand-second.svg");
const CENTER_WHITE: Svg = include_svg!("assets/center-circle-white.svg");
const CENTER_ORANGE: Svg = include_svg!("assets/center-circle-orange.svg");
const CENTER_BLACK: Svg = include_svg!("assets/center-circle-black.svg");
const CENTER_STROKE: Svg = include_svg!("assets/center-circle-stroke.svg");
const ALARM_BELL: Svg = include_svg!("assets/alarm-bell.svg");

// ── Per-size template parameters ───────────────────────────────────────
//
// Per-size variation lives in `const`s, not in branched code
// — one template function reads these.

#[derive(Clone, Copy)]
struct DigitalSizeParams {
    time_font_size: u16,
    header_font_size: u16,
    ampm_font_size: u16,
    top_padding: f32,
    bottom_padding: f32,
    show_year: bool,
    show_weekday: bool,
    show_alarm: bool,
    ampm_inline: bool,
    show_utc_offset: bool,
}

const DIGITAL_FULL: DigitalSizeParams = DigitalSizeParams {
    time_font_size: 200,
    header_font_size: 40,
    ampm_font_size: 40,
    top_padding: 32.0,
    bottom_padding: 32.0,
    show_year: true,
    show_weekday: true,
    show_alarm: true,
    ampm_inline: true,
    show_utc_offset: true,
};

const DIGITAL_LARGE: DigitalSizeParams = DigitalSizeParams {
    time_font_size: 120,
    header_font_size: 32,
    ampm_font_size: 32,
    top_padding: 16.0,
    bottom_padding: 16.0,
    show_year: true,
    show_weekday: true,
    show_alarm: false,
    ampm_inline: false,
    show_utc_offset: true,
};

const DIGITAL_MEDIUM: DigitalSizeParams = DigitalSizeParams {
    time_font_size: 96,
    header_font_size: 24,
    ampm_font_size: 24,
    top_padding: 16.0,
    bottom_padding: 16.0,
    show_year: true,
    show_weekday: true,
    show_alarm: false,
    ampm_inline: false,
    show_utc_offset: true,
};

const DIGITAL_SMALL: DigitalSizeParams = DigitalSizeParams {
    time_font_size: 64,
    header_font_size: 24,
    ampm_font_size: 24,
    top_padding: 16.0,
    bottom_padding: 16.0,
    show_year: false,
    show_weekday: false,
    show_alarm: false,
    ampm_inline: false,
    show_utc_offset: false,
};

// ── AnalogRound per-size template parameters ──────────────────────────
//
// Full and Large share the full-scale dial; Small
// and Medium use the half-scale dial.

#[derive(Clone, Copy)]
struct AnalogRoundSizeParams {
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

// ── Lifecycle entries ──────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        ..
    } = widget_size();
    let now = SystemTime::now();
    let params = Params::current();
    let effective_tz = params.timezone_override.as_deref().map(Tz::from_runtime);

    let root = match params.clock_style {
        ClockStyle::Digital => {
            render_digital(now, &params, pick_digital_size(w, h), effective_tz.as_ref())
        }
        ClockStyle::AnalogRound => render_analog_round(
            now,
            &params,
            pick_analog_round_size(w, h),
            effective_tz.as_ref(),
            w as f32,
            h as f32,
        ),
        // AnalogRect has no dedicated renderer yet; route it to Digital
        // so an operator-set value doesn't blank the widget.
        ClockStyle::AnalogRect => {
            render_digital(now, &params, pick_digital_size(w, h), effective_tz.as_ref())
        }
    };

    let _ = render_ui(w, h, root);
    // Re-render once per second so the displayed time advances.
    request_frame_after(1000);
}

/// Fired on every params- or system-snapshot delivery after the first.
/// Trigger an immediate re-render so operator changes don't wait for
/// the next 1s tick.
///
/// The render path re-reads `Params::current()` and `system::current()`
/// itself, so no explicit diffing is needed here.
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    request_frame();
}

/// Resolve `widget_size()` dimensions to the matching per-size `const`.
/// The host delivers raw `(width, height)`;
/// we match the four canonical `WidgetSize` viewports (Full / Large / Medium / Small)
/// and fall back to `DIGITAL_SMALL` for anything outside the catalogue.
fn pick_digital_size(w: u32, h: u32) -> &'static DigitalSizeParams {
    match (w, h) {
        (1280, 480) => &DIGITAL_FULL,
        (638, 480) => &DIGITAL_LARGE,
        (638, 238) => &DIGITAL_MEDIUM,
        _ => &DIGITAL_SMALL,
    }
}

fn pick_analog_round_size(w: u32, h: u32) -> &'static AnalogRoundSizeParams {
    match (w, h) {
        (1280, 480) => &ANALOG_ROUND_FULL,
        (638, 480) => &ANALOG_ROUND_LARGE,
        (638, 238) => &ANALOG_ROUND_MEDIUM,
        _ => &ANALOG_ROUND_SMALL,
    }
}

fn font_weight(style: NumbersFontStyle) -> FontWeight {
    match style {
        NumbersFontStyle::Regular => FontWeight::Regular,
        NumbersFontStyle::SemiBold => FontWeight::SemiBold,
        NumbersFontStyle::Bold => FontWeight::Bold,
    }
}

// ── Digital template ───────────────────────────────────────────────────

fn render_digital(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    tz: Option<&Tz>,
) -> Node {
    let time_format = system::current().time_format();
    let is_12h = matches!(time_format, TimeFormat::Hour12);

    let header_node = header(now, params, size, tz);
    let time_node = time_row(now, params, size, is_12h, tz);
    let ampm_row_node = (!size.ampm_inline && is_12h).then(|| ampm_line(now, size, tz));
    let alarm_node = size.show_alarm.then(|| alarm_row(size, is_12h)).flatten();

    let mut children: Vec<Node> = Vec::with_capacity(8);
    // Top padding: PropsData has only uniform `padding`; emit a fixed-height
    // spacer node instead.
    children.push(spacer_px(size.top_padding));
    if let Some(n) = header_node {
        children.push(n);
    }
    children.push(spacer(1.0));
    children.push(time_node);
    children.push(spacer(1.0));
    if let Some(n) = ampm_row_node {
        children.push(n);
    }
    if let Some(n) = alarm_node {
        children.push(n);
    }
    children.push(spacer_px(size.bottom_padding));

    col(props!(flex: 1.0), children)
}

// ── Header (date + timezone) ───────────────────────────────────────────

fn header(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    tz: Option<&Tz>,
) -> Option<Node> {
    if !params.show_date && !params.show_timezone {
        return None;
    }
    let text_str = compose_header(now, params, size, tz);
    Some(center(
        props!(),
        [text(
            text_str,
            style!(size: size.header_font_size as u32, weight: 400, color: GRAY_60),
        )],
    ))
}

fn compose_header(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    tz: Option<&Tz>,
) -> String {
    match (params.show_date, params.show_timezone) {
        (true, true) => {
            let mut s = compose_date(now, size, tz);
            s.push_str("    ");
            s.push_str(&compose_timezone(now, size, tz));
            s
        }
        (true, false) => compose_date(now, size, tz),
        (false, true) => compose_timezone(now, size, tz),
        (false, false) => String::new(),
    }
}

/// Compose the date string from per-size visibility flags.
/// Uses strftime so the weekday / month-name come from
/// the host's chrono (correct locale + correct timezone
/// when `tz` is an override).
fn compose_date(now: SystemTime, size: &DigitalSizeParams, tz: Option<&Tz>) -> String {
    let shifted = local_unix_secs(&now, tz);
    let pattern = match (size.show_weekday, size.show_year) {
        (false, false) => "%-d %B",
        (true, false) => "%a %-d %B",
        (false, true) => "%-d %B, %Y",
        (true, true) => "%a %-d %B, %Y",
    };
    strftime(shifted, pattern)
}

fn compose_timezone(now: SystemTime, size: &DigitalSizeParams, tz: Option<&Tz>) -> String {
    let (label, offset_secs) = match tz {
        Some(tz) => (
            tz.iana().to_owned(),
            resolve_tz_offset(tz, now.unix_secs).unwrap_or(now.utc_offset_secs),
        ),
        None => (system::current().timezone().to_owned(), now.utc_offset_secs),
    };
    if size.show_utc_offset {
        let mut s = label;
        s.push_str(" (");
        push_utc_offset(&mut s, offset_secs);
        s.push(')');
        s
    } else {
        label
    }
}

// ── Time row (time text + optional inline AM/PM) ───────────────────────

fn time_row(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    is_12h: bool,
    tz: Option<&Tz>,
) -> Node {
    let weight = font_weight(params.numbers_font_style) as u16;
    let time_str = format_time(
        now,
        FormatTimeOpts {
            timezone: tz.cloned(),
            with_seconds: params.show_seconds,
            ..FormatTimeOpts::default()
        },
    );
    let time_node = text(
        time_str,
        style!(size: size.time_font_size as u32, weight: weight, color: WHITE),
    );

    if size.ampm_inline && is_12h {
        let ampm = ampm_glyph(now, tz);
        // AM/PM sits to the right of the time text, separated by a 32px gap.
        // Pixel-exact anchoring to the time text's right edge would need
        // an anchor primitive the SDK doesn't yet expose;
        // the visible gap is close enough for the in-line `Hour12` layout.
        row(
            props!(gap: 32.0),
            [
                time_node,
                text(
                    ampm,
                    style!(size: size.ampm_font_size as u32, weight: 400, color: GRAY_60),
                ),
            ],
        )
    } else {
        center(props!(), [time_node])
    }
}

fn ampm_line(now: SystemTime, size: &DigitalSizeParams, tz: Option<&Tz>) -> Node {
    center(
        props!(),
        [text(
            ampm_glyph(now, tz),
            style!(size: size.ampm_font_size as u32, weight: 400, color: GRAY_60),
        )],
    )
}

/// `"AM"` / `"PM"` for the given moment in the effective timezone.
/// Uses strftime so the AM/PM boundary follows the override tz,
/// not just the system tz.
fn ampm_glyph(now: SystemTime, tz: Option<&Tz>) -> &'static str {
    let shifted = local_unix_secs(&now, tz);
    // %H is 00–23; cheap branch on the leading digit beats
    // a host call to chrono's `%p` for the AM/PM string.
    let hour_str = strftime(shifted, "%H");
    let hour: u8 = hour_str.parse().unwrap_or(0);
    if hour >= 12 { "PM" } else { "AM" }
}

// ── Alarm row (Full only when next_alarm = Some) ───────────────────────

fn alarm_row(size: &DigitalSizeParams, is_12h: bool) -> Option<Node> {
    let snap = system::current();
    let alarm = snap.next_alarm()?;
    let alarm_time = format_alarm_time(alarm, is_12h);
    Some(center(
        props!(gap: 8.0),
        [text(
            alarm_time,
            style!(size: size.header_font_size as u32, weight: 400, color: GRAY_60),
        )],
    ))
}

fn format_alarm_time(alarm: bmc_wasm_sdk::system::NextAlarmView<'_>, is_12h: bool) -> String {
    let secs = alarm.fire_at_utc_ms / 1000;
    // Alarm fire time is operator-set in the system tz;
    // render it in the same tz here. `timezone_override`
    // doesn't shift the alarm row (alarms are a system-tz
    // construct, not an event-local-tz one).
    let pattern = if is_12h { "%I:%M %p" } else { "%H:%M" };
    strftime(secs, pattern)
}

// ── AnalogRound template ───────────────────────────────────────────────
//
// All hands rotate around the canvas centre via `Draw::rotated`.
// The SDK rotation primitive pivots on canvas-centre only (no per-image pivot),
// so each hand image is positioned such that its SVG-coordinate pivot lands
// at the canvas centre — see `place_hand_at_pivot`.
//
// Per-size template parameters live in `AnalogRoundSizeParams` consts above.

/// Hour-hand SVG dims: viewBox 0 0 63 290 rendered into a 290×290 square
/// via `xMidYMid meet` → content centered horizontally with (290 − 63) / 2 = 113.5 px
/// letterboxing on each side.
/// Pivot is (31, 121) in viewBox coords;
/// in render coords that becomes (113.5 + 31, 121) = (144.5, 121).
const HAND_HOUR_VIEWPORT: f32 = 290.0;
const HAND_HOUR_PIVOT_X: f32 = 144.5;
const HAND_HOUR_PIVOT_Y: f32 = 121.0;

/// Minute-hand: viewBox 0 0 50 400 → 400×400 viewport, letterbox 175 →
/// pivot at (175 + 25, 200) = (200, 200) = viewport centre.
const HAND_MINUTE_VIEWPORT: f32 = 400.0;
const HAND_MINUTE_PIVOT_X: f32 = 200.0;
const HAND_MINUTE_PIVOT_Y: f32 = 200.0;

/// Second-hand: viewBox 0 0 4 398 → 398×398 viewport, letterbox 197 →
/// pivot at (197 + 2, 198) = (199, 198), basically viewport centre.
const HAND_SECOND_VIEWPORT: f32 = 398.0;
const HAND_SECOND_PIVOT_X: f32 = 199.0;
const HAND_SECOND_PIVOT_Y: f32 = 198.0;

fn render_analog_round(
    now: SystemTime,
    params: &Params,
    size: &AnalogRoundSizeParams,
    tz: Option<&Tz>,
    viewport_w: f32,
    viewport_h: f32,
) -> Node {
    // Single canvas at the widget viewport size — the SDK's `Draw::rotated`
    // pivots around canvas centre, so making the canvas match the widget
    // and centering the dial at the canvas centre lets every hand rotate
    // around the dial midpoint without nested layout.
    let centre_x = viewport_w / 2.0;
    let centre_y = viewport_h / 2.0;
    let dial_top_y = centre_y - size.canvas / 2.0;
    // Decomposed local time in the effective timezone — the SystemTime
    // struct holds system-tz fields; for an override we re-derive hour
    // / minute / second from `local_unix_secs`.
    let (hour12, minute, second) = local_clock_components(&now, tz);

    let mut draws: Vec<Draw> = Vec::with_capacity(16);

    // Dial — single 390-native SVG, rendered at `size.canvas`
    // so the dial scales 1:1 (Full / Large) or down to 195 (Small / Medium).
    // Centred on the viewport midpoint so its midpoint coincides
    // with the hand pivots.
    let dial_x = centre_x - size.canvas / 2.0;
    let dial_y = centre_y - size.canvas / 2.0;
    draws.push(Draw::svg(
        dial_x,
        dial_y,
        size.canvas,
        size.canvas,
        &DIAL_ROUND,
        TRANSPARENT,
    ));

    // Timezone text inside the dial. `size.timezone_y` is in dial-inner-rect
    // coords (390 or 195); we translate to widget viewport via `dial_top_y`.
    if params.show_timezone {
        let label = tz.map_or_else(
            || system::current().timezone().to_owned(),
            |t| t.iana().to_owned(),
        );
        draws.push(Draw::text(
            centre_x,
            dial_top_y + size.timezone_y,
            label,
            style!(
                size: size.timezone_font_size,
                weight: 400,
                color: GRAY_60,
                align: bmc_wasm_sdk::TextAlign::Center,
            ),
        ));
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
                &HAND_HOUR_ROUND,
            ),
        )
        .transition(500, Easing::EaseOut),
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
                &HAND_MINUTE_ROUND,
            ),
        )
        .transition(500, Easing::EaseOut),
    );

    // Centre-circle stack — order matters (later draws cover earlier ones):
    // big white disc, optional orange (with second hand active),
    // small black hole, white stroke ring.
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.scale,
        54.0,
        &CENTER_WHITE,
    ));

    if params.show_seconds {
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
                    &HAND_SECOND_ROUND,
                ),
            )
            .transition(200, Easing::EaseOut),
        );
        draws.push(centre_icon(
            centre_x,
            centre_y,
            size.scale,
            16.0,
            &CENTER_ORANGE,
        ));
    }
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.scale,
        8.0,
        &CENTER_BLACK,
    ));
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.scale,
        10.0,
        &CENTER_STROKE,
    ));

    // Date window (Full / Large only, when `show_date` is set).
    // 60×60 hollow circle with the day-of-month text inside; the border
    // ring is a 32-point Catmull-Rom closed path stroked at 1px.
    if size.show_date_window && params.show_date {
        date_window(centre_x, dial_top_y, &now, tz, &mut draws);
    }

    // Alarm row (Full only when an alarm is scheduled).
    //
    // The design floats this left of the dial in the wider Full viewport;
    // without a "draw outside the dial canvas" primitive we anchor it
    // below the dial centre instead. The visual baseline review will
    // confirm or push back on this placement.
    if size.show_alarm {
        analog_alarm_row(centre_x, dial_top_y, &mut draws);
    }

    canvas(props!(width: viewport_w, height: viewport_h), draws)
}

fn local_clock_components(now: &SystemTime, tz: Option<&Tz>) -> (u8, u8, u8) {
    match tz {
        None => (now.hour, now.minute, now.second),
        Some(_) => {
            // Override path — strftime decomposes the shifted unix_secs.
            let shifted = local_unix_secs(now, tz);
            let hms = strftime(shifted, "%H %M %S");
            let mut parts = hms.split_ascii_whitespace();
            let h: u8 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let m: u8 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let s: u8 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            (h, m, s)
        }
    }
}

/// Hour-hand angle: `hour12 * 30° + minute * 0.5°`.
/// Returned in radians for the SDK's rotation primitive.
/// The half-degree-per-minute creep matches a real analog clock
/// — the hour hand drifts smoothly between hour ticks instead
///   of snapping on the hour.
fn hour_angle(hour: u8, minute: u8) -> f32 {
    let hour12 = f32::from(hour % 12);
    let m = f32::from(minute);
    (hour12 * 30.0 + m * 0.5).to_radians()
}

fn minute_angle(minute: u8) -> f32 {
    (f32::from(minute) * 6.0).to_radians()
}

fn second_angle(second: u8) -> f32 {
    (f32::from(second) * 6.0).to_radians()
}

/// Position a hand icon at the canvas so its SVG-coordinate
/// pivot lands exactly on the canvas centre.
///
/// The SDK's rotation primitive then rotates the hand
/// around its own pivot (since the rotation axis is canvas-centre).
fn place_hand_at_pivot(
    centre_x: f32,
    centre_y: f32,
    scale: f32,
    viewport: f32,
    pivot_x: f32,
    pivot_y: f32,
    icon: &'static Svg,
) -> Draw {
    let w = viewport * scale;
    let h = viewport * scale;
    let top_left_x = centre_x - pivot_x * scale;
    let top_left_y = centre_y - pivot_y * scale;
    Draw::svg(top_left_x, top_left_y, w, h, icon, TRANSPARENT)
}

fn centre_icon(
    centre_x: f32,
    centre_y: f32,
    scale: f32,
    native_side: f32,
    icon: &'static Svg,
) -> Draw {
    let side = native_side * scale;
    let top_left_x = centre_x - side / 2.0;
    let top_left_y = centre_y - side / 2.0;
    Draw::svg(top_left_x, top_left_y, side, side, icon, TRANSPARENT)
}

fn date_window(
    centre_x: f32,
    dial_top_y: f32,
    now: &SystemTime,
    tz: Option<&Tz>,
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
    for i in 0..32 {
        let theta = (i as f32 / 32.0) * std::f32::consts::TAU;
        ring_pts.push((cx + radius * theta.cos(), cy + radius * theta.sin()));
    }
    draws.push(Draw::path(
        ring_pts,
        1.0,
        GRAY_60,
        true,
        false,
        bmc_wasm_sdk::Interpolation::CatmullRom,
    ));
    // Day number — read decomposed `day` in the effective timezone.
    let day_str = match tz {
        None => format!("{}", now.day),
        Some(_) => strftime(local_unix_secs(now, tz), "%-d"),
    };
    draws.push(Draw::text(
        cx,
        cy,
        day_str,
        style!(
            size: 24,
            weight: 400,
            color: GRAY_60,
            align: bmc_wasm_sdk::TextAlign::Center,
        ),
    ));
}

fn analog_alarm_row(centre_x: f32, dial_top_y: f32, draws: &mut Vec<Draw>) {
    let snap = system::current();
    let Some(alarm) = snap.next_alarm() else {
        return;
    };
    // ALARM_BELL is registered with the host so the asset is ready for
    // draw; the row renders text-only because the design pairs the time
    // with a palette-tinted bell and the SDK has no per-path SVG tint.
    let _ = ALARM_BELL;
    let is_12h = matches!(system::current().time_format(), TimeFormat::Hour12);
    let alarm_time = format_alarm_time(alarm, is_12h);
    // y=178 inside the dial inner-rect, horizontally centred
    // under the dial centre.
    //
    // The design floats this row left of the dial; without
    // an out-of-canvas anchor primitive we centre it instead.
    draws.push(Draw::text(
        centre_x,
        dial_top_y + 178.0,
        alarm_time,
        style!(
            size: 24,
            weight: 400,
            color: GRAY_60,
            align: bmc_wasm_sdk::TextAlign::Center,
        ),
    ));
}

// ── Spacers and number helpers ─────────────────────────────────────────

fn spacer_px(h: f32) -> Node {
    // Fixed-height spacer — empty col with explicit height. The SDK's
    // `spacer(flex)` primitive is flex-based; for px-exact padding we
    // emit a node with a literal `height` prop instead.
    col(props!(height: h), Vec::<Node>::new())
}

fn push_u32(s: &mut String, n: u32) {
    if n >= 10 {
        push_u32(s, n / 10);
    }
    s.push((b'0' + (n % 10) as u8) as char);
}

fn push_pad2(s: &mut String, n: u32) {
    if n < 10 {
        s.push('0');
    }
    push_u32(s, n);
}

fn push_utc_offset(s: &mut String, offset_secs: i32) {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    let hours = abs / 3_600;
    let mins = (abs % 3_600) / 60;
    s.push(sign);
    push_pad2(s, hours);
    s.push(':');
    push_pad2(s, mins);
}
