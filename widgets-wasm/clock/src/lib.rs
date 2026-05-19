// Copyright (C) 2026  Braiins Systems s.r.o.

//! Clock widget — three render modes (analog round / analog rectangular / digital)
//! and four sizes (Small / Medium / Large / Full).

use bmc_wasm_sdk::host::SystemTime;
use bmc_wasm_sdk::system::TimeFormat;
use bmc_wasm_sdk::{
    FontWeight, FormatTimeOpts, GRAY_60, Node, WHITE, WidgetSize, center, col, format_time, props,
    render_ui, request_frame_after, row, spacer, style, system, text, widget_size,
};

// ── Per-size template parameters ───────────────────────────────────────
//
// Mirror of the `WidgetClockDigitalTemplate` instantiations at
// `bmc-display/ui/widgets/categories/clock.slint:520-609` on `bmc/stable-26.02`.
// Per-size variation lives in `const`s, not in branched code — one template
// function reads these.

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

/// Manifest-derived widget params. Stage A defaults are the canonical
/// `ClockWidget::Default` values from `bmc-display/src/data.rs`. Live
/// per-instance values land in Stage C.
#[derive(Clone, Copy)]
struct ClockParams {
    show_date: bool,
    show_seconds: bool,
    show_timezone: bool,
    numbers_weight: FontWeight,
}

impl Default for ClockParams {
    fn default() -> Self {
        Self {
            show_date: true,
            show_seconds: true,
            show_timezone: true,
            numbers_weight: FontWeight::SemiBold,
        }
    }
}

// ── Top-level entry ────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        ..
    } = widget_size();
    let now = SystemTime::now();
    let params = ClockParams::default();
    let size = pick_size(w, h);

    let root = render_digital(now, params, size);

    let _ = render_ui(w, h, root);
    // Re-render once per second so the displayed time advances.
    request_frame_after(1000);
}

/// Resolve `widget_size()` dimensions to the matching per-size `const`.
/// The slint widget uses an enum `WidgetSize`; the host delivers raw
/// dimensions, so we match the four canonical sizes from
/// `clock.slint:520-609`.
fn pick_size(w: u32, h: u32) -> &'static DigitalSizeParams {
    match (w, h) {
        (1280, 480) => &DIGITAL_FULL,
        (638, 480) => &DIGITAL_LARGE,
        (638, 238) => &DIGITAL_MEDIUM,
        _ => &DIGITAL_SMALL,
    }
}

// ── Digital template ───────────────────────────────────────────────────

fn render_digital(now: SystemTime, params: ClockParams, size: &DigitalSizeParams) -> Node {
    let time_format = system::current().time_format();
    let is_12h = matches!(time_format, TimeFormat::Hour12);

    let header_node = header(now, params, size);
    let time_node = time_row(now, params, size, is_12h);
    let ampm_row_node = (!size.ampm_inline && is_12h).then(|| ampm_line(now, size));
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

fn header(now: SystemTime, params: ClockParams, size: &DigitalSizeParams) -> Option<Node> {
    if !params.show_date && !params.show_timezone {
        return None;
    }
    let text_str = compose_header(now, params, size);
    Some(center(
        props!(),
        [text(
            text_str,
            style!(size: size.header_font_size as u32, weight: 400, color: GRAY_60),
        )],
    ))
}

fn compose_header(now: SystemTime, params: ClockParams, size: &DigitalSizeParams) -> String {
    match (params.show_date, params.show_timezone) {
        (true, true) => {
            let mut s = compose_date(now, size);
            s.push_str("    ");
            s.push_str(&compose_timezone(size));
            s
        }
        (true, false) => compose_date(now, size),
        (false, true) => compose_timezone(size),
        (false, false) => String::new(),
    }
}

fn compose_date(now: SystemTime, size: &DigitalSizeParams) -> String {
    let mut s = String::new();
    if size.show_weekday {
        s.push_str(weekday_name(now.weekday));
        s.push(' ');
    }
    push_u8(&mut s, now.day);
    s.push(' ');
    s.push_str(month_name(now.month));
    if size.show_year {
        s.push_str(", ");
        push_u16(&mut s, now.year);
    }
    s
}

fn compose_timezone(size: &DigitalSizeParams) -> String {
    let snap = system::current();
    let tz = snap.timezone();
    if size.show_utc_offset {
        // Format the system's current offset; SystemTime carries it as
        // signed seconds and the host has already applied it to the
        // decomposed fields.
        let offset_secs = SystemTime::now().utc_offset_secs;
        let mut s = String::from(tz);
        s.push_str(" (");
        push_utc_offset(&mut s, offset_secs);
        s.push(')');
        s
    } else {
        tz.to_owned()
    }
}

// ── Time row (time text + optional inline AM/PM) ───────────────────────

fn time_row(now: SystemTime, params: ClockParams, size: &DigitalSizeParams, is_12h: bool) -> Node {
    let time_str = format_time(
        now,
        FormatTimeOpts {
            with_seconds: params.show_seconds,
            ..FormatTimeOpts::default()
        },
    );
    let time_node = text(
        time_str,
        style!(size: size.time_font_size as u32, weight: params.numbers_weight as u16, color: WHITE),
    );

    if size.ampm_inline && is_12h {
        let ampm = ampm_glyph(now);
        // AM/PM placed to the right of the time text. The slint sets
        // `x: time-text.x + time-text.width + 32px` and a y-offset; we
        // approximate with a row + gap. Exact pixel-level placement is
        // out of scope until the SDK exposes the same anchor primitive.
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

fn ampm_line(now: SystemTime, size: &DigitalSizeParams) -> Node {
    center(
        props!(),
        [text(
            ampm_glyph(now),
            style!(size: size.ampm_font_size as u32, weight: 400, color: GRAY_60),
        )],
    )
}

fn ampm_glyph(now: SystemTime) -> &'static str {
    if now.hour >= 12 { "PM" } else { "AM" }
}

// ── Alarm row (Full only when next_alarm = Some) ───────────────────────

fn alarm_row(size: &DigitalSizeParams, is_12h: bool) -> Option<Node> {
    let snap = system::current();
    let alarm = snap.next_alarm()?;
    // SDK gap: slint colorizes the alarm-bell glyph (Palette.gray-40);
    // until the SDK exposes a tint primitive, render the formatted time
    // alone and revisit the bell icon when image colorize ships
    // (see CLOCK-WIDGET-WASM-PORT-PLAN.md, SDK gaps #1).
    let _ = alarm;
    let alarm_time = format_alarm_time(snap.next_alarm()?, is_12h);
    // Bottom padding is appended as a separate spacer in `render_digital`,
    // not as a prop on the alarm row (PropsData has only uniform `padding`).
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
    // Reuse the canonical strftime path so the formatting flows through
    // the same host-side chrono pipeline as the main clock readout.
    let pattern = if is_12h { "%I:%M %p" } else { "%H:%M" };
    bmc_wasm_sdk::strftime(secs, pattern)
}

// ── Spacers and number helpers ─────────────────────────────────────────

fn spacer_px(h: f32) -> Node {
    // Fixed-height spacer — empty col with explicit height. The SDK's
    // `spacer(flex)` primitive is flex-based; for px-exact padding we
    // emit a node with a literal `height` prop instead.
    col(props!(height: h), Vec::<Node>::new())
}

fn push_u8(s: &mut String, n: u8) {
    push_u32(s, n as u32);
}

fn push_u16(s: &mut String, n: u16) {
    push_u32(s, n as u32);
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

// Widget-local weekday/month name tables. The slint widget reads
// `datetime.weekday` and `datetime.month-name` strings pre-rendered by
// the host's chrono formatter; on wasm we lift them from `SystemTime`'s
// decomposed fields. Move to the SDK when a second widget needs them.

fn weekday_name(weekday: u8) -> &'static str {
    // SystemTime convention: 0 = Monday, 6 = Sunday.
    match weekday {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        _ => "Sun",
    }
}

fn month_name(month: u8) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}
