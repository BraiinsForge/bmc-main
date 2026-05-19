// Copyright (C) 2026  Braiins Systems s.r.o.

//! Clock widget — three render modes (analog round / analog rectangular / digital)
//! and four sizes (Small / Medium / Large / Full).

mod manifest_params;

use bmc_wasm_sdk::host::{SystemTime, request_frame, request_frame_after};
use bmc_wasm_sdk::system::TimeFormat;
use bmc_wasm_sdk::{
    FontWeight, FormatTimeOpts, GRAY_60, Node, Tz, WHITE, WidgetSize, center, col, format_time,
    local_unix_secs, props, render_ui, resolve_tz_offset, row, spacer, strftime, style, system,
    text, widget_size,
};

use manifest_params::{NumbersFontStyle, Params};

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
    let size = pick_size(w, h);
    let effective_tz = params.timezone_override.as_deref().map(Tz::from_runtime);

    let root = render_digital(now, &params, size, effective_tz.as_ref());

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
    // SDK gap: slint colorizes the alarm-bell glyph (Palette.gray-40);
    // until the SDK exposes a tint primitive, render the formatted time
    // alone and revisit the bell icon when image colorize ships
    // (see CLOCK-WIDGET-WASM-PORT-PLAN.md, SDK gaps #1).
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
