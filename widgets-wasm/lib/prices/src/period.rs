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

//! Selectable period and the Nexus window/candle tokens it maps to.

/// A selectable chart period offered by both ticker widgets: every Nexus
/// lookback window. Variant names avoid leading digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    H1,
    D1,
    D7,
    Mo1,
}

impl Period {
    /// Every period, in manifest order.
    pub const ALL: [Period; 4] = [Period::H1, Period::D1, Period::D7, Period::Mo1];

    /// Parse the manifest wire value (the Nexus window token, case-sensitive).
    /// `None` on any other string, so the caller can surface an input error.
    #[must_use]
    pub fn parse(s: &str) -> Option<Period> {
        Period::ALL.into_iter().find(|p| p.window() == s)
    }

    /// Nexus window token; also the manifest wire value.
    #[must_use]
    pub fn window(self) -> &'static str {
        match self {
            Period::H1 => "1h",
            Period::D1 => "1d",
            Period::D7 => "7d",
            Period::Mo1 => "1mo",
        }
    }

    /// Nexus candle size for this period. Short windows preserve intraday
    /// detail; one month and longer use daily-or-coarser samples.
    #[must_use]
    pub fn candle(self) -> Candle {
        match self {
            Period::H1 => Candle::M1,
            Period::D1 => Candle::M15,
            Period::D7 => Candle::H1,
            Period::Mo1 => Candle::D1,
        }
    }
}

/// A candle size with its known bucket width in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Candle {
    M1,
    M15,
    H1,
    D1,
}

impl Candle {
    /// Nexus candle token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Candle::M1 => "1m",
            Candle::M15 => "15m",
            Candle::H1 => "1h",
            Candle::D1 => "1d",
        }
    }

    /// Bucket width in seconds.
    #[must_use]
    pub fn width_secs(self) -> u64 {
        match self {
            Candle::M1 => 60,
            Candle::M15 => 900,
            Candle::H1 => 3_600,
            Candle::D1 => 86_400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_every_window_token() {
        // The manifest wire value is the Nexus window token verbatim, so
        // parse must accept exactly the tokens window() produces.
        for period in Period::ALL {
            assert_eq!(Period::parse(period.window()), Some(period));
        }
        assert_eq!(Period::ALL.len(), 4);
    }

    #[test]
    fn parse_rejects_legacy_and_near_miss_tokens() {
        // "30d" was the pre-rename manifest value; Nexus year tokens are
        // uppercase, so lowercase forms must not resolve.
        assert_eq!(Period::parse("30d"), None);
        assert_eq!(Period::parse("1y"), None);
        assert_eq!(Period::parse("24h"), None);
        assert_eq!(Period::parse(""), None);
        assert_eq!(Period::parse("7D"), None);
        assert_eq!(Period::parse("FULL"), None);
    }

    #[test]
    fn candle_sizes_match_period_sampling() {
        assert_eq!(Period::H1.candle(), Candle::M1);
        assert_eq!(Period::D1.candle(), Candle::M15);
        assert_eq!(Period::D7.candle(), Candle::H1);
        assert_eq!(Period::Mo1.candle(), Candle::D1);
    }

    #[test]
    fn candle_width_table_is_the_bucket_seconds() {
        assert_eq!(Candle::M1.width_secs(), 60);
        assert_eq!(Candle::M15.width_secs(), 900);
        assert_eq!(Candle::H1.width_secs(), 3_600);
        assert_eq!(Candle::D1.width_secs(), 86_400);
    }

    #[test]
    fn candle_tokens_match_nexus() {
        assert_eq!(Candle::M1.token(), "1m");
        assert_eq!(Candle::M15.token(), "15m");
        assert_eq!(Candle::H1.token(), "1h");
        assert_eq!(Candle::D1.token(), "1d");
    }
}
