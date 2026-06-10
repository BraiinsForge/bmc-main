// Copyright (C) 2026  Braiins Systems s.r.o.

//! Selectable period and the Nexus window/candle tokens it maps to.

/// A selectable chart period offered by both ticker widgets. Variant names
/// avoid leading digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    D1,
    D7,
    D30,
}

impl Period {
    /// Parse the manifest wire value. `None` on any other string, so the caller
    /// can surface an input error.
    #[must_use]
    pub fn parse(s: &str) -> Option<Period> {
        match s {
            "1d" => Some(Period::D1),
            "7d" => Some(Period::D7),
            "30d" => Some(Period::D30),
            _ => None,
        }
    }

    /// Nexus window token.
    #[must_use]
    pub fn window(self) -> &'static str {
        match self {
            Period::D1 => "1d",
            Period::D7 => "7d",
            Period::D30 => "1mo",
        }
    }

    /// Nexus candle size for this period.
    #[must_use]
    pub fn candle(self) -> Candle {
        match self {
            Period::D1 => Candle::M15,
            Period::D7 => Candle::H1,
            Period::D30 => Candle::H4,
        }
    }

    /// Candle width for the market-open recency heuristic, kept separate from
    /// [`candle`](Self::candle) so the staleness window follows the market's
    /// natural update cadence rather than the chart's sample density.
    #[must_use]
    pub fn liveness(self) -> Candle {
        match self {
            Period::D1 => Candle::M15,
            Period::D7 => Candle::H1,
            Period::D30 => Candle::D1,
        }
    }
}

/// A candle size with its known bucket width in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Candle {
    M1,
    M15,
    H1,
    H4,
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
            Candle::H4 => "4h",
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
            Candle::H4 => 14_400,
            Candle::D1 => 86_400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_the_three_wire_values_and_rejects_others() {
        assert_eq!(Period::parse("1d"), Some(Period::D1));
        assert_eq!(Period::parse("7d"), Some(Period::D7));
        assert_eq!(Period::parse("30d"), Some(Period::D30));
        assert_eq!(Period::parse("24h"), None);
        assert_eq!(Period::parse(""), None);
        assert_eq!(Period::parse("7D"), None);
    }

    #[test]
    fn window_and_candle_tokens_match_nexus() {
        assert_eq!(Period::D1.window(), "1d");
        assert_eq!(Period::D7.window(), "7d");
        assert_eq!(Period::D30.window(), "1mo");
        assert_eq!(Period::D1.candle(), Candle::M15);
        assert_eq!(Period::D7.candle(), Candle::H1);
        assert_eq!(Period::D30.candle(), Candle::H4);
    }

    #[test]
    fn liveness_window_stays_coarse_for_the_month_view() {
        // The recency bucket is independent of the chart candle: the 30-day
        // view draws 4h candles but keeps a day-wide staleness window, so an
        // overnight-closed market is not mistaken for a dead feed.
        assert_eq!(Period::D1.liveness(), Candle::M15);
        assert_eq!(Period::D7.liveness(), Candle::H1);
        assert_eq!(Period::D30.liveness(), Candle::D1);
        assert_ne!(Period::D30.liveness(), Period::D30.candle());
    }

    #[test]
    fn candle_width_table_is_the_bucket_seconds() {
        assert_eq!(Candle::M1.width_secs(), 60);
        assert_eq!(Candle::M15.width_secs(), 900);
        assert_eq!(Candle::H1.width_secs(), 3_600);
        assert_eq!(Candle::H4.width_secs(), 14_400);
        assert_eq!(Candle::D1.width_secs(), 86_400);
        assert_eq!(Candle::M15.token(), "15m");
        assert_eq!(Candle::H1.token(), "1h");
        assert_eq!(Candle::H4.token(), "4h");
        assert_eq!(Candle::D1.token(), "1d");
        assert_eq!(Candle::M1.token(), "1m");
    }
}
