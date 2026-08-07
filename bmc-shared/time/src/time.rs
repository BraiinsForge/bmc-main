// Copyright (C) 2025  Braiins Systems s.r.o.
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

use chrono::{Duration, TimeZone};
use chrono_tz::OffsetComponents;
use core::fmt;
use serde::{Deserialize, Serialize, Serializer};
use serde_with::DeserializeFromStr;
use std::str::FromStr;
use strum_macros::{Display, EnumString};

use crate::timezone_variant::{TIMEZONE_BY_IANA, TIMEZONE_VARIANTS};

const DEFAULT_CHRONO: chrono_tz::Tz = chrono_tz::Etc::GMT;
const DEFAULT_POSIX: &str = "GMT0";

#[derive(
    Copy, EnumString, Display, Debug, Default, Serialize, Deserialize, Clone, PartialEq, Eq,
)]
pub enum TimeSystem {
    #[strum(serialize = "%I:%M")]
    Hour12,
    #[strum(serialize = "%H:%M")]
    #[default]
    Hour24,
}

#[expect(dead_code)]
impl TimeSystem {
    pub(crate) fn display_with_seconds(self) -> TimeSystemWithSeconds {
        match self {
            TimeSystem::Hour12 => TimeSystemWithSeconds::Hour12,
            TimeSystem::Hour24 => TimeSystemWithSeconds::Hour24,
        }
    }

    #[must_use]
    pub fn is_24(&self) -> bool {
        matches!(self, Self::Hour24)
    }
}

#[derive(EnumString, Display)]
pub(crate) enum TimeSystemWithSeconds {
    #[strum(serialize = "%I:%M:%S")]
    Hour12,
    #[strum(serialize = "%H:%M:%S")]
    Hour24,
}

#[derive(Clone, Debug, DeserializeFromStr)]
pub struct Timezone {
    chrono: chrono_tz::Tz,
    posix: &'static str,
}

#[derive(thiserror::Error, Debug)]
pub enum TimezoneError {
    #[error("Couldn't parse timezone")]
    ParseTimezone,
    #[error("Couldn't parse offset")]
    Offset,
}

impl Timezone {
    /// This function may panic, if IANA string is not supported.
    /// It should be used only in [TIMEZONE_VARIANTS] initialization.
    pub(crate) fn new(iana: &'static str, posix: &'static str) -> Self {
        // IANA values are normalized - replaced whitespace with underscore, which is a typical format & required for chrono_tz.
        // OpenWRT is doing it anyway - https://github.com/openwrt/openwrt/blob/4dd35ca6747a57261f3b10982a4a8cc765d6549f/package/base-files/files/etc/init.d/system#L25
        // It was tested using `uci set system.@system[0].zonename="America/Port of Spain"` and `uci set system.@system[0].zonename="America/Port_of_Spain"`
        let iana = iana.replace(' ', "_");
        let expect_msg = format!("BUG: invalid IANA timezone '{iana}'");
        let chrono = chrono_tz::Tz::from_str(&iana).expect(&expect_msg);

        Self { chrono, posix }
    }

    #[must_use]
    #[inline]
    pub fn iana(&self) -> &str {
        self.chrono.name()
    }

    /// Returns the city part of the IANA name in a human-readable format.
    /// E.g. `"Europe/Prague"` → `"Prague"`, `"America/New_York"` → `"New York"`.
    #[must_use]
    pub fn city_name(&self) -> String {
        let iana = self.iana();
        let city = iana.rsplit('/').next().unwrap_or(iana);
        city.replace('_', " ")
    }

    #[must_use]
    #[inline]
    pub fn posix(&self) -> &str {
        self.posix
    }

    #[must_use]
    #[inline]
    pub fn chrono(&self) -> &chrono_tz::Tz {
        &self.chrono
    }

    /// Returns list of supported timezones for OpenWrt
    #[inline]
    pub fn list() -> &'static [Timezone] {
        TIMEZONE_VARIANTS.as_slice()
    }

    /// O(1) lookup of a supported timezone by IANA name. Returns `None`
    /// if the name is not in the curated list.
    #[must_use]
    pub fn lookup(iana: &str) -> Option<&'static Timezone> {
        TIMEZONE_BY_IANA.get(iana).copied()
    }

    /// Returns current timezone offset from UTC
    #[must_use]
    pub fn chrono_offset(&self) -> chrono_tz::TzOffset {
        let now = chrono::Utc::now().naive_utc();
        self.chrono.offset_from_utc_datetime(&now)
    }

    /// Returns current timezone offset from UTC
    #[must_use]
    pub fn offset(&self) -> Offset {
        let offset = self.chrono_offset();
        let offset_duration = offset.base_utc_offset() + offset.dst_offset();
        Offset::new(offset_duration)
    }

    /// Returns a compact UTC offset string like `"+1"` or `"+5:45"`.
    #[must_use]
    pub fn display_offset(&self) -> String {
        let offset = self.offset();
        let h = offset.hours();
        let m = offset.minutes().abs();
        if m == 0 {
            format!("{h:+}")
        } else {
            format!("{h:+}:{m:02}")
        }
    }
}

impl Serialize for Timezone {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.iana())
    }
}

impl FromStr for Timezone {
    type Err = TimezoneError;

    fn from_str(iana: &str) -> Result<Self, Self::Err> {
        Self::lookup(iana)
            .cloned()
            .ok_or(TimezoneError::ParseTimezone)
    }
}

impl From<Timezone> for chrono_tz::Tz {
    fn from(value: Timezone) -> Self {
        value.chrono
    }
}

impl From<&Timezone> for chrono_tz::Tz {
    fn from(value: &Timezone) -> Self {
        value.chrono
    }
}

impl fmt::Display for Timezone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            f.write_str(self.iana())
        } else {
            f.write_str(&self.city_name())
        }
    }
}

impl Default for Timezone {
    fn default() -> Self {
        Self {
            chrono: DEFAULT_CHRONO,
            posix: DEFAULT_POSIX,
        }
    }
}

impl PartialEq for Timezone {
    fn eq(&self, other: &Self) -> bool {
        self.chrono == other.chrono && self.posix == other.posix
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Offset {
    inner: Duration,
}

impl Offset {
    fn new(value: Duration) -> Self {
        Self { inner: value }
    }

    /// Total offset hours (signed).
    #[must_use]
    pub fn hours(&self) -> i64 {
        self.inner.num_hours()
    }

    /// Remaining minutes after whole hours (always 0..59).
    #[must_use]
    pub fn minutes(&self) -> i64 {
        self.inner.num_minutes().rem_euclid(60)
    }
}

impl Default for Offset {
    fn default() -> Self {
        Self {
            inner: Duration::zero(),
        }
    }
}

impl fmt::Display for Offset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hours = self.inner.num_hours();
        let minutes = self.inner.num_minutes().rem_euclid(60);
        write!(f, "{hours:+03}:{minutes:02}")
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum DateFormat {
    #[default]
    DdMmYyyyDot,
    DdMmYyyySlash,
    DMYyyySlash,
    MDYyyySlash,
    DdMmYyyyDash,
    YyyyMDSlash,
    YyyyMmDdDot,
    YyyyMmDdDash,
}

impl From<u8> for DateFormat {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::DdMmYyyyDot,
            1 => Self::DdMmYyyySlash,
            2 => Self::DMYyyySlash,
            3 => Self::MDYyyySlash,
            4 => Self::DdMmYyyyDash,
            5 => Self::YyyyMDSlash,
            6 => Self::YyyyMmDdDot,
            7 => Self::YyyyMmDdDash,
            _ => Self::default(),
        }
    }
}

impl DateFormat {
    #[must_use]
    pub fn format_string(&self) -> &str {
        match self {
            DateFormat::DdMmYyyyDot => "%d.%m.%Y",   // 15.03.2025
            DateFormat::DdMmYyyySlash => "%d/%m/%Y", // 15/08/2025
            DateFormat::DMYyyySlash => "%-d/%-m/%Y", // 15/8/2025
            DateFormat::MDYyyySlash => "%-m/%-d/%Y", // 8/15/2025
            DateFormat::DdMmYyyyDash => "%d-%m-%Y",  // 15-08-2025
            DateFormat::YyyyMDSlash => "%Y/%-m/%-d", // 2025/8/15
            DateFormat::YyyyMmDdDot => "%Y.%m.%d",   // 2025.08.15
            DateFormat::YyyyMmDdDash => "%Y-%m-%d",  // 2025-08-15
        }
    }
}

/// The derived order follows the declaration, which follows the
/// discriminants — so an ordered set of days iterates as a calendar week
/// and its numbers come out ascending.
#[derive(
    Serialize,
    Deserialize,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Copy,
    Display,
    Default,
)]
pub enum WeekDay {
    #[default]
    Monday = 1,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl WeekDay {
    #[must_use]
    pub fn as_number_string(self) -> String {
        (self as u8).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_dd_mm_yyyy_dot() {
        let date = NaiveDate::from_ymd_opt(2025, 3, 15).expect("BUG: invalid date");
        let fmt = DateFormat::DdMmYyyyDot;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15.03.2025");
    }

    #[test]
    fn test_dd_mm_yyyy_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).expect("BUG: invalid date");
        let fmt = DateFormat::DdMmYyyySlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15/08/2025");
    }

    #[test]
    fn test_d_m_yyyy_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).expect("BUG: invalid date");
        let fmt = DateFormat::DMYyyySlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15/8/2025");
    }

    #[test]
    fn test_m_d_yyyy_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).expect("BUG: invalid date");
        let fmt = DateFormat::MDYyyySlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "8/15/2025");
    }

    #[test]
    fn test_dd_mm_yyyy_dash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).expect("BUG: invalid date");
        let fmt = DateFormat::DdMmYyyyDash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15-08-2025");
    }

    #[test]
    fn test_yyyy_m_d_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).expect("BUG: invalid date");
        let fmt = DateFormat::YyyyMDSlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "2025/8/15");
    }

    #[test]
    fn test_yyyy_mm_dd_dot() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).expect("BUG: invalid date");
        let fmt = DateFormat::YyyyMmDdDot;
        assert_eq!(date.format(fmt.format_string()).to_string(), "2025.08.15");
    }

    #[test]
    fn test_yyyy_mm_dd_dash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).expect("BUG: invalid date");
        let fmt = DateFormat::YyyyMmDdDash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "2025-08-15");
    }
}
