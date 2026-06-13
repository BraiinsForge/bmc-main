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

//! Nexus price-resource path building and HTTP-status classification. Host-pure.

use bmc_wasm_sdk::url::join_path_segments;

use crate::instrument::split_pair;
use crate::period::{Candle, Period};

/// Build the Nexus price URL with each instrument component as one path segment.
#[must_use]
pub fn prices_url(base: &str, symbol: &str, period: Period, candle: Candle) -> String {
    match split_pair(symbol) {
        Some((base_symbol, quote_symbol)) => join_path_segments(
            base,
            &[
                "prices",
                period.window(),
                candle.token(),
                base_symbol,
                quote_symbol,
            ],
        ),
        None => join_path_segments(base, &["prices", period.window(), candle.token(), symbol]),
    }
}

/// How an HTTP status folds into a fetch outcome class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchClass {
    Ok,
    InputError,
    /// The resource does not exist. Kept apart from [`FetchClass::InputError`]
    /// because Nexus also answers 404 for an instrument it knows
    /// but has no candles in the requested window.
    NotFound,
    Backoff,
    Transient,
}

impl FetchClass {
    #[must_use]
    pub const fn uses_poll_interval(self) -> bool {
        matches!(self, Self::InputError | Self::NotFound | Self::Backoff)
    }
}

/// Why a price reply carried nothing to draw. Only [`PriceMiss::NotFound`]
/// leaves a question the reference resource can answer, so a widget must keep
/// the two apart for as long as it shows the resulting placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceMiss {
    /// The request itself was refused; the instrument is not the issue.
    Rejected,
    /// No such resource — either the symbol is wrong or the window is empty.
    NotFound,
}

impl PriceMiss {
    #[must_use]
    pub const fn of(class: FetchClass) -> Option<Self> {
        match class {
            FetchClass::InputError => Some(Self::Rejected),
            FetchClass::NotFound => Some(Self::NotFound),
            FetchClass::Ok | FetchClass::Backoff | FetchClass::Transient => None,
        }
    }
}

#[must_use]
pub fn classify(status: u32) -> FetchClass {
    match status {
        200..=299 => FetchClass::Ok,
        400 => FetchClass::InputError,
        404 => FetchClass::NotFound,
        503 => FetchClass::Backoff,
        _ => FetchClass::Transient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_url_joins_base_and_keeps_pair_slash() {
        assert_eq!(
            prices_url(
                "https://nexus/api/v1/data/",
                "BTC-USD",
                Period::D7,
                Period::D7.candle()
            ),
            "https://nexus/api/v1/data/prices/7d/1h/BTC/USD"
        );
        assert_eq!(
            prices_url("b/", "BTC-USD", Period::D7, Candle::H4),
            "b/prices/7d/4h/BTC/USD"
        );
    }

    #[test]
    fn prices_url_percent_encodes_index_and_fx_specials() {
        assert_eq!(
            prices_url("b/", "^GSPC", Period::D7, Period::D7.candle()),
            "b/prices/7d/1h/%5EGSPC"
        );
        assert_eq!(
            prices_url("b/", "EURUSD=X", Period::D7, Period::D7.candle()),
            "b/prices/7d/1h/EURUSD%3DX"
        );
    }

    #[test]
    fn prices_url_keeps_a_configured_symbol_in_one_path_segment() {
        assert_eq!(
            prices_url("b/", "../../foo", Period::D7, Period::D7.candle()),
            "b/prices/7d/1h/..%2F..%2Ffoo"
        );
    }

    #[test]
    fn classify_maps_each_status_band() {
        assert_eq!(classify(200), FetchClass::Ok);
        assert_eq!(classify(204), FetchClass::Ok);
        assert_eq!(classify(400), FetchClass::InputError);
        assert_eq!(classify(404), FetchClass::NotFound);
        assert_eq!(classify(503), FetchClass::Backoff);
        assert_eq!(classify(500), FetchClass::Transient);
        assert_eq!(classify(0), FetchClass::Transient);
    }

    #[test]
    fn only_a_missing_resource_leaves_a_question_the_reference_can_answer() {
        // A refused request says nothing about the instrument, so no later
        // reference reply may reinterpret it as a closed market.
        assert_eq!(PriceMiss::of(classify(404)), Some(PriceMiss::NotFound));
        assert_eq!(PriceMiss::of(classify(400)), Some(PriceMiss::Rejected));
    }

    #[test]
    fn a_reply_that_carries_data_or_may_yet_is_not_a_miss() {
        for status in [200, 503, 500, 0] {
            assert_eq!(PriceMiss::of(classify(status)), None);
        }
    }

    #[test]
    fn input_errors_and_backoff_responses_use_the_poll_interval() {
        assert!(FetchClass::InputError.uses_poll_interval());
        assert!(FetchClass::NotFound.uses_poll_interval());
        assert!(FetchClass::Backoff.uses_poll_interval());
        assert!(!FetchClass::Ok.uses_poll_interval());
        assert!(!FetchClass::Transient.uses_poll_interval());
    }
}
