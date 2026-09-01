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

//! Shared infrastructure used by all three render modes
//! (digital, analog-round, analog-rect): palette, tz helpers,
//! the alarm-row drawer, numeric utilities,
//! and the typography-knob mapping.

pub(crate) use bmc_wasm_sdk::format::{TzLabel, push_utc_offset, resolve_tz_for_label};
use bmc_wasm_sdk::system::{DateFormat, TimeFormat};
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::NumbersFontStyle;

// ── Palette ────────────────────────────────────────────────────────────
// One set of colours per render mode (day / night).

#[derive(Clone, Copy)]
pub(crate) struct ClockPalette {
    /// Most prominent foreground: digital time text, hour
    /// and minute hands. White in day, red in night.
    pub(crate) primary: Color,
    /// Second-hand accent (orange in day, red in night).
    pub(crate) second_hand: Color,
    /// Subdued text: header date/timezone, AM/PM label.
    pub(crate) text: Color,
    /// Date-window day-number text and 1px border ring.
    pub(crate) date_window: Color,
    /// Alarm-bell glyph + adjacent alarm time text.
    pub(crate) alarm_bell: Color,
    /// Dial small ticks (between the big numeral positions).
    pub(crate) tick_small: Color,
    /// Dial big ticks at 12 / 3 / 6 / 9 positions.
    pub(crate) tick_large: Color,
    /// Dial outer rim ring.
    pub(crate) dial_rim: Color,
    /// Centre-circle stack — bigger white disc behind the hands.
    pub(crate) centre_white: Color,
    /// Centre-circle stack — orange pivot dot under the second hand.
    pub(crate) centre_orange: Color,
    /// Centre-circle stack — outer stroke ring.
    pub(crate) centre_stroke: Color,
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

pub(crate) fn clock_palette(night_mode: bool) -> ClockPalette {
    if night_mode {
        PALETTE_NIGHT
    } else {
        PALETTE_DAY
    }
}

// ── Timezone helpers ───────────────────────────────────────────────────

/// Resolve `tz` → system tz → Etc/GMT (UTC) into a concrete `Tz`.
/// Projection paths (hands, digits) use this;
/// the timezone-line code instead consults the raw Option chain
/// so it can render a visible signal when nothing was configured.
#[expect(
    dead_code,
    reason = "retained for future callers; renderers now use offset_secs"
)]
pub(crate) fn effective_tz(tz: Option<&Tz>) -> Tz {
    if let Some(t) = tz {
        return t.clone();
    }
    if let Some(name) = system::current().timezone() {
        return Tz::from_runtime(name);
    }
    Tz::from_runtime("Etc/GMT")
}

/// Apply a precomputed UTC offset to `now.unix_secs` and decompose
/// to wall-clock fields. The caller is responsible for resolving
/// the offset once per `render()` (see `resolve_tz_for_label`).
pub(crate) fn local_or_system(now: &SystemTime, offset_secs: i32) -> LocalDateTime {
    SystemTime {
        unix_secs: now.unix_secs + i64::from(offset_secs),
    }
    .utc()
}

/// Day/month order of a reading date row: "Mon 3 March" vs "Mon, March 3".
/// The system `DateFormat` is purely numeric, so only its ordering matters here;
/// year-first variants collapse to day-first
/// because "2026 Mon 3 March" reads as a log line, not a clock face.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DateOrder {
    DayFirst,
    MonthFirst,
}

pub(crate) fn date_order(format: DateFormat) -> DateOrder {
    match format {
        DateFormat::MDYyyySlash => DateOrder::MonthFirst,
        DateFormat::DdMmYyyyDot
        | DateFormat::DdMmYyyySlash
        | DateFormat::DMYyyySlash
        | DateFormat::DdMmYyyyDash
        | DateFormat::YyyyMDSlash
        | DateFormat::YyyyMmDdDot
        | DateFormat::YyyyMmDdDash => DateOrder::DayFirst,
    }
}

// ── Numeric helpers ────────────────────────────────────────────────────

pub(crate) fn f32_from_u32(value: u32) -> f32 {
    f32::from(u16::try_from(value).expect("BUG: widget dimensions and font sizes fit in u16"))
}

pub(crate) fn f32_from_usize(value: usize) -> f32 {
    f32::from(u16::try_from(value).expect("BUG: short widget labels fit in u16"))
}

// ── Typography ─────────────────────────────────────────────────────────

pub(crate) fn font_weight(style: NumbersFontStyle) -> FontWeight {
    match style {
        NumbersFontStyle::Regular => FontWeight::REGULAR,
        NumbersFontStyle::SemiBold => FontWeight::SEMIBOLD,
        NumbersFontStyle::Bold => FontWeight::BOLD,
    }
}

pub(crate) const fn time_font_family() -> FontFamily {
    FontFamily::DeckSans
}

pub(crate) const fn alarm_row_font_family() -> FontFamily {
    FontFamily::DeckSans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_text_uses_deck_sans_family() {
        assert_eq!(time_font_family(), FontFamily::DeckSans);
    }

    #[test]
    fn alarm_row_text_uses_deck_sans_family() {
        assert_eq!(alarm_row_font_family(), FontFamily::DeckSans);
    }
}

// ── Alarm row — single renderer shared by digital + both analog modes ──

const ALARM_BELL: Svg = include_svg!("assets/alarm-bell.svg");

/// Where the bell+time group sits on the row.
#[derive(Copy, Clone)]
pub(crate) enum AlarmAnchor {
    /// Bell's left edge sits at x.
    LeftX(f32),
    /// Alarm-time text's right edge sits at x.
    RightX(f32),
}

/// Emit the alarm-bell + alarm-time pair at the given vertical
/// centre `y`, anchored per `AlarmAnchor`.
///
/// Bell and text use a single `bell_size` so they share
/// the same line-box (font size = `bell_size as u32`).
///
/// Returns `Some(width)` when an alarm was emitted; `None` when no alarm is set.
pub(crate) fn alarm_row_draws(
    anchor: AlarmAnchor,
    y: f32,
    bell_size: f32,
    font_weight: FontWeight,
    color: Color,
    draws: &mut Vec<Draw>,
) -> Option<f32> {
    let snap = system::current();
    let alarm = snap.next_alarm()?;
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
    // Optical compensation: the bell's visual mass sits below its viewBox
    // centre (cy=13/24) and digit glyphs sit above the font's EM-box
    // centre (where `VerticalAlign::Center` anchors), so geometric
    // centring leaves the bell visibly lower than the digits.
    let optical_offset = bell_size * 0.08;
    draws.push(
        Draw::svg(
            x,
            y - bell_size / 2.0 - optical_offset,
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
            family: alarm_row_font_family(),
            align: TextAlign::Left,
            valign: VerticalAlign::Center,
        ),
    ));
    Some(total_w)
}

pub(crate) fn format_alarm_time(
    alarm: bmc_wasm_sdk::system::NextAlarmView<'_>,
    is_12h: bool,
) -> String {
    let secs = alarm.fire_at_utc_ms / 1000;
    // Alarms are a system-wide construct (the deck wakes at the operator-set time);
    // the per-widget `timezone_override` does not shift the alarm row.
    // Shift the UTC timestamp by the system tz offset before strftime
    // — `host_format_date` formats through DateTime::<Utc>,
    // so without the shift an alarm set for 06:00 CEST would render as 04:00.
    let offset_secs = system::current()
        .timezone()
        .and_then(|name| format::resolve_tz_offset(&Tz::from_runtime(name), secs))
        .unwrap_or(0);
    let shifted = secs + i64::from(offset_secs);
    let pattern = if is_12h { "%I:%M %p" } else { "%H:%M" };
    strftime(shifted, pattern)
}
