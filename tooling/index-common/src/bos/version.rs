// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::commit::CommitHashShort;
use crate::impl_serde_as_string;
use chrono::{Datelike, NaiveDate};
use regex::{Match, Regex};
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::num::{NonZero, ParseIntError};
use std::str::FromStr;
use std::sync::LazyLock;
use strum::{Display, EnumString};
use tap::Pipe;
use thiserror::Error;

/// Representation of the full version string, example: `2022-09-27-0-06ba61b5-22.08.1-plus-nightly`.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct BosVersion {
    /// The date of the release.
    pub date: NaiveDate,
    /// Index of the release within the day specified by `date`, usually just `0`.
    pub day_index: u8,
    /// First 4 bytes (8 chars hex) of the commit hash.
    pub commit: CommitHashShort,
    /// Display version name, example: `22.08.1`.
    pub version: VersionName,
    /// Whether it's the "plus" version.
    pub is_plus: bool,
    /// Version suffix, used for internal builds only.
    pub build: Option<InternalBuildSuffix>,
}

impl_serde_as_string!(BosVersion);

/// Display version name, example: `22.08.1`.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VersionName {
    /// Last two digits of a year.
    pub year: u8,
    /// Month number.
    pub month: u8,
    /// Patch release number.
    pub patch: Option<NonZero<u8>>,
}

impl VersionName {
    #[must_use]
    pub const fn patch(year: u8, month: u8, patch: u8) -> Self {
        let patch = match NonZero::<u8>::new(patch) {
            Some(patch) => Some(patch),
            None => panic!("patch number is zero"),
        };
        Self { year, month, patch }
    }
}

impl_serde_as_string!(VersionName);

/// Version suffix, used for internal builds only.
#[derive(EnumString, Display, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[strum(serialize_all = "kebab-case")]
pub enum InternalBuildSuffix {
    Rc,
    Nightly,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
}

impl Display for BosVersion {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{year}-{month:02}-{day:02}-{day_index}-{commit}-{version}",
            year = self.date.year(),
            month = self.date.month(),
            day = self.date.day(),
            day_index = self.day_index,
            commit = self.commit,
            version = self.version,
        )?;

        if self.is_plus {
            write!(f, "-plus")?;
        }

        if let Some(build) = self.build {
            write!(f, "-{build}")?;
        }

        Ok(())
    }
}

impl Display for VersionName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}.{:02}", self.year, self.month)?;

        if let Some(patch) = self.patch {
            write!(f, ".{patch}")?;
        }

        Ok(())
    }
}

impl FromStr for BosVersion {
    type Err = ParseVersionError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        fn inner(input: &str) -> Result<BosVersion, ErrorReason> {
            use ErrorReason::{
                InvalidBuildSuffix, InvalidDate, InvalidIntToken, InvalidVersionName, Missing,
                PatternMismatch,
            };

            static RE: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r"^(\d{4})?-(\d{2})?-(\d{2})?-(\d+)?-([0-9A-Fa-f]{8})?-([.0-9]+)?(-plus)?(?:-(\w+))?$").expect("hardcoded regex is invalid")
            });

            fn parse_int_token<T>(
                name: &'static str,
                token: Option<Match<'_>>,
                parser: impl Fn(&str) -> Result<T, ParseIntError>,
            ) -> Result<T, ErrorReason> {
                #[expect(clippy::redundant_closure)]
                token
                    .ok_or(Missing(name))?
                    .as_str()
                    .pipe(|x| parser(x))
                    .map_err(|e| InvalidIntToken(name, e))
            }

            let captures = RE.captures(input).ok_or(PatternMismatch)?;

            Ok(BosVersion {
                date: {
                    let year = parse_int_token("year", captures.get(1), str::parse)?;
                    let month = parse_int_token("month", captures.get(2), str::parse)?;
                    let day = parse_int_token("day", captures.get(3), str::parse)?;
                    NaiveDate::from_ymd_opt(year, month, day).ok_or(InvalidDate)?
                },
                day_index: parse_int_token("day index", captures.get(4), str::parse)?,
                commit: parse_int_token("commit", captures.get(5), str::parse)?,
                version: captures
                    .get(6)
                    .ok_or(Missing("version"))?
                    .as_str()
                    .parse::<VersionName>()
                    .map_err(InvalidVersionName)?,
                is_plus: captures.get(7).is_some(),
                build: captures
                    .get(8)
                    .map(|m| m.as_str().parse::<InternalBuildSuffix>())
                    .transpose()
                    .map_err(|_| InvalidBuildSuffix)?,
            })
        }

        inner(input).map_err(|reason| ParseVersionError {
            reason,
            invalid_version: input.to_owned(),
        })
    }
}

impl FromStr for VersionName {
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

#[derive(Error, Debug, Clone, Eq, PartialEq)]
#[error("invalid format: {invalid_version} ({reason})")]
pub struct ParseVersionError {
    pub reason: ErrorReason,
    pub invalid_version: String,
}

#[derive(Error, Debug, Clone, Eq, PartialEq)]
pub enum ErrorReason {
    #[error("pattern doesn't match")]
    PatternMismatch,
    #[error("'{0}' is missing")]
    Missing(&'static str),
    #[error("invalid {0}: {1}")]
    InvalidIntToken(&'static str, ParseIntError),
    #[error("invalid version name: {0}")]
    InvalidVersionName(ParseIntError),
    #[error("invalid date")]
    InvalidDate,
    #[error("invalid build suffix")]
    InvalidBuildSuffix,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use rand::seq::SliceRandom;
    use tap::Tap;

    #[test]
    fn version_name_ordering() {
        let sorted = [
            "22.02.2", "22.02.3", "22.02.4", "22.05", "22.08", "22.08.1", "23.01", "23.02",
        ]
        .map(|x| x.parse::<VersionName>().unwrap());

        let mut copy = sorted;
        let mut rng = StdRng::seed_from_u64(4);
        for _ in 0..1000 {
            copy.shuffle(&mut rng);
            copy.sort();
            assert_eq!(
                sorted.map(|x| x.to_string()),
                copy.map(|x| x.to_string()),
                "versions aren't sorted properly"
            );
        }
    }

    fn base() -> BosVersion {
        BosVersion {
            date: NaiveDate::from_ymd_opt(2022, 9, 27).unwrap(),
            day_index: 0,
            commit: CommitHashShort::from(0x06ba61b5),
            version: VersionName {
                year: 22,
                month: 8,
                patch: None,
            },
            is_plus: false,
            build: None,
        }
    }

    //
    // display
    //

    #[test]
    fn display_simple() {
        let expected = "2022-09-27-0-06ba61b5-22.08";
        let output = base().to_string();
        assert_eq!(expected, output);
    }

    #[test]
    fn display_patch() {
        let expected = "2022-09-27-0-06ba61b5-22.08.1";
        let output = base()
            .tap_mut(|x| x.version.patch = Some(NonZero::<u8>::new(1).unwrap()))
            .to_string();
        assert_eq!(expected, output);
    }

    #[test]
    fn display_plus() {
        let expected = "2022-09-27-0-06ba61b5-22.08-plus";
        let output = base().tap_mut(|x| x.is_plus = true).to_string();
        assert_eq!(expected, output);
    }

    #[test]
    fn display_nightly() {
        let expected = "2022-09-27-0-06ba61b5-22.08-nightly";
        let output = base()
            .tap_mut(|x| x.build = Some(InternalBuildSuffix::Nightly))
            .to_string();
        assert_eq!(expected, output);
    }

    #[test]
    fn display_plus_nightly() {
        let expected = "2022-09-27-0-06ba61b5-22.08-plus-nightly";
        let output = base()
            .tap_mut(|x| x.is_plus = true)
            .tap_mut(|x| x.build = Some(InternalBuildSuffix::Nightly))
            .to_string();
        assert_eq!(expected, output);
    }

    //
    // parse
    //

    #[test]
    fn parse_simple() {
        let expected = Ok(base());
        let output = "2022-09-27-0-06ba61b5-22.08".parse();
        assert_eq!(expected, output);
    }

    #[test]
    fn parse_patch() {
        let expected =
            Ok(base().tap_mut(|x| x.version.patch = Some(NonZero::<u8>::new(1).unwrap())));
        let output = "2022-09-27-0-06ba61b5-22.08.1".parse();
        assert_eq!(expected, output);
    }

    #[test]
    fn parse_plus() {
        let expected = Ok(base().tap_mut(|x| x.is_plus = true));
        let output = "2022-09-27-0-06ba61b5-22.08-plus".parse();
        assert_eq!(expected, output);
    }

    #[test]
    fn parse_nightly() {
        let expected = Ok(base().tap_mut(|x| x.build = Some(InternalBuildSuffix::Nightly)));
        let output = "2022-09-27-0-06ba61b5-22.08-nightly".parse();
        assert_eq!(expected, output);
    }

    #[test]
    fn parse_plus_nightly() {
        let expected = Ok(base()
            .tap_mut(|x| x.is_plus = true)
            .tap_mut(|x| x.build = Some(InternalBuildSuffix::Nightly)));
        let output = "2022-09-27-0-06ba61b5-22.08-plus-nightly".parse();
        assert_eq!(expected, output);
    }

    //
    // error
    //

    #[test]
    fn error_pattern_mismatch() {
        const TEXT: &str = "hello";
        let expected = Err(ParseVersionError {
            reason: ErrorReason::PatternMismatch,
            invalid_version: TEXT.to_owned(),
        });
        let output = TEXT.parse::<BosVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn error_missing() {
        const TEXT: &str = "2022-09-27-0--22.08";
        let expected = Err(ParseVersionError {
            reason: ErrorReason::Missing("commit"),
            invalid_version: TEXT.to_owned(),
        });
        let output = TEXT.parse::<BosVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn error_invalid_int_token() {
        const TEXT: &str = "2022-09-27-99999-06ba61b5-22.08";
        let expected = Err(ParseVersionError {
            reason: ErrorReason::InvalidIntToken("day index", "99999".parse::<u8>().err().unwrap()),
            invalid_version: TEXT.to_owned(),
        });
        let output = TEXT.parse::<BosVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn error_invalid_version_name() {
        const TEXT: &str = "2022-09-27-0-06ba61b5-22.08.1.1";
        let expected = Err(ParseVersionError {
            reason: ErrorReason::InvalidVersionName("1.1".parse::<u8>().err().unwrap()),
            invalid_version: TEXT.to_owned(),
        });
        let output = TEXT.parse::<BosVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn error_invalid_date() {
        const TEXT: &str = "2022-13-27-0-06ba61b5-22.08";
        let expected = Err(ParseVersionError {
            reason: ErrorReason::InvalidDate,
            invalid_version: TEXT.to_owned(),
        });
        let output = TEXT.parse::<BosVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn error_invalid_build_suffix() {
        const TEXT: &str = "2022-09-27-0-06ba61b5-22.08-banana";
        let expected = Err(ParseVersionError {
            reason: ErrorReason::InvalidBuildSuffix,
            invalid_version: TEXT.to_owned(),
        });
        let output = TEXT.parse::<BosVersion>();
        assert_eq!(expected, output);
    }

    #[test]
    fn error_invalid_build_suffix_plus() {
        const TEXT: &str = "2022-09-27-0-06ba61b5-22.08-plus-banana";
        let expected = Err(ParseVersionError {
            reason: ErrorReason::InvalidBuildSuffix,
            invalid_version: TEXT.to_owned(),
        });
        let output = TEXT.parse::<BosVersion>();
        assert_eq!(expected, output);
    }
}
