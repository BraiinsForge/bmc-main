// Copyright (C) 2026  Braiins Systems s.r.o.

//! Digital clock render mode — big numerals with an optional header
//! (date + timezone) and a fixed-height footer (AM/PM or alarm).

use bmc_wasm_sdk::system::TimeFormat;
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::Params;
use crate::shared::{
    AlarmAnchor, ClockPalette, TzLabel, alarm_row_draws, font_weight, local_or_system,
    push_utc_offset, resolve_tz_for_label,
};

// ── Per-size template parameters ───────────────────────────────────────

#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "fixed per-size tables encode several independent display toggles"
)]
pub(crate) struct DigitalSizeParams {
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

/// Resolve `widget_size()` dimensions to the matching per-size `const`.
/// The host delivers raw `(width, height)`;
/// we match the four canonical `WidgetSize` viewports (Full / Large / Medium / Small)
/// and fall back to `DIGITAL_SMALL` for anything outside the catalogue.
fn pick_size(w: u32, h: u32) -> &'static DigitalSizeParams {
    match (w, h) {
        (1280, 480) => &DIGITAL_FULL,
        (638, 480) => &DIGITAL_LARGE,
        (638, 238) => &DIGITAL_MEDIUM,
        _ => &DIGITAL_SMALL,
    }
}

// ── Render ─────────────────────────────────────────────────────────────

pub(crate) fn render(
    now: SystemTime,
    params: &Params,
    w: u32,
    h: u32,
    tz: Option<&Tz>,
    palette: &ClockPalette,
) -> Node {
    let size = pick_size(w, h);
    let is_12h = matches!(system::current().time_format(), Some(TimeFormat::Hour12));

    let label = resolve_tz_for_label(tz, now.unix_secs);
    let offset_secs = match &label {
        TzLabel::Resolved { offset_secs, .. } => *offset_secs,
        TzLabel::Unknown {
            system_offset_secs, ..
        } => *system_offset_secs,
    };

    let header_node = header(now, params, size, &label, palette);
    let time_node = time_row(now, params, size, is_12h, tz, offset_secs, palette);
    let ampm_row_node =
        (!size.ampm_inline && is_12h).then(|| ampm_line(now, size, offset_secs, palette));
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
    label: &TzLabel,
    palette: &ClockPalette,
) -> Option<Node> {
    if !params.show_date && !params.show_timezone {
        return None;
    }
    let offset_secs = match label {
        TzLabel::Resolved { offset_secs, .. } => *offset_secs,
        TzLabel::Unknown {
            system_offset_secs, ..
        } => *system_offset_secs,
    };
    let date_str = if params.show_date {
        compose_date(now, size, offset_secs)
    } else {
        String::new()
    };
    let (tz_str, tz_color) = if params.show_timezone {
        compose_timezone(label, size, palette)
    } else {
        (String::new(), palette.text)
    };
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
fn compose_date(now: SystemTime, size: &DigitalSizeParams, offset_secs: i32) -> String {
    let shifted = now.unix_secs + i64::from(offset_secs);
    let fmt = system::current().date_format().unwrap_or_default();
    strftime(
        shifted,
        date_pattern(fmt, size.show_weekday, size.show_year),
    )
}

/// Pick a strftime pattern for the long-form date row.
///
/// The system `DateFormat` enum is purely numeric
/// (separators + day/month/year ordering); the date row
/// is a *reading* row that uses spaces and month names,
/// so the separator choice is irrelevant here.
///
/// Only month-first ordering (US `MDYyyySlash`) is honoured
/// — year-first variants collapse to day-first because
/// "2026 Mon 3 March" reads as a log line, not a clock face.
pub(crate) fn date_pattern(
    format: system::DateFormat,
    show_weekday: bool,
    show_year: bool,
) -> &'static str {
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

fn compose_timezone(
    label: &TzLabel,
    size: &DigitalSizeParams,
    palette: &ClockPalette,
) -> (String, Color) {
    match label {
        TzLabel::Resolved { city, offset_secs } => {
            if size.show_utc_offset {
                let mut s = city.clone();
                s.push_str(" (");
                push_utc_offset(&mut s, *offset_secs);
                s.push(')');
                (s, palette.text)
            } else {
                (city.clone(), palette.text)
            }
        }
        TzLabel::Unknown { city, .. } => {
            if size.show_utc_offset {
                let mut s = city.clone();
                s.push_str(" (unknown)");
                (s, RED_50)
            } else {
                (city.clone(), RED_50)
            }
        }
    }
}

// ── Time row (time text + optional inline AM/PM) ───────────────────────

fn time_row(
    now: SystemTime,
    params: &Params,
    size: &DigitalSizeParams,
    is_12h: bool,
    tz: Option<&Tz>,
    offset_secs: i32,
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
        let ampm = ampm_glyph(now, offset_secs);
        // AM/PM sits to the right of the time text, separated by a 32px gap.
        // Pixel-exact anchoring to the time text's right edge would
        // need an anchor primitive the SDK doesn't yet expose;
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
    offset_secs: i32,
    palette: &ClockPalette,
) -> Node {
    center(
        props!(),
        [text(
            ampm_glyph(now, offset_secs),
            style!(
                size: u32::from(size.ampm_font_size),
                weight: FontWeight::REGULAR,
                color: palette.text
            ),
        )],
    )
}

/// `"AM"` / `"PM"` for the given moment in the effective timezone.
fn ampm_glyph(now: SystemTime, offset_secs: i32) -> &'static str {
    if local_or_system(&now, offset_secs).hour >= 12 {
        "PM"
    } else {
        "AM"
    }
}

// ── Alarm row ──────────────────────────────────────────────────────────

/// Build the digital alarm-row node: a fixed-size canvas
/// wrapping `alarm_row_draws`. `None` when no alarm is set.
fn alarm_row(size: &DigitalSizeParams, params: &Params, palette: &ClockPalette) -> Option<Node> {
    system::current().next_alarm()?;
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

// ── Spacer ─────────────────────────────────────────────────────────────

fn spacer_px(h: f32) -> Node {
    // Fixed-height spacer — empty col with explicit height.
    // The SDK's `spacer(flex)` primitive is flex-based;
    // for px-exact padding we emit a node with a literal
    // `height` prop instead.
    col(props!(height: h), Vec::<Node>::new())
}
