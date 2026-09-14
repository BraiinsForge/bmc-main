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

use super::*;

#[test]
fn duration_zero_and_negative() {
    assert_eq!(format_duration(0, false), "T-0");
    assert_eq!(format_duration(-1, false), "T-0");
    assert_eq!(format_duration(-100, true), "T-0");
}

#[test]
fn duration_seconds_only() {
    assert_eq!(format_duration(59, false), "0d 00h 00m");
    assert_eq!(format_duration(59, true), "0d 00h 00m 59s");
}

#[test]
fn duration_minutes() {
    assert_eq!(format_duration(60, false), "0d 00h 01m");
    assert_eq!(format_duration(3_599, true), "0d 00h 59m 59s");
}

#[test]
fn duration_hours() {
    assert_eq!(format_duration(3_600, false), "0d 01h 00m");
    assert_eq!(format_duration(3_661, false), "0d 01h 01m");
    assert_eq!(format_duration(3_661, true), "0d 01h 01m 01s");
}

#[test]
fn duration_days() {
    assert_eq!(format_duration(86_400, false), "1d 00h 00m");
    assert_eq!(format_duration(2_598_840, false), "30d 01h 54m");
    assert_eq!(format_duration(2_598_840, true), "30d 01h 54m 00s");
}

#[test]
fn duration_large() {
    // 365 days
    assert_eq!(format_duration(365 * 86_400, false), "365d 00h 00m");
}

fn caption(label: &TzLabel) -> String {
    let mut s = String::new();
    push_tz_caption(&mut s, label);
    s
}

#[test]
fn tz_caption_resolved_whole_and_half_hour() {
    let whole = TzLabel::Resolved {
        city: "Prague".to_owned(),
        offset_secs: 7_200,
    };
    assert_eq!(caption(&whole), "Prague (+2)");
    let half = TzLabel::Resolved {
        city: "Kolkata".to_owned(),
        offset_secs: 19_800,
    };
    assert_eq!(caption(&half), "Kolkata (+5:30)");
}

#[test]
fn tz_caption_resolved_negative_offset() {
    let label = TzLabel::Resolved {
        city: "New York".to_owned(),
        offset_secs: -18_000,
    };
    assert_eq!(caption(&label), "New York (-5)");
}

#[test]
fn tz_caption_unknown_reads_unknown() {
    let label = TzLabel::Unknown {
        city: "Prague".to_owned(),
        system_offset_secs: 3_600,
    };
    assert_eq!(caption(&label), "Prague (unknown)");
}

/// Monday the 14th of September 2026, 10:30 UTC — 12:30 in Prague.
#[cfg(not(target_arch = "wasm32"))]
const SEPTEMBER_MORNING: crate::host::SystemTime = crate::host::SystemTime {
    unix_secs: 1_789_381_800,
};

#[cfg(not(target_arch = "wasm32"))]
fn with_system_tz(tz: Option<&str>, check: impl FnOnce()) {
    let mut snapshot = crate::system::SnapshotBuilder::new();
    if let Some(tz) = tz {
        snapshot = snapshot.timezone(tz);
    }
    crate::system::set_current(snapshot.build());
    check();
    crate::system::set_current(crate::system::Snapshot::empty());
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_native_clock_reads_the_system_zone_the_host_would() {
    with_system_tz(Some("Europe/Prague"), || {
        let opts = FormatTimeOpts {
            with_seconds: true,
            ..FormatTimeOpts::default()
        };
        assert_eq!(format_time(SEPTEMBER_MORNING, opts), "12:30:00");
        assert_eq!(
            format_date(SEPTEMBER_MORNING, FormatDateOpts::default()),
            "14.09.2026"
        );
        assert_eq!(
            caption(&resolve_tz_for_label(None, SEPTEMBER_MORNING.unix_secs)),
            "Prague (+2)"
        );
    });
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn an_override_zone_the_deck_does_not_ship_reads_unknown() {
    with_system_tz(None, || {
        let helsinki = Tz::from_runtime("Europe/Helsinki");
        assert_eq!(
            caption(&resolve_tz_for_label(
                Some(&helsinki),
                SEPTEMBER_MORNING.unix_secs
            )),
            "Helsinki (+3)"
        );
        let half_typed = Tz::from_runtime("Europe/Helsi");
        assert_eq!(
            caption(&resolve_tz_for_label(
                Some(&half_typed),
                SEPTEMBER_MORNING.unix_secs
            )),
            "Helsi (unknown)"
        );
    });
}

#[test]
#[cfg(target_arch = "wasm32")]
fn read_host_buf_empty() {
    let buf = [0_u8; 64];
    assert_eq!(read_host_buf(&buf, 0), "");
    assert_eq!(read_host_buf(&buf, -1), "");
}

#[test]
#[cfg(target_arch = "wasm32")]
fn read_host_buf_valid() {
    let mut buf = [0_u8; 64];
    buf[..5].copy_from_slice(b"hello");
    assert_eq!(read_host_buf(&buf, 5), "hello");
}

#[test]
#[cfg(target_arch = "wasm32")]
fn read_host_buf_clamped() {
    let mut buf = [0_u8; 64];
    buf.fill(b'x');
    // len > 64 should be clamped
    assert_eq!(read_host_buf(&buf, 100), "x".repeat(64));
}

#[test]
fn f64_fixed_positive_with_decimals() {
    assert_eq!(format_f64_fixed(2.5, 2), "2.50");
    assert_eq!(format_f64_fixed(2.55, 2), "2.55");
    assert_eq!(format_f64_fixed(0.05, 2), "0.05");
    assert_eq!(format_f64_fixed(123.456, 2), "123.46");
    assert_eq!(format_f64_fixed(1.0, 3), "1.000");
}

#[test]
fn f64_fixed_zero_and_signed_zero() {
    assert_eq!(format_f64_fixed(0.0, 2), "0.00");
    assert_eq!(format_f64_fixed(-0.0, 2), "0.00");
}

#[test]
fn f64_fixed_negative() {
    assert_eq!(format_f64_fixed(-1.0, 2), "-1.00");
    assert_eq!(format_f64_fixed(-0.05, 2), "-0.05");
    assert_eq!(format_f64_fixed(-123.456, 2), "-123.46");
}

#[test]
fn f64_fixed_zero_decimals() {
    assert_eq!(format_f64_fixed(123.456, 0), "123");
    assert_eq!(format_f64_fixed(-2.5, 0), "-3");
    assert_eq!(format_f64_fixed(0.0, 0), "0");
}

#[test]
fn f64_fixed_clamps_excessive_decimals() {
    assert_eq!(format_f64_fixed(1.0, 10), "1.000000000");
    assert_eq!(format_f64_fixed(1.0, 9), "1.000000000");
}

#[test]
fn f64_fixed_does_not_emit_negative_zero() {
    assert_eq!(format_f64_fixed(-0.001, 2), "0.00");
}

mod si_carry {
    use super::{_host_format_number, _host_format_si_parts};

    #[test]
    fn a_mantissa_that_rounds_to_a_thousand_takes_the_next_prefix() {
        assert_eq!(
            _host_format_si_parts(999.99e18, 4, "H/s"),
            (_host_format_number(1.0, 3), "ZH/s".to_owned())
        );
        assert_eq!(
            _host_format_si_parts(999.96, 4, "W"),
            (_host_format_number(1.0, 3), "kW".to_owned())
        );
    }

    #[test]
    fn a_mantissa_that_rounds_short_of_a_thousand_keeps_its_prefix() {
        assert_eq!(
            _host_format_si_parts(999.94e18, 4, "H/s"),
            (_host_format_number(999.9, 1), "EH/s".to_owned())
        );
    }
}

mod si_decimals {
    use super::si_decimals;

    #[test]
    fn targets_the_requested_significant_figures() {
        assert_eq!(si_decimals(13.2, 3), 1); // 13.2
        assert_eq!(si_decimals(154.0, 3), 0); // 154
        assert_eq!(si_decimals(9.11, 3), 2); // 9.11
        assert_eq!(si_decimals(312.5, 3), 0); // 313
        assert_eq!(si_decimals(0.0, 3), 2); // 0.00
    }
}
