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
    H3,
    H6,
    H12,
    D1,
    D3,
    D7,
    D14,
    Mo1,
    Mo3,
    Mo6,
    Mo9,
    Y1,
    Y2,
    Y3,
    Y5,
    Y10,
    Y25,
    Full,
}

impl Period {
    /// Every period, in manifest order.
    pub const ALL: [Period; 19] = [
        Period::H1,
        Period::H3,
        Period::H6,
        Period::H12,
        Period::D1,
        Period::D3,
        Period::D7,
        Period::D14,
        Period::Mo1,
        Period::Mo3,
        Period::Mo6,
        Period::Mo9,
        Period::Y1,
        Period::Y2,
        Period::Y3,
        Period::Y5,
        Period::Y10,
        Period::Y25,
        Period::Full,
    ];

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
            Period::H3 => "3h",
            Period::H6 => "6h",
            Period::H12 => "12h",
            Period::D1 => "1d",
            Period::D3 => "3d",
            Period::D7 => "7d",
            Period::D14 => "14d",
            Period::Mo1 => "1mo",
            Period::Mo3 => "3mo",
            Period::Mo6 => "6mo",
            Period::Mo9 => "9mo",
            Period::Y1 => "1Y",
            Period::Y2 => "2Y",
            Period::Y3 => "3Y",
            Period::Y5 => "5Y",
            Period::Y10 => "10Y",
            Period::Y25 => "25Y",
            Period::Full => "full",
        }
    }

    /// Nexus candle size for this period. Short windows preserve intraday
    /// detail; one month and longer use daily-or-coarser samples.
    #[must_use]
    pub fn candle(self) -> Candle {
        match self {
            Period::H1 | Period::H3 => Candle::M1,
            Period::H6 | Period::H12 => Candle::M5,
            Period::D1 => Candle::M15,
            Period::D3 => Candle::M30,
            Period::D7 | Period::D14 => Candle::H1,
            Period::Mo1 | Period::Mo3 | Period::Mo6 | Period::Mo9 | Period::Y1 => Candle::D1,
            Period::Y2 | Period::Y3 | Period::Y5 => Candle::W1,
            Period::Y10 | Period::Y25 | Period::Full => Candle::Mo1,
        }
    }

    /// Nexus candle size for the candlestick view: coarser than the
    /// sparkline mapping so candle bodies stay readable, preferring natural
    /// exchange sizes.
    #[must_use]
    pub fn candlestick_candle(self) -> Candle {
        match self {
            Period::H1 => Candle::M1,
            Period::H3 | Period::H6 => Candle::M5,
            Period::H12 | Period::D1 => Candle::M15,
            Period::D3 | Period::D7 => Candle::H1,
            Period::D14 => Candle::H4,
            Period::Mo1 | Period::Mo3 | Period::Mo6 | Period::Mo9 | Period::Y1 => Candle::D1,
            Period::Y2 | Period::Y3 | Period::Y5 => Candle::W1,
            Period::Y10 | Period::Y25 | Period::Full => Candle::Mo1,
        }
    }
}

/// A candle size with its known bucket width in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Candle {
    M1,
    M5,
    M15,
    M30,
    H1,
    H4,
    D1,
    W1,
    Mo1,
}

impl Candle {
    /// Nexus candle token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Candle::M1 => "1m",
            Candle::M5 => "5m",
            Candle::M15 => "15m",
            Candle::M30 => "30m",
            Candle::H1 => "1h",
            Candle::H4 => "4h",
            Candle::D1 => "1d",
            Candle::W1 => "1w",
            Candle::Mo1 => "1mo",
        }
    }

    /// Bucket width in seconds. One month is approximated as 30 days,
    /// matching the Nexus bucket size.
    #[must_use]
    pub fn width_secs(self) -> u64 {
        match self {
            Candle::M1 => 60,
            Candle::M5 => 300,
            Candle::M15 => 900,
            Candle::M30 => 1_800,
            Candle::H1 => 3_600,
            Candle::H4 => 14_400,
            Candle::D1 => 86_400,
            Candle::W1 => 604_800,
            Candle::Mo1 => 2_592_000,
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
        assert_eq!(Period::ALL.len(), 19);
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
        assert_eq!(Period::H3.candle(), Candle::M1);
        assert_eq!(Period::H6.candle(), Candle::M5);
        assert_eq!(Period::H12.candle(), Candle::M5);
        assert_eq!(Period::D1.candle(), Candle::M15);
        assert_eq!(Period::D3.candle(), Candle::M30);
        assert_eq!(Period::D7.candle(), Candle::H1);
        assert_eq!(Period::D14.candle(), Candle::H1);
        assert_eq!(Period::Mo1.candle(), Candle::D1);
        assert_eq!(Period::Mo3.candle(), Candle::D1);
        assert_eq!(Period::Mo6.candle(), Candle::D1);
        assert_eq!(Period::Mo9.candle(), Candle::D1);
        assert_eq!(Period::Y1.candle(), Candle::D1);
        assert_eq!(Period::Y2.candle(), Candle::W1);
        assert_eq!(Period::Y3.candle(), Candle::W1);
        assert_eq!(Period::Y5.candle(), Candle::W1);
        assert_eq!(Period::Y10.candle(), Candle::Mo1);
        assert_eq!(Period::Y25.candle(), Candle::Mo1);
        assert_eq!(Period::Full.candle(), Candle::Mo1);
    }

    #[test]
    fn candlestick_candles_follow_readable_resolution_boundaries() {
        // Use the finest natural size that fits the full-width plot,
        // or monthly when Nexus has no coarser size.
        assert_eq!(Period::H1.candlestick_candle(), Candle::M1);
        assert_eq!(Period::H3.candlestick_candle(), Candle::M5);
        assert_eq!(Period::H6.candlestick_candle(), Candle::M5);
        assert_eq!(Period::H12.candlestick_candle(), Candle::M15);
        assert_eq!(Period::D1.candlestick_candle(), Candle::M15);
        assert_eq!(Period::D3.candlestick_candle(), Candle::H1);
        assert_eq!(Period::D7.candlestick_candle(), Candle::H1);
        assert_eq!(Period::D14.candlestick_candle(), Candle::H4);
        assert_eq!(Period::Mo1.candlestick_candle(), Candle::D1);
        assert_eq!(Period::Mo3.candlestick_candle(), Candle::D1);
        assert_eq!(Period::Mo6.candlestick_candle(), Candle::D1);
        assert_eq!(Period::Mo9.candlestick_candle(), Candle::D1);
        assert_eq!(Period::Y1.candlestick_candle(), Candle::D1);
        assert_eq!(Period::Y2.candlestick_candle(), Candle::W1);
        assert_eq!(Period::Y3.candlestick_candle(), Candle::W1);
        assert_eq!(Period::Y5.candlestick_candle(), Candle::W1);
        assert_eq!(Period::Y10.candlestick_candle(), Candle::Mo1);
        assert_eq!(Period::Y25.candlestick_candle(), Candle::Mo1);
        assert_eq!(Period::Full.candlestick_candle(), Candle::Mo1);
    }

    #[test]
    fn candle_width_table_is_the_bucket_seconds() {
        assert_eq!(Candle::M1.width_secs(), 60);
        assert_eq!(Candle::M5.width_secs(), 300);
        assert_eq!(Candle::M15.width_secs(), 900);
        assert_eq!(Candle::M30.width_secs(), 1_800);
        assert_eq!(Candle::H1.width_secs(), 3_600);
        assert_eq!(Candle::H4.width_secs(), 14_400);
        assert_eq!(Candle::D1.width_secs(), 86_400);
        assert_eq!(Candle::W1.width_secs(), 604_800);
        assert_eq!(Candle::Mo1.width_secs(), 2_592_000);
    }

    #[test]
    fn candle_tokens_match_nexus() {
        assert_eq!(Candle::M1.token(), "1m");
        assert_eq!(Candle::M5.token(), "5m");
        assert_eq!(Candle::M15.token(), "15m");
        assert_eq!(Candle::M30.token(), "30m");
        assert_eq!(Candle::H1.token(), "1h");
        assert_eq!(Candle::H4.token(), "4h");
        assert_eq!(Candle::D1.token(), "1d");
        assert_eq!(Candle::W1.token(), "1w");
        assert_eq!(Candle::Mo1.token(), "1mo");
    }
}
