// Copyright (C) 2026  Braiins Systems s.r.o.

//! `RelativeTimeLive` — host-formatted, self-updating relative-time label.

use bmc_wasm_protocol::{RelTimeClamp, RelTimeFormat};

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

/// Primary unit `(secs, label)` and the next-smaller unit shown alongside it,
/// e.g. days band → `d` primary + `h` secondary. Seconds band has no secondary.
fn bands(mag: u64) -> ((u64, &'static str), Option<(u64, &'static str)>) {
    if mag < MINUTE {
        ((1, "s"), None)
    } else if mag < HOUR {
        ((MINUTE, "m"), Some((1, "s")))
    } else if mag < DAY {
        ((HOUR, "h"), Some((MINUTE, "m")))
    } else {
        ((DAY, "d"), Some((HOUR, "h")))
    }
}

/// Format `now - anchor` (seconds) as two segments, e.g. `2d 7h ago` / `in 3m 5s`;
/// the smaller segment is dropped when zero (`2m ago`). `clamp` pins direction.
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
    let ((p_secs, p_label), secondary) = bands(mag);
    let primary = mag / p_secs;
    let core = match secondary {
        Some((s_secs, s_label)) if (mag % p_secs) / s_secs > 0 => {
            let s = (mag % p_secs) / s_secs;
            format!("{primary}{p_label} {s}{s_label}")
        }
        _ => format!("{primary}{p_label}"),
    };
    match format {
        RelTimeFormat::Short if future => format!("in {core}"),
        RelTimeFormat::Short => format!("{core} ago"),
    }
}

/// Milliseconds until the label next changes — aligned to the smallest shown
/// unit (the secondary, or seconds), so the host wakes at that boundary. ≥ 1 s.
#[must_use]
pub fn next_change_delay_ms(delta_secs: i64, clamp: RelTimeClamp) -> u32 {
    let (mag, future) = magnitude_and_future(delta_secs, clamp);
    let ((p_secs, _), secondary) = bands(mag);
    let tick = secondary.map_or(p_secs, |(s, _)| s);
    let delay_secs = if future {
        (mag % tick) + 1
    } else {
        tick - (mag % tick)
    };
    u32::try_from(delay_secs.saturating_mul(1_000)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{format_rel, next_change_delay_ms};
    use bmc_wasm_protocol::{RelTimeClamp, RelTimeFormat};

    const SHORT: RelTimeFormat = RelTimeFormat::Short;
    const AUTO: RelTimeClamp = RelTimeClamp::Auto;

    #[test]
    fn zero_is_now() {
        assert_eq!(format_rel(0, SHORT, AUTO), "now");
    }

    #[test]
    fn seconds_band_is_single_segment() {
        assert_eq!(format_rel(5, SHORT, AUTO), "5s ago");
        assert_eq!(format_rel(-5, SHORT, AUTO), "in 5s");
        assert_eq!(format_rel(59, SHORT, AUTO), "59s ago");
    }

    #[test]
    fn two_segments_per_band() {
        assert_eq!(format_rel(90, SHORT, AUTO), "1m 30s ago");
        assert_eq!(format_rel(3_660, SHORT, AUTO), "1h 1m ago");
        assert_eq!(format_rel(200_000, SHORT, AUTO), "2d 7h ago");
        assert_eq!(format_rel(-90, SHORT, AUTO), "in 1m 30s");
    }

    #[test]
    fn drops_the_smaller_segment_when_zero() {
        assert_eq!(format_rel(120, SHORT, AUTO), "2m ago");
        assert_eq!(format_rel(3_600, SHORT, AUTO), "1h ago");
        assert_eq!(format_rel(86_400, SHORT, AUTO), "1d ago");
        assert_eq!(format_rel(172_800, SHORT, AUTO), "2d ago");
    }

    #[test]
    fn clamp_pins_direction() {
        assert_eq!(format_rel(-5, SHORT, RelTimeClamp::ElapsedOnly), "now");
        assert_eq!(format_rel(5, SHORT, RelTimeClamp::RemainingOnly), "now");
        assert_eq!(format_rel(-5, SHORT, RelTimeClamp::RemainingOnly), "in 5s");
    }

    #[test]
    fn delay_aligns_to_the_smaller_segment() {
        // seconds band and minutes-band (secondary = seconds) both tick each second.
        assert_eq!(next_change_delay_ms(5, AUTO), 1_000);
        assert_eq!(next_change_delay_ms(90, AUTO), 1_000);
        // hours band → secondary minutes; days band → secondary hours.
        assert_eq!(next_change_delay_ms(3_600, AUTO), 60_000);
        assert_eq!(next_change_delay_ms(200_000, AUTO), 1_600_000);
    }

    #[test]
    fn delay_counts_down_toward_the_boundary() {
        assert_eq!(next_change_delay_ms(-5, AUTO), 1_000);
        // "in 1h" one second past the hour edge → 2 s until it drops to "59m 59s".
        assert_eq!(next_change_delay_ms(-3_601, AUTO), 2_000);
    }

    #[test]
    fn deserializes_from_wire() {
        use bmc_wasm_protocol::{NODE_RELTIME, TextStyle};

        use crate::tree::{TreeNode, deserialize_tree};

        let mut bytes = vec![NODE_RELTIME];
        bytes.extend_from_slice(&1_700_000_000_i64.to_le_bytes());
        bytes.push(u8::from(RelTimeFormat::Short));
        bytes.push(u8::from(RelTimeClamp::ElapsedOnly));
        bytes.extend_from_slice(&TextStyle::default().to_bytes());

        let node = deserialize_tree(&bytes).expect("BUG: RelTime should deserialize");
        assert!(matches!(
            node,
            TreeNode::RelTime {
                anchor: 1_700_000_000,
                format: RelTimeFormat::Short,
                clamp: RelTimeClamp::ElapsedOnly,
                ..
            }
        ));
    }
}
