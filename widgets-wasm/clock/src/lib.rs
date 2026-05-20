// Copyright (C) 2026  Braiins Systems s.r.o.

//! Clock widget — three render modes (analog round / analog rectangular / digital)
//! and four sizes (Small / Medium / Large / Full).

mod manifest_params;

use bmc_wasm_sdk::system::TimeFormat;
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use manifest_params::{ClockStyle, NumbersFontStyle, Params};

// ── Palette ────────────────────────────────────────────────────────────
// One set of colours per render mode (day / night).

#[derive(Clone, Copy)]
struct ClockPalette {
    /// Most prominent foreground: digital time text, hour and minute
    /// hands. White in day, red in night.
    primary: Color,
    /// Second-hand accent (orange in day, red in night).
    second_hand: Color,
    /// Subdued text: header date/timezone, AM/PM label.
    text: Color,
    /// Date-window day-number text and 1px border ring.
    date_window: Color,
    /// Alarm-bell glyph + adjacent alarm time text.
    alarm_bell: Color,
    /// Dial small ticks (between the big numeral positions).
    tick_small: Color,
    /// Dial big ticks at 12 / 3 / 6 / 9 positions.
    tick_large: Color,
    /// Dial outer rim ring.
    dial_rim: Color,
    /// Centre-circle stack — bigger white disc behind the hands.
    centre_white: Color,
    /// Centre-circle stack — orange pivot dot under the second hand.
    centre_orange: Color,
    /// Centre-circle stack — outer stroke ring.
    centre_stroke: Color,
}

const PALETTE_DAY: ClockPalette = ClockPalette {
    primary: WHITE,
    second_hand: ORANGE_40,
    text: GRAY_60,
    date_window: GRAY_60,
    alarm_bell: GRAY_60,
    tick_small: GRAY_70,
    tick_large: WHITE,
    dial_rim: GRAY_80,
    centre_white: WHITE,
    centre_orange: ORANGE_40,
    centre_stroke: GRAY_60,
};

const PALETTE_NIGHT: ClockPalette = ClockPalette {
    primary: RED_50,
    second_hand: RED_50,
    text: RED_50,
    date_window: RED_50,
    alarm_bell: RED_50,
    tick_small: RED_80,
    tick_large: RED_50,
    dial_rim: RED_100,
    centre_white: RED_50,
    centre_orange: RED_50,
    centre_stroke: RED_60,
};

/// Resolve `tz` → system tz → Etc/GMT (UTC) into a concrete `Tz`.
/// Projection paths (hands, digits) use this;
/// the timezone-line code instead consults the raw Option chain
/// so it can render a visible signal when nothing was configured.
fn effective_tz(tz: Option<&Tz>) -> Tz {
    if let Some(t) = tz {
        return t.clone();
    }
    if let Some(name) = system::current().timezone() {
        return Tz::from_runtime(name);
    }
    Tz::from_runtime("Etc/GMT")
}

/// `tz` → system tz → UTC fallback chain for projection paths
/// (hands, digits). Timezone-line code uses fallible `.local(tz)`
/// directly so it can render a visible signal on lookup failure.
fn local_or_system(now: &SystemTime, tz: &Tz) -> LocalDateTime {
    now.local(tz)
        .or_else(|| {
            system::current()
                .timezone()
                .and_then(|name| now.local(&Tz::from_runtime(name)))
        })
        .unwrap_or_else(|| now.utc())
}

fn local_unix_secs_or_system(now: &SystemTime, tz: &Tz) -> i64 {
    local_unix_secs(now, tz)
        .or_else(|| {
            system::current()
                .timezone()
                .and_then(|name| local_unix_secs(now, &Tz::from_runtime(name)))
        })
        .unwrap_or(now.unix_secs)
}

fn clock_palette(night_mode: bool) -> ClockPalette {
    if night_mode {
        PALETTE_NIGHT
    } else {
        PALETTE_DAY
    }
}

fn f32_from_u32(value: u32) -> f32 {
    f32::from(u16::try_from(value).expect("BUG: widget dimensions and font sizes fit in u16"))
}

fn f32_from_usize(value: usize) -> f32 {
    f32::from(u16::try_from(value).expect("BUG: short widget labels fit in u16"))
}

// ── Analog assets ─────────────────────────────────────────────────

const DIAL_ROUND: Svg = include_svg!("assets/analog/dial-round.svg");

const DIAL_RECT_FULL: Svg = include_svg!("assets/analog/dial-rect-full.svg");
const DIAL_RECT_LARGE: Svg = include_svg!("assets/analog/dial-rect-large.svg");
const DIAL_RECT_MEDIUM: Svg = include_svg!("assets/analog/dial-rect-medium.svg");
const DIAL_RECT_SMALL: Svg = include_svg!("assets/analog/dial-rect-small.svg");

const HAND_HOUR: Svg = include_svg!("assets/analog/hand-hour.svg");
const HAND_MINUTE: Svg = include_svg!("assets/analog/hand-minute.svg");
const HAND_SECOND: Svg = include_svg!("assets/analog/hand-second.svg");
const CENTER_WHITE: Svg = include_svg!("assets/analog/center-circle-white.svg");
const CENTER_ORANGE: Svg = include_svg!("assets/analog/center-circle-orange.svg");
const CENTER_BLACK: Svg = include_svg!("assets/analog/center-circle-black.svg");
const CENTER_STROKE: Svg = include_svg!("assets/analog/center-circle-stroke.svg");
const ALARM_BELL: Svg = include_svg!("assets/alarm-bell.svg");

// ── Per-size template parameters ───────────────────────────────────────
//
// Per-size variation lives in `const`s, not in branched code
// — one template function reads these.

#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "fixed per-size tables encode several independent display toggles"
)]
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

// ── AnalogRect per-size template parameters ──────────────────────────
//
// The per-size variation lives in the dial SVG itself (one per size);
// the renderer stretches whatever it's handed to the full viewport.

#[derive(Clone, Copy)]
struct AnalogRectSizeParams {
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
    let palette = clock_palette(system::current().night_mode().unwrap_or(false));
    let viewport_w = f32_from_u32(w);
    let viewport_h = f32_from_u32(h);

    let root = match params.clock_style {
        ClockStyle::AnalogRound => render_analog_round(
            now,
            &params,
            pick_analog_round_size(w, h),
            effective_tz.as_ref(),
            &palette,
            viewport_w,
            viewport_h,
        ),
        ClockStyle::AnalogRect => render_analog_rect(
            now,
            &params,
            pick_analog_rect_size(w, h),
            effective_tz.as_ref(),
            &palette,
            viewport_w,
            viewport_h,
        ),
        ClockStyle::Digital => render_digital(
            now,
            &params,
            pick_digital_size(w, h),
            effective_tz.as_ref(),
            &palette,
        ),
    };

    let _ = render_ui(w, h, root);
    // Re-render once per second so the displayed time advances.
    request_frame_after(1000);
}

/// Fires after every per-widget params delivery (operator change).
/// Trigger an immediate re-render so operator changes don't wait for
/// the next 1s tick.
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    request_frame();
}

/// Fires after every deck-wide system snapshot delivery
/// (timezone, formats, next-alarm, night-mode, …).
///
/// Same reason for immediate re-render — night-mode flips
/// shouldn't sit on screen for up to a second before
/// the palette swap takes effect.
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
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

fn pick_analog_rect_size(w: u32, h: u32) -> &'static AnalogRectSizeParams {
    match (w, h) {
        (1280, 480) => &ANALOG_RECT_FULL,
        (638, 480) => &ANALOG_RECT_LARGE,
        (638, 238) => &ANALOG_RECT_MEDIUM,
        _ => &ANALOG_RECT_SMALL,
    }
}

fn font_weight(style: NumbersFontStyle) -> FontWeight {
    match style {
        NumbersFontStyle::Regular => FontWeight::REGULAR,
        NumbersFontStyle::SemiBold => FontWeight::SEMIBOLD,
        NumbersFontStyle::Bold => FontWeight::BOLD,
    }
}

// ── Digital template ───────────────────────────────────────────────────

fn render_digital(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    tz: Option<&Tz>,
    palette: &ClockPalette,
) -> Node {
    let is_12h = matches!(system::current().time_format(), Some(TimeFormat::Hour12));

    let header_node = header(now, params, size, tz, palette);
    let time_node = time_row(now, params, size, is_12h, tz, palette);
    let ampm_row_node = (!size.ampm_inline && is_12h).then(|| ampm_line(now, size, tz, palette));
    let alarm_node = size
        .show_alarm
        .then(|| alarm_row(size, params, palette))
        .flatten();

    // Fixed-height slots above/below the flex-spaced time row,
    // so the time row's vertical centre stays put as header /
    // ampm / alarm toggle on and off.
    //
    // One row is enough: ampm-inline and show-alarm are mutually
    // exclusive per size (Full has alarm + inline ampm; the rest have
    // standalone ampm and no alarm), so the footer is at most one row.
    let slot_h = f32::from(size.header_font_size) * 1.2;
    let header_slot = col(
        props!(height: slot_h),
        header_node.into_iter().collect::<Vec<_>>(),
    );
    let footer_slot = col(
        props!(height: slot_h),
        [ampm_row_node, alarm_node]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
    );

    col(
        props!(flex: 1.0),
        [
            spacer_px(size.top_padding),
            header_slot,
            spacer(1.0),
            time_node,
            spacer(1.0),
            footer_slot,
            spacer_px(size.bottom_padding),
        ],
    )
}

// ── Header (date + timezone) ───────────────────────────────────────────

fn header(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    tz: Option<&Tz>,
    palette: &ClockPalette,
) -> Option<Node> {
    if !params.show_date && !params.show_timezone {
        return None;
    }
    let date_str = if params.show_date {
        compose_date(now, size, tz)
    } else {
        String::new()
    };
    let (tz_str, tz_resolved) = if params.show_timezone {
        compose_timezone(now, size, tz)
    } else {
        (String::new(), true)
    };
    let tz_color = if tz_resolved { palette.text } else { RED_50 };
    let date_style = style!(
        size: u32::from(size.header_font_size),
        weight: FontWeight::REGULAR,
        color: palette.text,
    );
    let tz_style = style!(
        size: u32::from(size.header_font_size),
        weight: FontWeight::REGULAR,
        color: tz_color,
    );
    let mut row_children: Vec<Node> = Vec::with_capacity(2);
    if !date_str.is_empty() {
        row_children.push(text(date_str, date_style));
    }
    if !tz_str.is_empty() {
        row_children.push(text(tz_str, tz_style));
    }
    Some(center(props!(), [row(props!(gap: 80.0), row_children)]))
}

/// Compose the date string from per-size visibility flags.
/// Uses strftime so the weekday / month-name come from
/// the host's chrono (correct locale + correct timezone).
fn compose_date(now: SystemTime, size: &DigitalSizeParams, tz: Option<&Tz>) -> String {
    let effective = effective_tz(tz);
    let shifted = local_unix_secs_or_system(&now, &effective);
    let fmt = system::current().date_format().unwrap_or_default();
    strftime(
        shifted,
        date_pattern(fmt, size.show_weekday, size.show_year),
    )
}

/// Pick a strftime pattern for the long-form date row.
///
/// The system `DateFormat` enum is purely numeric (separators + day/month/year ordering);
/// the date row is a *reading* row that uses spaces and month names,
/// so the separator choice is irrelevant here.
///
/// Only month-first ordering (US `MDYyyySlash`) is honoured — year-first
/// variants collapse to day-first because "2026 Mon 3 March" reads as
/// a log line, not a clock face.
fn date_pattern(format: system::DateFormat, show_weekday: bool, show_year: bool) -> &'static str {
    use system::DateFormat::MDYyyySlash;
    match (format, show_weekday, show_year) {
        (MDYyyySlash, true, false) => "%a, %B %-d",
        (MDYyyySlash, true, true) => "%a, %B %-d, %Y",
        (MDYyyySlash, false, false) => "%B %-d",
        (MDYyyySlash, false, true) => "%B %-d, %Y",
        (_, true, false) => "%a %-d %B",
        (_, true, true) => "%a %-d %B %Y",
        (_, false, false) => "%-d %B",
        (_, false, true) => "%-d %B %Y",
    }
}

/// City-only portion of an IANA timezone name; underscores normalised to spaces.
/// `Europe/Prague` → `Prague`; `America/New_York` → `New York`.
fn tz_city(iana: &str) -> String {
    iana.rsplit('/').next().unwrap_or(iana).replace('_', " ")
}

/// Returns `(label, resolved)` where `resolved` is `false` when the
/// host doesn't recognise the tz name — caller paints the label red.
fn compose_timezone(now: SystemTime, size: &DigitalSizeParams, tz: Option<&Tz>) -> (String, bool) {
    let effective = effective_tz(tz);
    let label = tz_city(effective.iana());
    let offset = resolve_tz_offset(&effective, now.unix_secs);
    if size.show_utc_offset {
        let mut s = label;
        s.push_str(" (");
        match offset {
            Some(secs) => push_utc_offset(&mut s, secs),
            None => s.push_str("unknown"),
        }
        s.push(')');
        (s, offset.is_some())
    } else {
        (label, offset.is_some())
    }
}

// ── Time row (time text + optional inline AM/PM) ───────────────────────

fn time_row(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    is_12h: bool,
    tz: Option<&Tz>,
    palette: &ClockPalette,
) -> Node {
    let weight = font_weight(params.numbers_font_style);
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
        style!(
            size: u32::from(size.time_font_size),
            weight: weight,
            color: palette.primary
        ),
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
                    style!(
                        size: u32::from(size.ampm_font_size),
                        weight: FontWeight::REGULAR,
                        color: palette.text
                    ),
                ),
            ],
        )
    } else {
        center(props!(), [time_node])
    }
}

fn ampm_line(
    now: SystemTime,
    size: &DigitalSizeParams,
    tz: Option<&Tz>,
    palette: &ClockPalette,
) -> Node {
    center(
        props!(),
        [text(
            ampm_glyph(now, tz),
            style!(
                size: u32::from(size.ampm_font_size),
                weight: FontWeight::REGULAR,
                color: palette.text
            ),
        )],
    )
}

/// `"AM"` / `"PM"` for the given moment in the effective timezone.
fn ampm_glyph(now: SystemTime, tz: Option<&Tz>) -> &'static str {
    let effective = effective_tz(tz);
    if local_or_system(&now, &effective).hour >= 12 {
        "PM"
    } else {
        "AM"
    }
}

// ── Alarm row — single renderer shared by digital + both analog modes ──

/// Where the bell+time group sits on the row.
#[derive(Copy, Clone)]
enum AlarmAnchor {
    /// Bell's left edge sits at x.
    LeftX(f32),
    /// Alarm-time text's right edge sits at x.
    RightX(f32),
}

/// Emit the alarm-bell + alarm-time pair at the given vertical centre `y`,
/// anchored per `AlarmAnchor`. Bell and text use a single `bell_size`
/// so they share the same line-box (font size = `bell_size as u32`).
/// Returns the rendered group width;
/// 0 when no alarm is set (no draws emitted).
fn alarm_row_draws(
    anchor: AlarmAnchor,
    y: f32,
    bell_size: f32,
    font_weight: FontWeight,
    color: Color,
    draws: &mut Vec<Draw>,
) -> f32 {
    let snap = system::current();
    let Some(alarm) = snap.next_alarm() else {
        return 0.0;
    };
    let is_12h = matches!(snap.time_format(), Some(TimeFormat::Hour12));
    let alarm_time = format_alarm_time(alarm, is_12h);
    let gap = 8.0_f32;
    // Approximate alarm-time width — the SDK doesn't expose text measurement
    // to widgets. `0.6 * font` per char is the tightest safe heuristic
    // for Inter `%H:%M` / `%I:%M %p` digits (each digit ≈ 0.55em,
    // plus headroom for the wider `M` glyph and trailing space in 12h mode).
    let approx_char_w = bell_size * 0.6;
    let approx_time_w = f32_from_usize(alarm_time.len()) * approx_char_w;
    let total_w = bell_size + gap + approx_time_w;
    let x = match anchor {
        AlarmAnchor::LeftX(x) => x,
        AlarmAnchor::RightX(rx) => rx - total_w,
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bell_size is a positive widget-pixel value well under u32 max"
    )]
    let font_size = bell_size as u32;
    draws.push(
        Draw::svg(
            x,
            y - bell_size / 2.0,
            bell_size,
            bell_size,
            &ALARM_BELL,
            color,
        )
        .with_anti_alias(),
    );
    draws.push(Draw::text(
        x + bell_size + gap,
        y,
        alarm_time,
        style!(
            size: font_size,
            weight: font_weight,
            color: color,
            align: TextAlign::Left,
            valign: VerticalAlign::Center,
        ),
    ));
    total_w
}

/// Build the digital alarm-row node: a fixed-size canvas
/// wrapping `alarm_row_draws`. `None` when no alarm is set.
fn alarm_row(size: &DigitalSizeParams, params: &Params, palette: &ClockPalette) -> Option<Node> {
    if system::current().next_alarm().is_none() {
        return None;
    }
    let bell = f32::from(size.header_font_size);
    let mut draws: Vec<Draw> = Vec::with_capacity(2);
    let total_w = alarm_row_draws(
        AlarmAnchor::LeftX(0.0),
        bell / 2.0,
        bell,
        font_weight(params.numbers_font_style),
        palette.alarm_bell,
        &mut draws,
    );
    if total_w <= 0.0 {
        return None;
    }
    Some(center(
        props!(),
        [canvas(props!(width: total_w, height: bell), draws)],
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

/// Uniform shrink applied to every hand viewport + its pivot
/// so the hand tips land inside the dial rim per the Figma reference.
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
const HAND_HOUR_VIEWPORT: f32 = 290.0 * HAND_SHRINK;
const HAND_HOUR_PIVOT_X: f32 = 144.5 * HAND_SHRINK;
const HAND_HOUR_PIVOT_Y: f32 = 121.0 * HAND_SHRINK;

/// Minute-hand: viewBox 0 0 50 400 → 400×400 viewport, letterbox 175
/// → pivot at (175 + 25, 200) = (200, 200) = viewport centre.
const HAND_MINUTE_VIEWPORT: f32 = 400.0 * HAND_SHRINK;
const HAND_MINUTE_PIVOT_X: f32 = 200.0 * HAND_SHRINK;
const HAND_MINUTE_PIVOT_Y: f32 = 200.0 * HAND_SHRINK;

/// Second-hand: viewBox 0 0 4 398 → 398×398 viewport, letterbox 197
/// → pivot at (197 + 2, 198) = (199, 198), basically viewport centre.
const HAND_SECOND_VIEWPORT: f32 = 398.0 * HAND_SHRINK;
const HAND_SECOND_PIVOT_X: f32 = 199.0 * HAND_SHRINK;
const HAND_SECOND_PIVOT_Y: f32 = 198.0 * HAND_SHRINK;

#[expect(
    clippy::too_many_lines,
    reason = "this renderer intentionally keeps one draw-order-sensitive template together"
)]
fn render_analog_round(
    now: SystemTime,
    params: &Params,
    size: &AnalogRoundSizeParams,
    tz: Option<&Tz>,
    palette: &ClockPalette,
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
        let iana_owned;
        let iana = if let Some(t) = tz {
            t.iana()
        } else {
            iana_owned = system::current().timezone().unwrap_or("Etc/GMT").to_owned();
            iana_owned.as_str()
        };
        let city = tz_city(iana);
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
        .transition("minute-hand", 500, Easing::EaseOut),
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
        palette.centre_white,
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
    // The CENTER_BLACK disc always stays black — it's the "hole"
    // in the centre stack regardless of palette.
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

#[expect(
    clippy::too_many_lines,
    reason = "this renderer intentionally keeps one draw-order-sensitive template together"
)]
fn render_analog_rect(
    now: SystemTime,
    params: &Params,
    size: &AnalogRectSizeParams,
    tz: Option<&Tz>,
    palette: &ClockPalette,
    viewport_w: f32,
    viewport_h: f32,
) -> Node {
    let centre_x = viewport_w / 2.0;
    let centre_y = viewport_h / 2.0;
    let effective = effective_tz(tz);
    let (hour12, minute, second) = local_clock_components(&now, &effective);
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
        let shifted = local_unix_secs_or_system(&now, &effective);
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
        let iana_owned;
        let iana = if let Some(t) = tz {
            t.iana()
        } else {
            iana_owned = system::current().timezone().unwrap_or("Etc/GMT").to_owned();
            iana_owned.as_str()
        };
        let city = tz_city(iana);
        let offset = resolve_tz_offset(&effective, now.unix_secs);
        let line = match offset {
            Some(secs) => {
                let mut s = String::with_capacity(city.len() + 10);
                s.push_str(&city);
                s.push_str(" (");
                push_utc_offset(&mut s, secs);
                s.push(')');
                s
            }
            None => {
                let mut s = String::with_capacity(city.len() + 10);
                s.push_str(&city);
                s.push_str(" (unknown)");
                s
            }
        };
        let tz_color = if offset.is_some() {
            palette.text
        } else {
            RED_50
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
        .transition("minute-hand", 500, Easing::EaseOut),
    );

    // Centre-circle stack — draw order matters.
    // White goes under the second hand so the hand reads against it;
    // orange covers the second-hand root; black + stroke sit on top regardless.
    draws.push(centre_icon(
        centre_x,
        centre_y,
        size.hand_scale,
        54.0,
        &CENTER_WHITE,
        palette.centre_white,
    ));
    if params.show_seconds {
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

fn local_clock_components(now: &SystemTime, tz: &Tz) -> (u8, u8, u8) {
    let local = local_or_system(now, tz);
    (local.hour, local.minute, local.second)
}

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

fn hour_angle(hour: u8, minute: u8) -> f32 {
    let hour12 = f32::from(hour % 12);
    let m = f32::from(minute);
    let target = (hour12 * 30.0 + m * 0.5).to_radians();
    HOUR_ANGLE_STATE.with(|s| unwrap_angle(s, target))
}

fn minute_angle(minute: u8) -> f32 {
    let target = (f32::from(minute) * 6.0).to_radians();
    MINUTE_ANGLE_STATE.with(|s| unwrap_angle(s, target))
}

fn second_angle(second: u8) -> f32 {
    let target = (f32::from(second) * 6.0).to_radians();
    SECOND_ANGLE_STATE.with(|s| unwrap_angle(s, target))
}

/// Position a hand icon at the canvas so its SVG-coordinate
/// pivot lands exactly on the canvas centre.
///
/// The SDK's rotation primitive then rotates the hand
/// around its own pivot (since the rotation axis is canvas-centre).
#[expect(
    clippy::too_many_arguments,
    reason = "hand placement is a flat geometry helper over explicit SVG metrics"
)]
fn place_hand_at_pivot(
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

fn centre_icon(
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

/// `+H` for whole-hour offsets, `+H:MM` for fractional.
/// Sign always emitted; hours unpadded; minutes zero-padded.
fn push_utc_offset(s: &mut String, offset_secs: i32) {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    let hours = abs / 3_600;
    let mins = (abs % 3_600) / 60;
    s.push(sign);
    push_u32(s, hours);
    if mins != 0 {
        s.push(':');
        push_pad2(s, mins);
    }
}
