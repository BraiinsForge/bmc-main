// Copyright (C) 2025  Braiins Systems s.r.o.

use chrono::{Duration, TimeZone};
use chrono_tz::OffsetComponents;
use core::fmt;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{convert::TryInto, str::FromStr};
use strum_macros::{Display, EnumString};

use crate::timezone_variant::TIMEZONE_VARIANTS;

const DEFAULT_IANA: &str = "Etc/GMT";
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
    pub iana: &'static str,
    pub posix: &'static str,
}

#[derive(thiserror::Error, Debug)]
pub enum TimezoneError {
    #[error("Couldn't parse timezone")]
    ParseTimezone,
    #[error("Couldn't get offset for timezone")]
    Offset,
}

impl Timezone {
    /// Returns list of supported timezones for OpenWrt
    pub fn timezone_list() -> impl Iterator<Item = Self> {
        IntoIterator::into_iter(TIMEZONE_VARIANTS)
    }

    /// Returns current timezone offset from UTC
    pub fn current_timezone_offset(&self) -> Result<Offset, TimezoneError> {
        TryInto::<chrono_tz::Tz>::try_into(self)
            .map_err(|_| TimezoneError::Offset)
            .map(|tz| {
                let now = chrono::Utc::now().naive_utc();
                let offset = tz.offset_from_utc_datetime(&now);
                let offset_duration = offset.base_utc_offset() + offset.dst_offset();
                Offset::new(offset_duration)
            })
    }

    /// Returns current timezone offset from UTC
    pub fn current_timezone_tz_offset(&self) -> Result<chrono_tz::TzOffset, TimezoneError> {
        TryInto::<chrono_tz::Tz>::try_into(self)
            .map_err(|_| TimezoneError::Offset)
            .map(|tz| {
                let now = chrono::Utc::now().naive_utc();
                tz.offset_from_utc_datetime(&now)
            })
    }

    #[must_use]
    pub fn normalize_iana(&self) -> String {
        self.iana.replace(' ', "_")
    }
}

impl FromStr for Timezone {
    type Err = TimezoneError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match TIMEZONE_VARIANTS
            .iter()
            .find(|tz| tz.iana == s || tz.normalize_iana() == s)
        {
            Some(timezone) => Ok(timezone.clone()),
            None => Err(TimezoneError::ParseTimezone),
        }
    }
}

impl TryInto<chrono_tz::Tz> for &Timezone {
    type Error = TimezoneError;

    fn try_into(self) -> Result<chrono_tz::Tz, Self::Error> {
        chrono_tz::Tz::from_str(&self.normalize_iana()).map_err(|_| TimezoneError::ParseTimezone)
    }
}

impl fmt::Display for Timezone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.iana)
    }
}

impl Default for Timezone {
    fn default() -> Self {
        Self {
            iana: DEFAULT_IANA,
            posix: DEFAULT_POSIX,
        }
    }
}

impl PartialEq for Timezone {
    fn eq(&self, other: &Self) -> bool {
        self.iana == other.iana && self.posix == other.posix
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

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
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
