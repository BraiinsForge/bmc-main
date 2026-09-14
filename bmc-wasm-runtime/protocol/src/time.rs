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

//! Dates and times as they cross the host boundary.
//!
//! Each type owns the bytes it travels as, so the host writing them and
//! the widget reading them cannot drift apart.
//!
//! With `domain`, the host's calendar too: `wall_clock` and `utc_offset_secs` read
//! any zone chrono knows and `Local`, `zone_offset_secs` only the zones the device ships.

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// The month's abbreviated name, or `""` where the month is not one.
#[must_use]
pub fn month_short(month: u8) -> &'static str {
    month
        .checked_sub(1)
        .and_then(|index| MONTHS.get(usize::from(index)))
        .copied()
        .unwrap_or_default()
}

/// The weekday's abbreviated name, counting Monday as 0.
#[must_use]
pub fn weekday_short(weekday: u8) -> &'static str {
    WEEKDAYS
        .get(usize::from(weekday))
        .copied()
        .unwrap_or_default()
}

/// `0` for a month that is not one, failing the day check with it.
fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

/// Zeller's congruence, mapped to this wire's Monday-as-0.
fn weekday_of(year: u16, month: u8, day: u8) -> u8 {
    let (month, year) = if month < 3 {
        (i64::from(month) + 12, i64::from(year) - 1)
    } else {
        (i64::from(month), i64::from(year))
    };
    let century = year.div_euclid(100);
    let year = year.rem_euclid(100);
    let saturday0 = (i64::from(day)
        + (13 * (month + 1)).div_euclid(5)
        + year
        + year.div_euclid(4)
        + century.div_euclid(4)
        + 5 * century)
        .rem_euclid(7);
    u8::try_from((saturday0 + 5).rem_euclid(7)).expect("BUG: rem_euclid(7) is 0..=6")
}

/// These bytes are host-written, so a day the calendar does not hold, or
/// a weekday label disagreeing with the date, is corruption to refuse.
fn is_a_real_day(year: u16, month: u8, day: u8, weekday: u8) -> bool {
    (1..=days_in_month(year, month)).contains(&day) && weekday == weekday_of(year, month, day)
}

/// A day, with no time of day and so no timezone.
///
/// Deliberately not a [`LocalDateTime`] with its clock fields zeroed: a
/// date names a day, and zeroes would read as midnight to anyone who
/// forgot which of the two they were holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// 0 = Monday, 6 = Sunday.
    pub weekday: u8,
}

impl CalendarDate {
    pub const WIRE_LEN: usize = 5;

    #[must_use]
    pub fn to_wire(self) -> [u8; Self::WIRE_LEN] {
        let year = self.year.to_le_bytes();
        [year[0], year[1], self.month, self.day, self.weekday]
    }

    /// Read what [`Self::to_wire`] wrote, or `None` if the bytes name no
    /// real day.
    #[must_use]
    pub fn from_wire(buf: [u8; Self::WIRE_LEN]) -> Option<Self> {
        let date = Self {
            year: u16::from_le_bytes([buf[0], buf[1]]),
            month: buf[2],
            day: buf[3],
            weekday: buf[4],
        };
        is_a_real_day(date.year, date.month, date.day, date.weekday).then_some(date)
    }

    #[must_use]
    pub fn month_short(&self) -> &'static str {
        month_short(self.month)
    }

    #[must_use]
    pub fn weekday_short(&self) -> &'static str {
        weekday_short(self.weekday)
    }
}

/// Wall-clock time in one particular zone, which the host resolved from
/// an instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// 0 = Monday, 6 = Sunday.
    pub weekday: u8,
}

impl LocalDateTime {
    pub const WIRE_LEN: usize = 8;

    #[must_use]
    pub fn to_wire(self) -> [u8; Self::WIRE_LEN] {
        let year = self.year.to_le_bytes();
        [
            year[0],
            year[1],
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            self.weekday,
        ]
    }

    /// Read what [`Self::to_wire`] wrote, or `None` if the bytes name no
    /// real moment.
    #[must_use]
    pub fn from_wire(buf: [u8; Self::WIRE_LEN]) -> Option<Self> {
        let at = Self {
            year: u16::from_le_bytes([buf[0], buf[1]]),
            month: buf[2],
            day: buf[3],
            hour: buf[4],
            minute: buf[5],
            second: buf[6],
            weekday: buf[7],
        };
        let clock_reads = at.hour < 24 && at.minute < 60 && at.second < 60;
        (is_a_real_day(at.year, at.month, at.day, at.weekday) && clock_reads).then_some(at)
    }

    #[must_use]
    pub fn seconds_since_midnight(&self) -> u32 {
        u32::from(self.hour) * 3_600 + u32::from(self.minute) * 60 + u32::from(self.second)
    }

    #[must_use]
    pub fn month_short(&self) -> &'static str {
        month_short(self.month)
    }

    #[must_use]
    pub fn weekday_short(&self) -> &'static str {
        weekday_short(self.weekday)
    }
}

/// A UTC instant under a chrono strftime pattern, or `None` for an instant
/// chrono cannot represent or a pattern it cannot format.
#[cfg(feature = "domain")]
#[must_use]
pub fn strftime_utc(unix_secs: i64, pattern: &str) -> Option<String> {
    use core::fmt::Write;

    let at = chrono::DateTime::<chrono::Utc>::from_timestamp(unix_secs, 0)?;
    let mut formatted = String::new();
    write!(formatted, "{}", at.format(pattern)).ok()?;
    Some(formatted)
}

/// A zone chrono can read an instant in: any IANA name
/// it knows, or `Local` for the process's own.
#[cfg(feature = "domain")]
enum Zone {
    Named(chrono_tz::Tz),
    Local,
}

#[cfg(feature = "domain")]
impl Zone {
    fn parse(name: &str) -> Option<Self> {
        if name == "Local" {
            return Some(Self::Local);
        }
        name.parse().ok().map(Self::Named)
    }

    /// The instant as this zone reads it, its offset fixed for that moment.
    fn at(&self, unix_secs: i64) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        let at_utc = chrono::DateTime::<chrono::Utc>::from_timestamp(unix_secs, 0)?;
        Some(match self {
            Self::Named(tz) => at_utc.with_timezone(tz).fixed_offset(),
            Self::Local => at_utc.with_timezone(&chrono::Local).fixed_offset(),
        })
    }
}

/// The wall clock a UTC instant reads in any zone chrono knows, or `Local`;
/// `None` where the zone is unknown.
#[cfg(feature = "domain")]
#[must_use]
pub fn wall_clock(unix_secs: i64, zone: &str) -> Option<LocalDateTime> {
    use chrono::{Datelike, Timelike};

    let local = Zone::parse(zone)?.at(unix_secs)?;
    Some(LocalDateTime {
        year: u16::try_from(local.year()).ok()?,
        month: u8::try_from(local.month()).ok()?,
        day: u8::try_from(local.day()).ok()?,
        hour: u8::try_from(local.hour()).ok()?,
        minute: u8::try_from(local.minute()).ok()?,
        second: u8::try_from(local.second()).ok()?,
        weekday: u8::try_from(local.weekday().num_days_from_monday()).ok()?,
    })
}

/// The UTC offset in seconds of any zone chrono knows, or `Local`,
/// at an instant so DST is honoured; `None` where the zone is unknown.
#[cfg(feature = "domain")]
#[must_use]
pub fn utc_offset_secs(unix_secs: i64, zone: &str) -> Option<i32> {
    Some(Zone::parse(zone)?.at(unix_secs)?.offset().local_minus_utc())
}

/// [`utc_offset_secs`] for the zones the device ships — `bmc_shared_time`'s
/// curated OpenWrt set, the one `tz!` checks against — and `None` for any other.
#[cfg(feature = "domain")]
#[must_use]
pub fn zone_offset_secs(unix_secs: i64, zone: &str) -> Option<i32> {
    bmc_shared_time::time::Timezone::lookup(zone)?;
    utc_offset_secs(unix_secs, zone)
}

#[cfg(test)]
mod tests {
    use super::{CalendarDate, LocalDateTime, month_short};

    /// Friday the 21st of August 2026, weekday 4 counting Monday as 0.
    #[test]
    fn a_date_survives_the_trip_it_encodes_itself_for() {
        let date = CalendarDate {
            year: 2026,
            month: 8,
            day: 21,
            weekday: 4,
        };
        assert_eq!(CalendarDate::from_wire(date.to_wire()), Some(date));
        assert_eq!(date.month_short(), "Aug");
        assert_eq!(date.weekday_short(), "Fri");
    }

    #[test]
    fn a_moment_survives_the_trip_it_encodes_itself_for() {
        let at = LocalDateTime {
            year: 2026,
            month: 8,
            day: 23,
            hour: 13,
            minute: 30,
            second: 5,
            weekday: 6,
        };
        assert_eq!(LocalDateTime::from_wire(at.to_wire()), Some(at));
        assert_eq!(at.seconds_since_midnight(), 48_605);
    }

    /// Months count from one, so zero is not "the month before February".
    #[test]
    fn a_month_outside_the_year_names_nothing() {
        assert_eq!(month_short(0), "");
        assert_eq!(month_short(13), "");
        assert_eq!(month_short(u8::MAX), "");
        assert_eq!(month_short(1), "Jan");
        assert_eq!(month_short(12), "Dec");
    }

    /// The 21st of August 2026 is a Friday;
    /// every wrong claim about that day must be refused, not repaired.
    #[test]
    fn a_day_the_calendar_does_not_hold_is_refused() {
        let wire = |month: u8, day: u8, weekday: u8| {
            let year = 2026_u16.to_le_bytes();
            CalendarDate::from_wire([year[0], year[1], month, day, weekday])
        };
        assert_eq!(wire(2, 31, 1), None, "February has no 31st");
        assert_eq!(wire(4, 31, 0), None, "April has no 31st");
        assert_eq!(wire(8, 21, 3), None, "the 21st is a Friday, not Thursday");
        assert!(wire(8, 21, 4).is_some());
    }

    #[test]
    fn leap_years_follow_all_three_gregorian_rules() {
        let feb29 = |year: u16, weekday: u8| {
            let y = year.to_le_bytes();
            CalendarDate::from_wire([y[0], y[1], 2, 29, weekday]).is_some()
        };
        assert!(feb29(2024, 3), "2024 is a leap year; the 29th a Thursday");
        assert!(!feb29(2023, 2), "2023 is none");
        assert!(!feb29(1900, 2), "a century is none");
        assert!(feb29(2000, 1), "unless divisible by 400; a Tuesday");
    }

    /// Zeroed bytes are what an untouched buffer holds,
    /// so they must not read as a real day.
    #[test]
    fn bytes_naming_no_real_day_are_refused() {
        assert_eq!(CalendarDate::from_wire([0; CalendarDate::WIRE_LEN]), None);
        assert_eq!(LocalDateTime::from_wire([0; LocalDateTime::WIRE_LEN]), None);
    }

    #[test]
    fn a_clock_beyond_its_range_is_refused() {
        let mut wire = LocalDateTime {
            year: 2026,
            month: 8,
            day: 23,
            hour: 13,
            minute: 30,
            second: 0,
            weekday: 6,
        }
        .to_wire();
        wire[4] = 24;
        assert_eq!(LocalDateTime::from_wire(wire), None);
    }
}

#[cfg(all(test, feature = "domain"))]
mod calendar_tests {
    use super::{LocalDateTime, strftime_utc, utc_offset_secs, wall_clock, zone_offset_secs};

    /// Monday the 14th of September 2026, 10:30 UTC.
    const SEPTEMBER_MORNING: i64 = 1_789_381_800;
    /// The 15th of January 2026, noon UTC.
    const JANUARY_NOON: i64 = 1_768_478_400;

    #[test]
    fn strftime_reads_the_instant_in_utc() {
        assert_eq!(
            strftime_utc(SEPTEMBER_MORNING, "%a %-d %B %Y %H:%M").as_deref(),
            Some("Mon 14 September 2026 10:30")
        );
    }

    /// `%Q` is no chrono specifier and a trailing `%` is cut short;
    /// both are refused rather than aborting the caller.
    #[test]
    fn a_pattern_chrono_cannot_format_reads_none() {
        assert_eq!(strftime_utc(SEPTEMBER_MORNING, "%Q"), None);
        assert_eq!(strftime_utc(SEPTEMBER_MORNING, "100%"), None);
    }

    #[test]
    fn a_wall_clock_shifts_by_the_zone_and_keeps_the_weekday() {
        assert_eq!(
            wall_clock(SEPTEMBER_MORNING, "Europe/Prague"),
            Some(LocalDateTime {
                year: 2026,
                month: 9,
                day: 14,
                hour: 12,
                minute: 30,
                second: 0,
                weekday: 0,
            })
        );
        assert_eq!(wall_clock(SEPTEMBER_MORNING, "Not/AZone"), None);
    }

    /// Prague is an hour ahead in winter and two in summer.
    #[test]
    fn a_zone_offset_follows_daylight_saving() {
        assert_eq!(zone_offset_secs(JANUARY_NOON, "Europe/Prague"), Some(3_600));
        assert_eq!(
            zone_offset_secs(SEPTEMBER_MORNING, "Europe/Prague"),
            Some(7_200)
        );
        assert_eq!(zone_offset_secs(JANUARY_NOON, "Not/AZone"), None);
    }

    /// `UTC` and the `Asia/Calcutta` alias are zones chrono knows but
    /// the device does not ship; only the curated reading refuses them.
    #[test]
    fn the_device_list_gates_the_offset_but_not_the_wall_clock() {
        for zone in ["UTC", "Asia/Calcutta"] {
            assert!(utc_offset_secs(JANUARY_NOON, zone).is_some(), "{zone}");
            assert!(wall_clock(JANUARY_NOON, zone).is_some(), "{zone}");
            assert_eq!(zone_offset_secs(JANUARY_NOON, zone), None, "{zone}");
        }
        assert_eq!(utc_offset_secs(JANUARY_NOON, "Asia/Calcutta"), Some(19_800));
    }
}
