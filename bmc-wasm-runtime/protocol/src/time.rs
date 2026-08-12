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
