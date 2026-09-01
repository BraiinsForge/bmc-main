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
    AlarmAnchor, ClockPalette, DateOrder, TzLabel, alarm_row_draws, date_order, font_weight,
    local_or_system, push_utc_offset, resolve_tz_for_label, time_font_family,
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
    /// Gap between the date and timezone spans in the header row.
    /// Small uses a tight value so a long city name (e.g. "Bahia Banderas")
    /// fits inside the 317px viewport without spilling.
    header_gap: f32,
    /// Maximum lines the flex-wrapped header row may grow to.
    /// Small reserves 2 so a long "City (UTC±HH:MM)" tz can drop below
    /// the date instead of overflowing into the time row.
    header_max_lines: u8,
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
    header_gap: 80.0,
    header_max_lines: 1,
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
    header_gap: 32.0,
    header_max_lines: 1,
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
    header_gap: 80.0,
    header_max_lines: 1,
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
    header_gap: 8.0,
    header_max_lines: 2,
    show_year: false,
    show_weekday: false,
    show_alarm: false,
    ampm_inline: false,
    show_utc_offset: true,
};

/// Resolve the SDK-classified `SizeVariant` to the matching per-size `const`.
fn pick_size(variant: SizeVariant) -> &'static DigitalSizeParams {
    match variant {
        SizeVariant::Full => &DIGITAL_FULL,
        SizeVariant::Large => &DIGITAL_LARGE,
        SizeVariant::Medium => &DIGITAL_MEDIUM,
        SizeVariant::Small => &DIGITAL_SMALL,
    }
}

impl DigitalSizeParams {
    /// Shrink the font sizes and paddings by `fit` so an off-canonical viewport
    /// — e.g. BMM101's 480×320 classified as Large but narrower than Large's
    /// canonical 638 — scales down instead of overflowing. Visibility flags and
    /// the reserved header line count are layout structure, not metrics, and
    /// pass through unchanged.
    fn scaled(self, fit: f32) -> Self {
        Self {
            time_font_size: scale_font(self.time_font_size, fit),
            header_font_size: scale_font(self.header_font_size, fit),
            ampm_font_size: scale_font(self.ampm_font_size, fit),
            top_padding: self.top_padding * fit,
            bottom_padding: self.bottom_padding * fit,
            header_gap: self.header_gap * fit,
            ..self
        }
    }
}

/// Scale a font size by `fit`, never below 1px. `fit` is in `(0, 1]`, so the
/// product stays within the original `u16`.
fn scale_font(value: u16, fit: f32) -> u16 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "fit is in (0, 1], so the scaled size is a small non-negative value"
    )]
    let scaled = (f32::from(value) * fit).round() as u16;
    scaled.max(1)
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
    let size = pick_size(variant).scaled(ws.fit());
    let size = &size;
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
    // Footer is always one line: ampm-inline and show-alarm are
    // mutually exclusive per size (Full has alarm + inline ampm;
    // the rest have standalone ampm and no alarm).
    //
    // Header reserves `header_max_lines` so a flex-wrapped tz row
    // (Small only, today) has room without spilling into the time row.
    let line_h = f32::from(size.header_font_size) * 1.2;
    let header_lines = f32::from(size.header_max_lines);
    let header_slot_h = line_h * header_lines + size.header_gap * (header_lines - 1.0).max(0.0);
    let header_slot = col(
        props!(height: header_slot_h),
        header_node.into_iter().collect::<Vec<_>>(),
    );
    let footer_slot = col(
        props!(height: line_h),
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
    let date_str: Option<String> = params
        .show_date
        .then(|| compose_date(now, size, offset_secs));
    let tz = params
        .show_timezone
        .then(|| compose_timezone(label, size, palette));
    let date_style = style!(
        size: u32::from(size.header_font_size),
        weight: FontWeight::REGULAR,
        color: palette.text,
    );
    let mut row_children: Vec<Node> = Vec::with_capacity(2);
    if let Some(s) = date_str {
        row_children.push(text(s, date_style));
    }
    if let Some((s, color)) = tz {
        let tz_style = style!(
            size: u32::from(size.header_font_size),
            weight: FontWeight::REGULAR,
            color: color,
        );
        row_children.push(text(s, tz_style));
    }
    Some(center(
        props!(),
        [row(props!(gap: size.header_gap, wrap: true), row_children)],
    ))
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
fn date_pattern(format: system::DateFormat, show_weekday: bool, show_year: bool) -> &'static str {
    use DateOrder::{DayFirst, MonthFirst};
    match (date_order(format), show_weekday, show_year) {
        (MonthFirst, true, false) => "%a, %B %-d",
        (MonthFirst, true, true) => "%a, %B %-d, %Y",
        (MonthFirst, false, false) => "%b %-d",
        (MonthFirst, false, true) => "%B %-d, %Y",
        (DayFirst, true, false) => "%a %-d %B",
        (DayFirst, true, true) => "%a %-d %B %Y",
        (DayFirst, false, false) => "%-d %b",
        (DayFirst, false, true) => "%-d %B %Y",
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
            color: palette.primary,
            family: time_font_family(),
        ),
    );

    if size.ampm_inline && is_12h {
        let ampm = ampm_glyph(now, offset_secs);
        // Keep the time text at the parent's horizontal centre when AM/PM
        // appears: reserve a fixed slot for AM/PM on the right and mirror
        // it with an empty slot of the same width on the left. The symmetric
        // row then centres the time at the geometric middle, so switching
        // 24h ↔ 12h doesn't shift the digits.
        let ampm_slot_w = f32::from(size.ampm_font_size) * 1.5;
        let ampm_text = text(
            ampm,
            style!(
                size: u32::from(size.ampm_font_size),
                weight: FontWeight::REGULAR,
                color: palette.text,
            ),
        );
        let left_reserve = col(props!(width: ampm_slot_w), Vec::<Node>::new());
        // Lift AM/PM by half its own height: cross_align: Center aligns the
        // text-box top to the row mid-line, but we want AM/PM's centre there.
        // A bottom filler of `ampm_font_size` makes the slot taller by that
        // amount, and centering shifts the (top-anchored) text up by half it.
        let ampm_lift = f32::from(size.ampm_font_size);
        let right_slot = col(
            props!(width: ampm_slot_w, cross_align: CrossAlign::Start),
            [ampm_text, spacer_px(ampm_lift)],
        );
        center(
            props!(),
            [row(
                props!(gap: 32.0, cross_align: CrossAlign::Center),
                [left_reserve, time_node, right_slot],
            )],
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
    let bell = f32::from(size.header_font_size);
    let mut draws: Vec<Draw> = Vec::with_capacity(2);
    let total_w = alarm_row_draws(
        AlarmAnchor::LeftX(0.0),
        bell / 2.0,
        bell,
        font_weight(params.numbers_font_style),
        palette.alarm_bell,
        &mut draws,
    )?;
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
