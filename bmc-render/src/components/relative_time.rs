// Copyright (C) 2026  Braiins Systems s.r.o.

//! `RelativeTimeLive` — host-formatted, self-updating relative-time label.

use bmc_wasm_protocol::{RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments};

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

fn magnitude_and_future(delta_secs: i64, clamp: RelTimeClamp) -> (u64, bool) {
    match clamp {
        RelTimeClamp::Auto => (delta_secs.unsigned_abs(), delta_secs.is_negative()),
        RelTimeClamp::ElapsedOnly => (delta_secs.max(0).unsigned_abs(), false),
        RelTimeClamp::RemainingOnly => (delta_secs.saturating_neg().max(0).unsigned_abs(), true),
    }
}

/// A unit shown in a label: size in seconds, abbreviation, and full (singular) word.
type Unit = (u64, &'static str, &'static str);

/// The largest unit for `mag` and the next-smaller one shown alongside it (the
/// seconds band has no secondary), e.g. days band → `d` primary + `h` secondary.
fn bands(mag: u64) -> (Unit, Option<Unit>) {
    if mag < MINUTE {
        ((1, "s", "second"), None)
    } else if mag < HOUR {
        ((MINUTE, "m", "minute"), Some((1, "s", "second")))
    } else if mag < DAY {
        ((HOUR, "h", "hour"), Some((MINUTE, "m", "minute")))
    } else {
        ((DAY, "d", "day"), Some((HOUR, "h", "hour")))
    }
}

/// Render one segment, e.g. `7m` / `7 minutes` / `1 minute`.
fn segment(count: u64, unit: Unit, length: RelTimeLength) -> String {
    let (_, abbrev, long) = unit;
    match length {
        RelTimeLength::Short => format!("{count}{abbrev}"),
        RelTimeLength::Long if count == 1 => format!("{count} {long}"),
        RelTimeLength::Long => format!("{count} {long}s"),
    }
}

/// Format `now - anchor` (seconds) per `format`: `length` picks abbreviation vs
/// full words, `segments` picks one unit (`7m`) or two (`7m 30s`, smaller dropped
/// when zero). `clamp` pins direction.
#[must_use]
#[expect(
    clippy::integer_division,
    reason = "whole-unit truncation is intentional"
)]
pub fn format_rel(delta_secs: i64, format: RelTimeFormat, clamp: RelTimeClamp) -> String {
    let (mag, future) = magnitude_and_future(delta_secs, clamp);
    if mag == 0 {
        return "now".to_owned();
    }
    let (primary, secondary) = bands(mag);
    let (p_secs, ..) = primary;
    let mut core = segment(mag / p_secs, primary, format.length);
    if let (RelTimeSegments::Double, Some(unit)) = (format.segments, secondary) {
        let (s_secs, ..) = unit;
        let s = (mag % p_secs) / s_secs;
        if s > 0 {
            core.push(' ');
            core.push_str(&segment(s, unit, format.length));
        }
    }
    if future {
        format!("in {core}")
    } else {
        format!("{core} ago")
    }
}

/// Milliseconds until the label next changes — aligned to the smallest shown
/// unit's boundary, so the host wakes only when the text ticks. `Single` tracks
/// the primary (a minute in the minutes band); `Double` tracks the secondary
/// (seconds), so it wakes far more often. ≥ 1 s. `None` only when the label is
/// pinned at "now" forever (`RemainingOnly` past its anchor); an `ElapsedOnly`
/// label before its anchor wakes once at the un-pin, then ticks normally.
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
    let ((p_secs, ..), secondary) = bands(mag);
    let tick = match format.segments {
        RelTimeSegments::Single => p_secs,
        RelTimeSegments::Double => secondary.map_or(p_secs, |(s, ..)| s),
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
    use super::{format_rel, next_change_delay_ms};
    use bmc_wasm_protocol::{RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments};

    const fn fmt(length: RelTimeLength, segments: RelTimeSegments) -> RelTimeFormat {
        RelTimeFormat { length, segments }
    }
    const SHORT_SINGLE: RelTimeFormat = fmt(RelTimeLength::Short, RelTimeSegments::Single);
    const SHORT_DOUBLE: RelTimeFormat = fmt(RelTimeLength::Short, RelTimeSegments::Double);
    const LONG_SINGLE: RelTimeFormat = fmt(RelTimeLength::Long, RelTimeSegments::Single);
    const LONG_DOUBLE: RelTimeFormat = fmt(RelTimeLength::Long, RelTimeSegments::Double);
    const AUTO: RelTimeClamp = RelTimeClamp::Auto;

    #[test]
    fn zero_is_now() {
        assert_eq!(format_rel(0, SHORT_SINGLE, AUTO), "now");
    }

    #[test]
    fn seconds_band_has_no_secondary() {
        assert_eq!(format_rel(5, SHORT_DOUBLE, AUTO), "5s ago");
        assert_eq!(format_rel(-5, SHORT_DOUBLE, AUTO), "in 5s");
        assert_eq!(format_rel(59, SHORT_DOUBLE, AUTO), "59s ago");
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
    fn long_spells_out_and_pluralizes() {
        assert_eq!(format_rel(1, LONG_SINGLE, AUTO), "1 second ago");
        assert_eq!(format_rel(5, LONG_SINGLE, AUTO), "5 seconds ago");
        assert_eq!(format_rel(60, LONG_SINGLE, AUTO), "1 minute ago");
        assert_eq!(format_rel(120, LONG_SINGLE, AUTO), "2 minutes ago");
        assert_eq!(format_rel(200_000, LONG_SINGLE, AUTO), "2 days ago");
        assert_eq!(format_rel(-90, LONG_SINGLE, AUTO), "in 1 minute");
    }

    #[test]
    fn long_double_spells_out_both_segments() {
        assert_eq!(format_rel(90, LONG_DOUBLE, AUTO), "1 minute 30 seconds ago");
        assert_eq!(format_rel(3_660, LONG_DOUBLE, AUTO), "1 hour 1 minute ago");
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
        // seconds band and minutes-band (secondary = seconds) both tick each second.
        assert_eq!(next_change_delay_ms(5, SHORT_DOUBLE, AUTO), Some(1_000));
        assert_eq!(next_change_delay_ms(90, SHORT_DOUBLE, AUTO), Some(1_000));
        // hours band → secondary minutes; days band → secondary hours.
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
        use bmc_wasm_protocol::{NODE_RELTIME, TextStyle};

        use crate::tree::{TreeNode, deserialize_tree};

        let mut bytes = vec![NODE_RELTIME];
        bytes.extend_from_slice(&1_700_000_000_i64.to_le_bytes());
        bytes.push(u8::from(SHORT_SINGLE));
        bytes.push(u8::from(RelTimeClamp::ElapsedOnly));
        bytes.extend_from_slice(&TextStyle::default().to_bytes());

        let node = deserialize_tree(&bytes).expect("BUG: RelTime should deserialize");
        assert!(matches!(
            node,
            TreeNode::RelTime {
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
