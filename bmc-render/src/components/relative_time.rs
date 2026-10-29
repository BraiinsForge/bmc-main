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

//! `RelativeTimeLive` — host-formatted, self-updating relative-time label.

use std::cell::RefCell;

use bmc_intl::{Direction, Intl, Language, TimeUnit, TimeWidth, format_message};
use bmc_ui::{RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments};

fn magnitude_and_future(delta_secs: i64, clamp: RelTimeClamp) -> (u64, bool) {
    match clamp {
        RelTimeClamp::Auto => (delta_secs.unsigned_abs(), delta_secs.is_negative()),
        RelTimeClamp::ElapsedOnly => (delta_secs.max(0).unsigned_abs(), false),
        RelTimeClamp::RemainingOnly => (delta_secs.saturating_neg().max(0).unsigned_abs(), true),
    }
}

const fn secs(unit: TimeUnit) -> u64 {
    match unit {
        TimeUnit::Second => 1,
        TimeUnit::Minute => 60,
        TimeUnit::Hour => 3_600,
        TimeUnit::Day => 86_400,
    }
}

/// A band whose unit has a smaller one to show beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pair {
    MinuteSecond,
    HourMinute,
    DayHour,
}

impl Pair {
    const fn larger(self) -> TimeUnit {
        match self {
            Self::MinuteSecond => TimeUnit::Minute,
            Self::HourMinute => TimeUnit::Hour,
            Self::DayHour => TimeUnit::Day,
        }
    }

    const fn smaller(self) -> TimeUnit {
        match self {
            Self::MinuteSecond => TimeUnit::Second,
            Self::HourMinute => TimeUnit::Minute,
            Self::DayHour => TimeUnit::Hour,
        }
    }
}

/// The band `mag` falls in: its pair, or `None` in the seconds band, which has no smaller unit.
fn band(mag: u64) -> Option<Pair> {
    if mag < secs(TimeUnit::Minute) {
        None
    } else if mag < secs(TimeUnit::Hour) {
        Some(Pair::MinuteSecond)
    } else if mag < secs(TimeUnit::Day) {
        Some(Pair::HourMinute)
    } else {
        Some(Pair::DayHour)
    }
}

fn largest(band: Option<Pair>) -> TimeUnit {
    band.map_or(TimeUnit::Second, Pair::larger)
}

/// How much of a label's units a [`RelTimeLength`] spells.
fn width(length: RelTimeLength) -> TimeWidth {
    match length {
        RelTimeLength::Short => TimeWidth::Narrow,
        RelTimeLength::Long => TimeWidth::Long,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Segments {
    Single(TimeUnit),
    /// The band's pair, with the count of its smaller unit while that is not zero;
    /// `None` in the seconds band.
    Double(Option<(Pair, Option<u64>)>),
}

impl Segments {
    fn largest(self) -> TimeUnit {
        match self {
            Self::Single(unit) => unit,
            Self::Double(band) => largest(band.map(|(pair, _)| pair)),
        }
    }
}

/// A span of the label: the count of its largest unit, and which units it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    count: u64,
    segments: Segments,
    width: TimeWidth,
}

/// What a label says, counted but not yet worded: integers only,
/// so telling whether it changed costs no formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelParts {
    Now(TimeWidth),
    Elapsed(Span),
    Ahead(Span),
}

/// Count `now - anchor` (seconds) per `format`: `segments` picks one unit (`7m`)
/// or two (`7m 30s`, the smaller dropped when zero). `clamp` pins direction.
#[must_use]
#[expect(
    clippy::integer_division,
    reason = "whole-unit truncation is intentional"
)]
pub fn rel_parts(delta_secs: i64, format: RelTimeFormat, clamp: RelTimeClamp) -> RelParts {
    let (mag, future) = magnitude_and_future(delta_secs, clamp);
    let width = width(format.length);
    if mag == 0 {
        return RelParts::Now(width);
    }
    let pair = band(mag);
    let unit = largest(pair);
    let segments = match format.segments {
        RelTimeSegments::Single => Segments::Single(unit),
        RelTimeSegments::Double => Segments::Double(pair.map(|pair| {
            let smaller = (mag % secs(unit)) / secs(pair.smaller());
            (pair, Some(smaller).filter(|&count| count > 0))
        })),
    };
    let span = Span {
        count: mag / secs(unit),
        segments,
        width,
    };
    if future {
        RelParts::Ahead(span)
    } else {
        RelParts::Elapsed(span)
    }
}

/// Word `parts` as `intl`'s language writes them.
///
/// A single segment takes CLDR's relative-time pattern, declined as the language needs.
/// CLDR has none for two: spelled out, each pair is a whole message its translator declines;
/// abbreviated, the counts go into a message, even while the smaller is hidden,
/// so the label keeps one form as it ticks.
#[must_use]
pub fn format_rel(intl: &Intl, parts: RelParts) -> String {
    let (span, direction) = match parts {
        RelParts::Now(width) => return intl.now(width),
        RelParts::Elapsed(span) => (span, Direction::Past),
        RelParts::Ahead(span) => (span, Direction::Future),
    };
    match (span.segments, span.width) {
        (Segments::Double(Some((pair, Some(smaller)))), TimeWidth::Long) => {
            spelled_out_pair(intl, (span.count, pair), smaller, direction)
        }
        (Segments::Double(band), TimeWidth::Narrow) => abbreviated(
            intl,
            (span.count, span.segments.largest()),
            band.and_then(|(pair, smaller)| smaller.map(|smaller| (smaller, pair.smaller()))),
            direction,
        ),
        (Segments::Single(_), _) | (Segments::Double(_), TimeWidth::Long) => {
            intl.relative_time(span.count, span.segments.largest(), span.width, direction)
        }
    }
}

fn abbreviated(
    intl: &Intl,
    (count, unit): (u64, TimeUnit),
    smaller: Option<(u64, TimeUnit)>,
    direction: Direction,
) -> String {
    let primary = intl.time_count(count, unit);
    let duration = match smaller {
        None => primary,
        Some((smaller, smaller_unit)) => {
            let secondary = intl.time_count(smaller, smaller_unit);
            format_message!(
                intl,
                default_message: "{primary} {secondary}",
                description: "Relative time, the two segments side by side: 7m 30s",
                values: { primary: primary, secondary: secondary },
            )
        }
    };
    match direction {
        Direction::Past => format_message!(
            intl,
            default_message: "{duration} ago",
            description: "Relative time label for a moment behind; duration is 7m 30s or 7m",
            values: { duration: duration },
        ),
        Direction::Future => format_message!(
            intl,
            default_message: "in {duration}",
            description: "Relative time label for a moment ahead; duration is 7m 30s or 7m",
            values: { duration: duration },
        ),
    }
}

fn spelled_out_pair(
    intl: &Intl,
    (count, pair): (u64, Pair),
    smaller: u64,
    direction: Direction,
) -> String {
    match (pair, direction) {
        (Pair::MinuteSecond, Direction::Past) => format_message!(
            intl,
            default_message: "{minutes, plural, one {{minutes} minute} other {{minutes} minutes}} {seconds, plural, one {{seconds} second} other {{seconds} seconds}} ago",
            description: "Relative time label for a moment behind, minutes and seconds spelled out; both are whole numbers",
            values: { minutes: count, seconds: smaller },
        ),
        (Pair::MinuteSecond, Direction::Future) => format_message!(
            intl,
            default_message: "in {minutes, plural, one {{minutes} minute} other {{minutes} minutes}} {seconds, plural, one {{seconds} second} other {{seconds} seconds}}",
            description: "Relative time label for a moment ahead, minutes and seconds spelled out; both are whole numbers",
            values: { minutes: count, seconds: smaller },
        ),
        (Pair::HourMinute, Direction::Past) => format_message!(
            intl,
            default_message: "{hours, plural, one {{hours} hour} other {{hours} hours}} {minutes, plural, one {{minutes} minute} other {{minutes} minutes}} ago",
            description: "Relative time label for a moment behind, hours and minutes spelled out; both are whole numbers",
            values: { hours: count, minutes: smaller },
        ),
        (Pair::HourMinute, Direction::Future) => format_message!(
            intl,
            default_message: "in {hours, plural, one {{hours} hour} other {{hours} hours}} {minutes, plural, one {{minutes} minute} other {{minutes} minutes}}",
            description: "Relative time label for a moment ahead, hours and minutes spelled out; both are whole numbers",
            values: { hours: count, minutes: smaller },
        ),
        (Pair::DayHour, Direction::Past) => format_message!(
            intl,
            default_message: "{days, plural, one {{days} day} other {{days} days}} {hours, plural, one {{hours} hour} other {{hours} hours}} ago",
            description: "Relative time label for a moment behind, days and hours spelled out; both are whole numbers",
            values: { days: count, hours: smaller },
        ),
        (Pair::DayHour, Direction::Future) => format_message!(
            intl,
            default_message: "in {days, plural, one {{days} day} other {{days} days}} {hours, plural, one {{hours} hour} other {{hours} hours}}",
            description: "Relative time label for a moment ahead, days and hours spelled out; both are whole numbers",
            values: { days: count, hours: smaller },
        ),
    }
}

/// The labels a tree worded this frame and the last, so a frame where no label ticked formats none.
/// Shared, not `&mut`, to ride beside the formatter through layout's recursion.
#[derive(Debug, Default)]
pub struct RelTimeWords {
    worded: RefCell<Worded>,
}

#[derive(Debug, Default)]
struct Worded {
    /// The formatter's language and translations: a new language or a pack (re)loaded words anew.
    by: Option<(Language, u64)>,
    this_frame: Vec<(RelParts, String)>,
    last_frame: Vec<(RelParts, String)>,
}

impl RelTimeWords {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            worded: RefCell::new(Worded {
                by: None,
                this_frame: Vec::new(),
                last_frame: Vec::new(),
            }),
        }
    }

    /// Start a frame: a label the last frame did not show is forgotten.
    pub(crate) fn begin_frame(&self) {
        let worded = &mut *self.worded.borrow_mut();
        std::mem::swap(&mut worded.this_frame, &mut worded.last_frame);
        worded.this_frame.clear();
    }

    /// [`format_rel`], from the labels already worded where `parts` is among them.
    #[must_use]
    pub(crate) fn word(&self, intl: &Intl, parts: RelParts) -> String {
        let worded = &mut *self.worded.borrow_mut();
        let by = Some(intl.wording());
        if worded.by != by {
            worded.by = by;
            worded.this_frame.clear();
            worded.last_frame.clear();
        }
        if let Some((_, text)) = worded.this_frame.iter().find(|(seen, _)| *seen == parts) {
            return text.clone();
        }
        let text = match worded
            .last_frame
            .iter()
            .position(|(seen, _)| *seen == parts)
        {
            Some(at) => worded.last_frame.swap_remove(at).1,
            None => format_rel(intl, parts),
        };
        worded.this_frame.push((parts, text.clone()));
        text
    }
}

/// Milliseconds until the label next changes, at least 1 s,
/// aligned to the smallest shown unit's boundary so the host wakes only when the text ticks.
/// `Single` tracks the largest unit (a minute in the minutes band);
/// `Double` tracks the smaller one (seconds), so it wakes far more often.
/// `None` only when the label is pinned at "now" forever (`RemainingOnly` past its anchor);
/// an `ElapsedOnly` label before its anchor wakes once at the un-pin, then ticks normally.
#[must_use]
pub fn next_change_delay_ms(
    delta_secs: i64,
    format: RelTimeFormat,
    clamp: RelTimeClamp,
) -> Option<u32> {
    // Past anchor: remaining stays 0 → pinned at "now" forever, no self-tick.
    if matches!(clamp, RelTimeClamp::RemainingOnly) && delta_secs > 0 {
        return None;
    }
    // Future anchor: pinned at "now" until the clock reaches it, then it un-pins
    // to "1s ago". Wake once at that instant, not every second.
    if matches!(clamp, RelTimeClamp::ElapsedOnly) && delta_secs < 0 {
        let un_pin_secs = 1_i64.saturating_sub(delta_secs);
        return Some(u32::try_from(un_pin_secs.saturating_mul(1_000)).unwrap_or(u32::MAX));
    }
    let (mag, future) = magnitude_and_future(delta_secs, clamp);
    let pair = band(mag);
    let tick = match format.segments {
        RelTimeSegments::Single => secs(largest(pair)),
        RelTimeSegments::Double => secs(pair.map_or(TimeUnit::Second, Pair::smaller)),
    };
    let delay_secs = if future {
        (mag % tick) + 1
    } else {
        tick - (mag % tick)
    };
    Some(u32::try_from(delay_secs.saturating_mul(1_000)).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::{next_change_delay_ms, rel_parts};
    use bmc_ui::{RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments};

    /// The label as English words it.
    fn format_rel(delta_secs: i64, format: RelTimeFormat, clamp: RelTimeClamp) -> String {
        let english = bmc_intl::formatter(bmc_intl::Language::default());
        super::format_rel(&english, rel_parts(delta_secs, format, clamp))
    }

    const fn fmt(length: RelTimeLength, segments: RelTimeSegments) -> RelTimeFormat {
        RelTimeFormat { length, segments }
    }
    const SHORT_SINGLE: RelTimeFormat = fmt(RelTimeLength::Short, RelTimeSegments::Single);
    const SHORT_DOUBLE: RelTimeFormat = fmt(RelTimeLength::Short, RelTimeSegments::Double);
    const LONG_SINGLE: RelTimeFormat = fmt(RelTimeLength::Long, RelTimeSegments::Single);
    const LONG_DOUBLE: RelTimeFormat = fmt(RelTimeLength::Long, RelTimeSegments::Double);
    const AUTO: RelTimeClamp = RelTimeClamp::Auto;

    #[test]
    fn a_label_is_worded_anew_once_its_languages_pack_loads() {
        let zh = bmc_intl::Language::parse("zh").expect("BUG: `zh` is a language");
        let profile = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/intl/profile-languages");
        let without_pack = bmc_intl::Translations::load(&[]).0.formatter(zh);
        let with_pack = bmc_intl::Translations::load(&[profile]).0.formatter(zh);
        let words = super::RelTimeWords::new();
        let parts = rel_parts(5, SHORT_SINGLE, AUTO);

        let worded = [&without_pack, &with_pack].map(|intl| words.word(intl, parts));

        assert_eq!(
            worded,
            ["5s ago", "5秒前"],
            "the same language, now speaking its pack's words, must not reuse the English label"
        );
    }

    #[test]
    fn a_label_is_worded_anew_when_the_language_changes() {
        let zh = bmc_intl::Language::parse("zh").expect("BUG: `zh` is a language");
        let profile = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/intl/profile-languages");
        let translations = bmc_intl::Translations::load(&[profile]).0;
        let [english, chinese] =
            [bmc_intl::Language::default(), zh].map(|language| translations.formatter(language));
        let words = super::RelTimeWords::new();
        let parts = rel_parts(5, SHORT_SINGLE, AUTO);

        let worded = [&english, &chinese].map(|intl| words.word(intl, parts));

        assert_eq!(
            worded,
            ["5s ago", "5秒前"],
            "a label worded in English must not be reused once the device speaks Chinese"
        );
    }

    #[test]
    fn zero_is_now() {
        assert_eq!(format_rel(0, SHORT_SINGLE, AUTO), "now");
    }

    #[test]
    fn seconds_band_has_no_smaller_unit() {
        assert_eq!(format_rel(5, SHORT_DOUBLE, AUTO), "5s ago");
        assert_eq!(format_rel(-5, SHORT_DOUBLE, AUTO), "in 5s");
        assert_eq!(format_rel(59, SHORT_DOUBLE, AUTO), "59s ago");
        assert_eq!(format_rel(5, LONG_DOUBLE, AUTO), "5 seconds ago");
        assert_eq!(format_rel(-5, LONG_DOUBLE, AUTO), "in 5 seconds");
    }

    #[test]
    fn short_double_shows_two_segments() {
        assert_eq!(format_rel(90, SHORT_DOUBLE, AUTO), "1m 30s ago");
        assert_eq!(format_rel(3_660, SHORT_DOUBLE, AUTO), "1h 1m ago");
        assert_eq!(format_rel(200_000, SHORT_DOUBLE, AUTO), "2d 7h ago");
        assert_eq!(format_rel(-90, SHORT_DOUBLE, AUTO), "in 1m 30s");
    }

    #[test]
    fn double_drops_the_smaller_segment_when_zero() {
        assert_eq!(format_rel(120, SHORT_DOUBLE, AUTO), "2m ago");
        assert_eq!(format_rel(3_600, SHORT_DOUBLE, AUTO), "1h ago");
        assert_eq!(format_rel(86_400, SHORT_DOUBLE, AUTO), "1d ago");
    }

    #[test]
    fn single_shows_only_the_largest_unit() {
        assert_eq!(format_rel(90, SHORT_SINGLE, AUTO), "1m ago");
        assert_eq!(format_rel(3_660, SHORT_SINGLE, AUTO), "1h ago");
        assert_eq!(format_rel(200_000, SHORT_SINGLE, AUTO), "2d ago");
        assert_eq!(format_rel(-90, SHORT_SINGLE, AUTO), "in 1m");
    }

    #[test]
    fn long_spells_the_units_out_by_plural() {
        assert_eq!(format_rel(1, LONG_SINGLE, AUTO), "1 second ago");
        assert_eq!(format_rel(5, LONG_SINGLE, AUTO), "5 seconds ago");
        assert_eq!(format_rel(60, LONG_SINGLE, AUTO), "1 minute ago");
        assert_eq!(format_rel(120, LONG_SINGLE, AUTO), "2 minutes ago");
        assert_eq!(format_rel(200_000, LONG_SINGLE, AUTO), "2 days ago");
        assert_eq!(
            format_rel(86_400, LONG_SINGLE, AUTO),
            "1 day ago",
            "a day counted, not CLDR's calendar `yesterday`"
        );
        assert_eq!(format_rel(-90, LONG_SINGLE, AUTO), "in 1 minute");
    }

    #[test]
    fn long_double_spells_both_segments_out_by_plural() {
        assert_eq!(format_rel(90, LONG_DOUBLE, AUTO), "1 minute 30 seconds ago");
        assert_eq!(format_rel(3_660, LONG_DOUBLE, AUTO), "1 hour 1 minute ago");
        assert_eq!(
            format_rel(7_380, LONG_DOUBLE, AUTO),
            "2 hours 3 minutes ago"
        );
        assert_eq!(format_rel(90_000, LONG_DOUBLE, AUTO), "1 day 1 hour ago");
        assert_eq!(format_rel(200_000, LONG_DOUBLE, AUTO), "2 days 7 hours ago");
    }

    #[test]
    fn long_double_ahead_spells_both_segments_out() {
        assert_eq!(format_rel(-90, LONG_DOUBLE, AUTO), "in 1 minute 30 seconds");
        assert_eq!(format_rel(-3_660, LONG_DOUBLE, AUTO), "in 1 hour 1 minute");
        assert_eq!(format_rel(-90_000, LONG_DOUBLE, AUTO), "in 1 day 1 hour");
    }

    #[test]
    fn long_double_reads_as_single_while_the_smaller_segment_is_zero() {
        assert_eq!(
            [60, 61].map(|delta| format_rel(delta, LONG_DOUBLE, AUTO)),
            ["1 minute ago", "1 minute 1 second ago"],
            "spelled out and declined either way, never a pair with a zero in it"
        );
    }

    #[test]
    fn every_form_writes_its_count_ungrouped_as_message_values_are() {
        let days_1200 = 1_200 * 86_400;
        assert_eq!(
            [SHORT_SINGLE, SHORT_DOUBLE, LONG_SINGLE, LONG_DOUBLE].map(|format| format_rel(
                days_1200 + 3_600,
                format,
                AUTO
            )),
            [
                "1200d ago",
                "1200d 1h ago",
                "1200 days ago",
                "1200 days 1 hour ago"
            ]
        );
    }

    #[test]
    fn clamp_pins_direction() {
        assert_eq!(
            format_rel(-5, SHORT_SINGLE, RelTimeClamp::ElapsedOnly),
            "now"
        );
        assert_eq!(
            format_rel(5, SHORT_SINGLE, RelTimeClamp::RemainingOnly),
            "now"
        );
        assert_eq!(
            format_rel(-5, SHORT_SINGLE, RelTimeClamp::RemainingOnly),
            "in 5s"
        );
    }

    #[test]
    fn double_delay_aligns_to_the_smaller_segment() {
        // seconds band and minutes-band (smaller unit = seconds) both tick each second.
        assert_eq!(next_change_delay_ms(5, SHORT_DOUBLE, AUTO), Some(1_000));
        assert_eq!(next_change_delay_ms(90, SHORT_DOUBLE, AUTO), Some(1_000));
        // hours band → smaller minutes; days band → smaller hours.
        assert_eq!(
            next_change_delay_ms(3_600, SHORT_DOUBLE, AUTO),
            Some(60_000)
        );
        assert_eq!(
            next_change_delay_ms(200_000, SHORT_DOUBLE, AUTO),
            Some(1_600_000)
        );
    }

    #[test]
    fn single_delay_aligns_to_the_largest_unit() {
        // Cadence follows segments, not length — LONG_SINGLE would match.
        assert_eq!(next_change_delay_ms(5, SHORT_SINGLE, AUTO), Some(1_000)); // seconds band → 1s
        assert_eq!(next_change_delay_ms(90, SHORT_SINGLE, AUTO), Some(30_000)); // 1m30s → next minute
        assert_eq!(
            next_change_delay_ms(3_600, SHORT_SINGLE, AUTO),
            Some(3_600_000)
        ); // → next hour
        assert_eq!(
            next_change_delay_ms(200_000, SHORT_SINGLE, AUTO),
            Some(59_200_000)
        ); // → next day
    }

    #[test]
    fn delay_counts_down_toward_the_boundary() {
        assert_eq!(next_change_delay_ms(-5, SHORT_DOUBLE, AUTO), Some(1_000));
        // "in 1h" one second past the hour edge → 2 s until it drops to "59m 59s".
        assert_eq!(
            next_change_delay_ms(-3_601, SHORT_DOUBLE, AUTO),
            Some(2_000)
        );
    }

    #[test]
    fn pinned_now_label_ticks_per_clamp() {
        // RemainingOnly past its anchor is pinned at "now" forever → no wake.
        assert_eq!(
            next_change_delay_ms(5, SHORT_SINGLE, RelTimeClamp::RemainingOnly),
            None
        );
        // ElapsedOnly before its anchor un-pins at the anchor: one wake at the
        // un-pin instant (delta == 1 → "1s ago"), here 6 s out.
        assert_eq!(
            next_change_delay_ms(-5, SHORT_SINGLE, RelTimeClamp::ElapsedOnly),
            Some(6_000)
        );
        // The un-pinned direction ticks normally.
        assert_eq!(
            next_change_delay_ms(5, SHORT_SINGLE, RelTimeClamp::ElapsedOnly),
            Some(1_000)
        );
    }

    #[test]
    fn deserializes_from_wire() {
        use bmc_ui::{NODE_RELTIME, TextStyle};

        use crate::tree::{Node, deserialize_tree};

        let mut bytes = vec![NODE_RELTIME];
        bytes.extend_from_slice(&1_700_000_000_i64.to_le_bytes());
        bytes.push(u8::from(SHORT_SINGLE));
        bytes.push(u8::from(RelTimeClamp::ElapsedOnly));
        bytes.extend_from_slice(&TextStyle::default().to_bytes());

        let node = deserialize_tree(&bytes).expect("BUG: RelTime should deserialize");
        assert!(matches!(
            node,
            Node::RelTime {
                anchor: 1_700_000_000,
                format: RelTimeFormat {
                    length: RelTimeLength::Short,
                    segments: RelTimeSegments::Single,
                },
                clamp: RelTimeClamp::ElapsedOnly,
                ..
            }
        ));
    }
}
