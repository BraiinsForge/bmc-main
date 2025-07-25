// Copyright (C) 2025  Braiins Systems s.r.o.

use chrono::{Duration, TimeZone};
use chrono_tz::OffsetComponents;
use core::fmt;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::str::FromStr;
use strum_macros::{Display, EnumString};

use crate::timezone_variant::TIMEZONE_VARIANTS;

const DEFAULT_CHRONO: chrono_tz::Tz = chrono_tz::Etc::GMT;
const DEFAULT_POSIX: &str = "GMT0";

#[derive(EnumString, Display, Debug, Serialize, Deserialize, Clone)]
pub enum TimeSystem {
    #[strum(serialize = "%I:%M")]
    Hour12,
    #[strum(serialize = "%H:%M")]
    Hour24,
}

#[expect(dead_code)]
impl TimeSystem {
    pub(crate) fn display_with_seconds(&self) -> TimeSystemWithSeconds {
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

impl Default for TimeSystem {
    fn default() -> Self {
        Self::Hour24
    }
}

#[derive(EnumString, Display)]
pub(crate) enum TimeSystemWithSeconds {
    #[strum(serialize = "%I:%M:%S")]
    Hour12,
    #[strum(serialize = "%H:%M:%S")]
    Hour24,
}

#[derive(Clone, Debug, SerializeDisplay, DeserializeFromStr)]
pub struct Timezone {
    chrono: chrono_tz::Tz,
    posix: &'static str,
}

#[derive(thiserror::Error, Debug)]
pub enum TimezoneError {
    #[error("Couldn't parse timezone")]
    ParseTimezone,
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
}

impl FromStr for Timezone {
    type Err = TimezoneError;

    fn from_str(iana: &str) -> Result<Self, Self::Err> {
        match Self::list().iter().find(|tz| tz.iana() == iana) {
            Some(timezone) => Ok(timezone.clone()),
            None => Err(TimezoneError::ParseTimezone),
        }
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
        f.write_str(self.iana())
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

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_dd_mm_yyyy_dot() {
        let date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
        let fmt = DateFormat::DdMmYyyyDot;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15.03.2025");
    }

    #[test]
    fn test_dd_mm_yyyy_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        let fmt = DateFormat::DdMmYyyySlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15/08/2025");
    }

    #[test]
    fn test_d_m_yyyy_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        let fmt = DateFormat::DMYyyySlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15/8/2025");
    }

    #[test]
    fn test_m_d_yyyy_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        let fmt = DateFormat::MDYyyySlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "8/15/2025");
    }

    #[test]
    fn test_dd_mm_yyyy_dash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        let fmt = DateFormat::DdMmYyyyDash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "15-08-2025");
    }

    #[test]
    fn test_yyyy_m_d_slash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        let fmt = DateFormat::YyyyMDSlash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "2025/8/15");
    }

    #[test]
    fn test_yyyy_mm_dd_dot() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        let fmt = DateFormat::YyyyMmDdDot;
        assert_eq!(date.format(fmt.format_string()).to_string(), "2025.08.15");
    }

    #[test]
    fn test_yyyy_mm_dd_dash() {
        let date = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        let fmt = DateFormat::YyyyMmDdDash;
        assert_eq!(date.format(fmt.format_string()).to_string(), "2025-08-15");
    }
}
