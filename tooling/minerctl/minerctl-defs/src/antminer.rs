// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::{ControlBoard, bos::BosPlatform};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use enum_assoc::Assoc;
use std::str::FromStr;
use strum::Display;

#[derive(Display, Assoc, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[func(pub const fn control_board(&self) -> ControlBoard)]
pub enum AntminerPlatform {
    #[assoc(control_board = ControlBoard::Zynq)]
    Zynq,
    #[assoc(control_board = ControlBoard::BBB)]
    BBB,
    #[assoc(control_board = ControlBoard::AML)]
    AML,
    #[assoc(control_board = ControlBoard::CVITEK)]
    CVITEK,
}

impl From<AntminerPlatform> for BosPlatform {
    fn from(value: AntminerPlatform) -> Self {
        match value {
            AntminerPlatform::AML => BosPlatform::Am3aml,
            AntminerPlatform::BBB => BosPlatform::Am3bbb,
            AntminerPlatform::CVITEK => BosPlatform::CvitekBm1Am2,
            AntminerPlatform::Zynq => BosPlatform::Am2s17,
        }
    }
}

#[must_use]
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct AntminerVersion(DateTime<Utc>);

impl AntminerVersion {
    /// Creates [`AntminerVersion`] from a UTC date. Panics if the date is invalid.
    pub const fn from_utc_ymd(year: i32, month: u32, day: u32) -> Self {
        let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
            panic!("invalid date")
        };

        let Some(datetime) = date.and_hms_opt(0, 0, 0) else {
            unreachable!();
        };

        let dt = DateTime::from_naive_utc_and_offset(datetime, Utc);

        Self::from_datetime::<Utc>(&dt)
    }

    pub const fn from_datetime<Tz: TimeZone>(dt: &DateTime<Tz>) -> Self {
        Self(dt.to_utc())
    }

    #[must_use]
    pub fn format(&self, fmt: &str) -> String {
        self.0.format(fmt).to_string()
    }
}

impl FromStr for AntminerVersion {
    type Err = chrono::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // replace tz names because chrono can't parse them: https://docs.rs/chrono/latest/chrono/format/strftime/index.html#fn6
        let s = s.replace("CST", "−06:00");

        let datetime = DateTime::parse_from_str(s.trim(), "%a %b %e %T %z %Y")?;

        Ok(Self(datetime.to_utc()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        let d = AntminerVersion::from_str("Tue Jun 22 17:45:49 CST 2021").unwrap();
        assert_eq!(format!("{:?}", d.0), "2021-06-22T23:45:49Z");
        let d = AntminerVersion::from_str("Mon Oct 31 19:01:57 CST 2022").unwrap();
        assert_eq!(format!("{:?}", d.0), "2022-11-01T01:01:57Z");
        let d = AntminerVersion::from_str("Mon Dec 26 17:10:01 CST 2022").unwrap();
        assert_eq!(format!("{:?}", d.0), "2022-12-26T23:10:01Z");
    }
}
