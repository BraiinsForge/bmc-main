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

//! The Home Assistant release a server reports in `auth_required`.

use core::str::FromStr;

/// The oldest release the client logs in to: 2024.3 added `climate.toggle`.
pub const MIN_VERSION: Version = Version {
    year: 2024,
    month: 3,
};

/// A Home Assistant release, named by the year and month its version starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub year: u16,
    pub month: u8,
}

impl Version {
    /// The release `version` belongs to, such as `2024.3.0`, `2024.3.0b1` or `2026.11.0.dev0`.
    /// Only the year and month are read, as the Home Assistant frontend compares versions,
    /// so a beta or a dev build counts as the release it leads to.
    /// `None` for anything else, such as the `0.118.5` of the releases before 2020.12.
    #[must_use]
    pub fn parse(version: &str) -> Option<Self> {
        let mut parts = version.split('.');
        let year = number(parts.next()?)?;
        let month = number(parts.next()?)?;
        (1..=12).contains(&month).then_some(Self { year, month })
    }
}

/// Digits only, since `parse` alone would also take a leading `+`.
fn number<T: FromStr>(text: &str) -> Option<T> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_release_beta_or_dev_build_is_read_as_its_year_and_month() {
        for version in [
            "2024.3.0",
            "2024.3.12",
            "2024.3.0b1",
            "2024.3.0.dev20240301",
        ] {
            assert_eq!(
                Version::parse(version),
                Some(Version {
                    year: 2024,
                    month: 3
                }),
                "{version}"
            );
        }
    }

    #[wasm_test]
    fn anything_but_a_year_and_a_month_is_unreadable() {
        for version in [
            "",
            "dev",
            "2024",
            "2024.",
            "0.118.5",
            "2024.13.0",
            "2024.0.0",
            "2024.x.0",
            "+2024.3.0",
            "2024.+3.0",
            "99999.3.0",
        ] {
            assert_eq!(Version::parse(version), None, "{version:?}");
        }
    }

    #[wasm_test]
    fn a_later_month_or_year_is_a_later_release() {
        let release = |year, month| Version { year, month };
        assert!(release(2024, 12) < release(2025, 1));
        assert!(release(2024, 12) > release(2024, 3));
    }
}
