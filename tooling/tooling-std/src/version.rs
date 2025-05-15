// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::impl_serde_as_string;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::num::{NonZero, ParseIntError};
use std::str::FromStr;
use thiserror::Error;

/// Toolbox version, example: `23.06.1` (stable), `24.06-rc.3`(release candidate) or `nightly` (nightly).
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum AppVersion {
    Stable(StableVersionName),
    ReleaseCandidate(StableVersionName, NonZero<u8>),
    Nightly,
}

impl AppVersion {
    #[must_use]
    pub const fn stable(year: u8, month: u8, patch: Option<u8>) -> Self {
        let patch = match patch {
            Some(patch) => match NonZero::<u8>::new(patch) {
                Some(patch) => Some(patch),
                None => panic!("patch number is zero"),
            },
            None => None,
        };

        Self::Stable(StableVersionName { year, month, patch })
    }

    #[must_use]
    pub const fn rc(year: u8, month: u8, patch: Option<u8>, rc_iteration: u8) -> Self {
        let patch = match patch {
            Some(patch) => match NonZero::new(patch) {
                Some(patch) => Some(patch),
                None => panic!("patch number is zero"),
            },
            None => None,
        };

        let Some(rc_iteration) = NonZero::new(rc_iteration) else {
            panic!("rc iteration is zero")
        };

        Self::ReleaseCandidate(StableVersionName { year, month, patch }, rc_iteration)
    }
}

impl_serde_as_string!(AppVersion);

impl Display for AppVersion {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            AppVersion::Stable(ver) => write!(f, "{ver}"),
            AppVersion::ReleaseCandidate(ver, iter) => write!(f, "{ver}-rc.{iter}"),
            AppVersion::Nightly => write!(f, "nightly"),
        }
    }
}

impl FromStr for AppVersion {
    type Err = ParseVersionError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input == "nightly" {
            Ok(Self::Nightly)
        } else if input.contains("rc") {
            let mut parts = input.split("-rc.");

            let stable = parts
                .next()
                .ok_or(ParseVersionError::MissingStableVersion)?;
            let stable = StableVersionName::from_str(stable)?;

            let iter = parts.next().ok_or(ParseVersionError::MissingRcIter)?;
            let iter: u8 = iter.parse()?;
            let iter = NonZero::new(iter).ok_or(ParseVersionError::ZeroRc)?;

            Ok(Self::ReleaseCandidate(stable, iter))
        } else {
            Ok(Self::Stable(StableVersionName::from_str(input)?))
        }
    }
}

#[derive(Error, Debug, Clone, Eq, PartialEq)]
pub enum ParseVersionError {
    #[error(transparent)]
    ChronoParseError(#[from] chrono::ParseError),
    #[error(transparent)]
    ParseIntError(#[from] ParseIntError),
    #[error("missing stable version")]
    MissingStableVersion,
    #[error("missing RC iter")]
    MissingRcIter,
    #[error("RC is zero")]
    ZeroRc,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct StableVersionName {
    /// Last two digits of a year.
    pub year: u8,
    /// Month number.
    pub month: u8,
    /// Patch release number.
    pub patch: Option<NonZero<u8>>,
}

impl_serde_as_string!(StableVersionName);

impl Display for StableVersionName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}.{:02}", self.year, self.month)?;

        if let Some(patch) = self.patch {
            write!(f, ".{patch}")?;
        }

        Ok(())
    }
}

impl FromStr for StableVersionName {
    type Err = ParseIntError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut tokens = input.splitn(3, '.');
        Ok(Self {
            year: tokens.next().unwrap_or_default().parse()?,
            month: tokens.next().unwrap_or_default().parse()?,
            patch: tokens.next().map(str::parse).transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_stable() {
        let expected = "23.06.1";
        let output = AppVersion::Stable(StableVersionName {
            year: 23,
            month: 6,
            patch: Some(NonZero::<u8>::new(1).unwrap()),
        })
        .to_string();
        assert_eq!(expected, output);
    }

    #[test]
    fn parse_stable() {
        let expected = Ok(AppVersion::Stable(StableVersionName {
            year: 23,
            month: 6,
            patch: Some(NonZero::<u8>::new(1).unwrap()),
        }));
        let output = "23.06.1".parse::<AppVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn display_rc() {
        let expected = "25.12.1-rc.1";
        let output = AppVersion::ReleaseCandidate(
            StableVersionName {
                year: 25,
                month: 12,
                patch: Some(NonZero::<u8>::new(1).unwrap()),
            },
            NonZero::<u8>::new(1).unwrap(),
        )
        .to_string();
        assert_eq!(expected, output);
    }

    #[test]
    fn parse_rc() {
        let expected = Ok(AppVersion::ReleaseCandidate(
            StableVersionName {
                year: 25,
                month: 12,
                patch: Some(NonZero::<u8>::new(1).unwrap()),
            },
            NonZero::<u8>::new(2).unwrap(),
        ));
        let output = "25.12.1-rc.2".parse::<AppVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn display_nightly() {
        let expected = "nightly";
        let output = AppVersion::Nightly.to_string();
        assert_eq!(expected, output);
    }

    #[test]
    fn parse_nightly() {
        let expected = Ok(AppVersion::Nightly);
        let output = "nightly".parse::<AppVersion>();
        assert_eq!(expected, output);
    }
}
