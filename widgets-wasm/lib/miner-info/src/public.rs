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

use bmc_wasm_sdk::{Hashvalue, Ratio, ufmt};

use crate::api::JsonLookup;
use crate::model::{Availability, Currency, Money, PublicData};

/// Blocks in a difficulty epoch, the denominator of the epoch progress.
const BLOCKS_PER_EPOCH: f64 = 2016.0;

pub(crate) fn parse_price_stats(json: &impl JsonLookup, currency: Currency, data: &mut PublicData) {
    if let Some(price) = json.f64("/price") {
        data.btc_price = Availability::Available(Money::new(price, currency));
    }
    // Quoted as a percent, unlike the adjustment fields below.
    if let Some(change) = json.f64("/percent_change_24h") {
        data.btc_change_24h = Availability::Available(Ratio::from_percent(change));
    }
}

pub(crate) fn parse_block(json: &impl JsonLookup, data: &mut PublicData) {
    if let Some(height) = json.i64("/0/height").and_then(|v| u64::try_from(v).ok()) {
        data.block_height = Availability::Available(height);
    }
}

pub(crate) fn parse_difficulty_stats(json: &impl JsonLookup, data: &mut PublicData) {
    // Both adjustments are quoted as fractions, which is what `Ratio` stores.
    if let Some(prev) = json.f64("/previous_adjustment") {
        data.prev_diff_adjust = Availability::Available(Ratio::from_fraction(prev));
    }
    if let Some(est) = json.f64("/estimated_adjustment") {
        data.est_diff_adjust = Availability::Available(Ratio::from_fraction(est));
    }
    if let Some(epoch) = json.f64("/block_epoch") {
        data.epoch_progress =
            Availability::Available(Ratio::from_fraction(epoch / BLOCKS_PER_EPOCH));
    }
}

pub(crate) fn parse_hashrate_stats(json: &impl JsonLookup, data: &mut PublicData) {
    // Quoted as a percent, unlike the adjustment fields.
    if let Some(fee_percent) = json.f64("/fees_percent") {
        data.avg_fee_share = Availability::Available(Ratio::from_percent(fee_percent));
    }
    if let Some(hashvalue) = json.f64("/hash_value") {
        data.hashvalue =
            Availability::Available(Hashvalue::from_bitcoin_per_terahash_day(hashvalue));
    }
}

#[must_use]
pub(crate) fn price_stats_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v1/price-stats?currency={}",
        currency.code()
    )
}

#[must_use]
pub(crate) fn block_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v2/blocks?limit=1&currency={}",
        currency.code()
    )
}

#[must_use]
pub(crate) fn difficulty_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v1/difficulty-stats?currency={}",
        currency.code()
    )
}

#[must_use]
pub(crate) fn hashrate_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v2/hashrate-stats?currency={}",
        currency.code()
    )
}

// Stop after this many samples even if the endpoint keeps returning points,
// so a malformed or unexpectedly long response can't grow the series unbounded.
const MAX_PRICE_HISTORY_POINTS: usize = 512;

// The series renders as a normalized sparkline, so its shape is the same
// in any fiat: the URL carries no currency and the endpoint is not refetched
// on a currency change.
#[must_use]
pub(crate) fn price_history_url(_currency: Currency) -> String {
    "https://public-api.braiins.com/v1/price-history?timeframe=1d".to_owned()
}

pub(crate) fn parse_price_history(json: &impl JsonLookup, data: &mut PublicData) {
    let mut points = Vec::new();
    for index in 0..MAX_PRICE_HISTORY_POINTS {
        let Some(price) = json.f64(&bmc_wasm_sdk::fmt!("/price/{}/y", index)) else {
            break;
        };
        points.push(price);
    }
    if !points.is_empty() {
        data.btc_price_history = points;
    }
}

pub(crate) fn reset_price_stats(data: &mut PublicData) {
    data.btc_price = Availability::Unavailable;
    data.btc_change_24h = Availability::Unavailable;
}

pub(crate) fn reset_block(data: &mut PublicData) {
    data.block_height = Availability::Unavailable;
}

pub(crate) fn reset_difficulty_stats(data: &mut PublicData) {
    data.prev_diff_adjust = Availability::Unavailable;
    data.est_diff_adjust = Availability::Unavailable;
    data.epoch_progress = Availability::Unavailable;
}

pub(crate) fn reset_hashrate_stats(data: &mut PublicData) {
    data.avg_fee_share = Availability::Unavailable;
    data.hashvalue = Availability::Unavailable;
}

pub(crate) fn reset_price_history(data: &mut PublicData) {
    data.btc_price_history.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests_support::MapJson;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn the_query_carries_the_currency_code() {
        assert!(price_stats_url(Currency::Usd).ends_with("currency=usd"));
    }

    #[test]
    fn parses_block_height() {
        let mut json = MapJson::default();
        json.ints.insert("/0/height", 900_123);
        let mut data = PublicData::default();
        parse_block(&json, &mut data);
        assert_eq!(data.block_height, Availability::Available(900_123));
    }

    /// The endpoint quotes the 24h change as a percent
    /// and both difficulty adjustments as fractions.
    /// `Ratio` stores fractions, so only the first is scaled
    /// — mixing the two up is a silent hundredfold error,
    /// and this is what catches it.
    #[test]
    fn percent_and_fraction_fields_land_on_the_same_scale() {
        let mut price = MapJson::default();
        price.floats.insert("/percent_change_24h", 6.25);
        let mut data = PublicData::default();
        parse_price_stats(&price, Currency::Usd, &mut data);
        let Availability::Available(change) = data.btc_change_24h else {
            panic!("BUG: the 24h change should be available");
        };
        assert!(approx(change.as_percent(), 6.25));

        let mut difficulty = MapJson::default();
        difficulty.floats.insert("/previous_adjustment", -0.045);
        difficulty.floats.insert("/estimated_adjustment", 0.105);
        parse_difficulty_stats(&difficulty, &mut data);
        let Availability::Available(prev) = data.prev_diff_adjust else {
            panic!("BUG: the previous adjustment should be available");
        };
        let Availability::Available(est) = data.est_diff_adjust else {
            panic!("BUG: the estimated adjustment should be available");
        };
        assert!(approx(prev.as_percent(), -4.5));
        assert!(approx(est.as_percent(), 10.5));
    }

    #[test]
    fn epoch_progress_is_the_block_count_over_the_epoch_length() {
        let mut json = MapJson::default();
        json.floats.insert("/block_epoch", 1008.0);
        let mut data = PublicData::default();
        parse_difficulty_stats(&json, &mut data);
        let Availability::Available(progress) = data.epoch_progress else {
            panic!("BUG: epoch progress should be available");
        };
        assert!(approx(progress.as_percent(), 50.0));
    }

    #[test]
    fn converts_hashvalue_bitcoin_to_satoshis() {
        let mut json = MapJson::default();
        json.floats.insert("/hash_value", 0.000_000_050_2);
        let mut data = PublicData::default();
        parse_hashrate_stats(&json, &mut data);
        let Availability::Available(value) = data.hashvalue else {
            panic!("BUG: hashvalue should be available");
        };
        assert!(approx(value.as_satoshis_per_terahash_day(), 5.02));
    }

    /// A parsed figure carries the currency it was asked for,
    /// so the symbol is read off the value rather than assumed.
    #[test]
    fn fiat_figures_carry_the_currency_they_were_fetched_in() {
        let mut json = MapJson::default();
        json.floats.insert("/price", 101_754.0);
        let mut data = PublicData::default();
        parse_price_stats(&json, Currency::Usd, &mut data);
        let Availability::Available(price) = data.btc_price else {
            panic!("BUG: BTC price should be available");
        };
        assert_eq!(price.currency, Currency::Usd);
    }

    #[test]
    fn collects_price_history_in_order_until_first_gap() {
        let mut json = MapJson::default();
        json.floats.insert("/price/0/y", 101_000.0);
        json.floats.insert("/price/1/y", 102_500.0);
        json.floats.insert("/price/2/y", 100_750.0);
        // index 3 missing on purpose: the series ends at the first gap
        json.floats.insert("/price/4/y", 999_999.0);
        let mut data = PublicData::default();
        parse_price_history(&json, &mut data);
        assert_eq!(
            data.btc_price_history,
            vec![101_000.0, 102_500.0, 100_750.0]
        );
    }

    #[test]
    fn reset_price_stats_clears_only_its_own_fields() {
        let mut data = PublicData {
            btc_price: Availability::Available(Money::new(104_250.0, Currency::Usd)),
            btc_change_24h: Availability::Available(Ratio::from_percent(1.82)),
            block_height: Availability::Available(900_123),
            ..PublicData::default()
        };
        reset_price_stats(&mut data);
        assert_eq!(data.btc_price, Availability::Unavailable);
        assert_eq!(data.btc_change_24h, Availability::Unavailable);
        assert_eq!(data.block_height, Availability::Available(900_123));
    }

    #[test]
    fn reset_hashrate_stats_clears_only_its_own_fields() {
        let mut data = PublicData {
            avg_fee_share: Availability::Available(Ratio::from_percent(1.4)),
            hashvalue: Availability::Available(Hashvalue::from_satoshis_per_terahash_day(5.02)),
            btc_price: Availability::Available(Money::new(104_250.0, Currency::Usd)),
            ..PublicData::default()
        };
        reset_hashrate_stats(&mut data);
        assert_eq!(data.avg_fee_share, Availability::Unavailable);
        assert_eq!(data.hashvalue, Availability::Unavailable);
        assert!(matches!(data.btc_price, Availability::Available(_)));
    }
}
