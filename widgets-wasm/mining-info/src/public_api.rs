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

use bmc_wasm_sdk::ufmt;

use crate::miner_api::JsonLookup;
use crate::model::{Availability, Currency, Money, PublicData};
use units::units::{Btc, ExaHashPerSecond, Percent, SatPerTeraHashDay};

const SAT_IN_BTC: f64 = 100_000_000.0;

pub(crate) fn currency_code(currency: Currency) -> &'static str {
    match currency {
        Currency::Usd => "usd",
        Currency::Eur => "eur",
    }
}

pub(crate) fn parse_price_stats(json: &impl JsonLookup, currency: Currency, data: &mut PublicData) {
    if let Some(price) = json.f64("/price") {
        data.btc_price = Availability::Available(Money {
            currency,
            value: price,
        });
    }
    if let Some(change) = json.f64("/percent_change_24h") {
        data.btc_change_24h_percent = Availability::Available(Percent(change));
    }
}

pub(crate) fn parse_block(json: &impl JsonLookup, data: &mut PublicData) {
    if let Some(height) = json.i64("/0/height").and_then(|v| u64::try_from(v).ok()) {
        data.block_height = Availability::Available(height);
    }
}

pub(crate) fn parse_difficulty_stats(json: &impl JsonLookup, data: &mut PublicData) {
    if let Some(prev) = json.f64("/previous_adjustment") {
        data.prev_diff_adjust_percent = Availability::Available(Percent(prev * 100.0));
    }
    if let Some(est) = json.f64("/estimated_adjustment") {
        data.est_diff_adjust_percent = Availability::Available(Percent(est * 100.0));
    }
    if let Some(epoch) = json.f64("/block_epoch") {
        data.epoch_progress_percent = Availability::Available(Percent(epoch / 2016.0 * 100.0));
    }
}

pub(crate) fn parse_hashrate_stats(
    json: &impl JsonLookup,
    currency: Currency,
    data: &mut PublicData,
) {
    if let Some(hashrate) = json.f64("/current_hashrate") {
        data.network_hashrate_ehs = Availability::Available(ExaHashPerSecond(hashrate));
    }
    if let Some(avg_fee) = json.f64("/avg_fees_per_block") {
        data.avg_fee_btc = Availability::Available(Btc(avg_fee));
    }
    if let Some(fee_percent) = json.f64("/fees_percent") {
        data.avg_fee_percent = Availability::Available(Percent(fee_percent));
    }
    if let Some(hashprice) = json.f64("/hash_price_currency") {
        data.hashprice = Availability::Available(Money {
            currency,
            value: hashprice,
        });
    }
    if let Some(hashvalue_btc) = json.f64("/hash_value") {
        data.hashvalue_sat_th_day =
            Availability::Available(SatPerTeraHashDay(hashvalue_btc * SAT_IN_BTC));
    }
}

pub(crate) fn price_stats_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v1/price-stats?currency={}",
        currency_code(currency)
    )
}

pub(crate) fn block_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v2/blocks?limit=1&currency={}",
        currency_code(currency)
    )
}

pub(crate) fn difficulty_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v1/difficulty-stats?currency={}",
        currency_code(currency)
    )
}

pub(crate) fn hashrate_url(currency: Currency) -> String {
    bmc_wasm_sdk::fmt!(
        "https://public-api.braiins.com/v2/hashrate-stats?currency={}",
        currency_code(currency)
    )
}

// Stop after this many samples even if the endpoint keeps returning points, so a
// malformed or unexpectedly long response can't grow the series unbounded.
const MAX_PRICE_HISTORY_POINTS: usize = 512;

// The series renders as a normalized sparkline, so its shape is the same in any
// fiat: the URL carries no currency and the endpoint is not refetched on a
// currency change.
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
    data.btc_change_24h_percent = Availability::Unavailable;
}

pub(crate) fn reset_block(data: &mut PublicData) {
    data.block_height = Availability::Unavailable;
}

pub(crate) fn reset_difficulty_stats(data: &mut PublicData) {
    data.prev_diff_adjust_percent = Availability::Unavailable;
    data.est_diff_adjust_percent = Availability::Unavailable;
    data.epoch_progress_percent = Availability::Unavailable;
}

pub(crate) fn reset_hashrate_stats(data: &mut PublicData) {
    data.network_hashrate_ehs = Availability::Unavailable;
    data.avg_fee_btc = Availability::Unavailable;
    data.avg_fee_percent = Availability::Unavailable;
    data.hashprice = Availability::Unavailable;
    data.hashvalue_sat_th_day = Availability::Unavailable;
}

pub(crate) fn reset_price_history(data: &mut PublicData) {
    data.btc_price_history.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::miner_api::tests_support::MapJson;
    use units::units::Quantity;

    #[test]
    fn maps_currency_to_api_code() {
        assert_eq!(currency_code(Currency::Usd), "usd");
        assert_eq!(currency_code(Currency::Eur), "eur");
    }

    #[test]
    fn parses_block_height() {
        let mut json = MapJson::default();
        json.ints.insert("/0/height", 900_123);
        let mut data = PublicData::default();
        parse_block(&json, &mut data);
        assert_eq!(data.block_height, Availability::Available(900_123));
    }

    #[test]
    fn converts_hashvalue_btc_to_sat() {
        let mut json = MapJson::default();
        json.floats.insert("/hash_value", 0.000_000_050_2);
        let mut data = PublicData::default();
        parse_hashrate_stats(&json, Currency::Usd, &mut data);
        let Availability::Available(value) = data.hashvalue_sat_th_day else {
            panic!("BUG: hashvalue should be available");
        };
        assert!((value.raw() - 5.02).abs() < 1e-9);
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
    fn reset_price_stats_clears_stale_currency_values() {
        let mut data = PublicData {
            btc_price: Availability::Available(Money {
                currency: Currency::Usd,
                value: 104_250.0,
            }),
            btc_change_24h_percent: Availability::Available(Percent(1.82)),
            block_height: Availability::Available(900_123),
            ..PublicData::default()
        };
        reset_price_stats(&mut data);
        assert_eq!(data.btc_price, Availability::Unavailable);
        assert_eq!(data.btc_change_24h_percent, Availability::Unavailable);
        assert_eq!(data.block_height, Availability::Available(900_123));
    }

    #[test]
    fn reset_hashrate_stats_clears_stale_hashprice_currency_values() {
        let mut data = PublicData {
            network_hashrate_ehs: Availability::Available(ExaHashPerSecond(650.5)),
            avg_fee_btc: Availability::Available(Btc(0.125)),
            avg_fee_percent: Availability::Available(Percent(1.4)),
            hashprice: Availability::Available(Money {
                currency: Currency::Usd,
                value: 0.052,
            }),
            hashvalue_sat_th_day: Availability::Available(SatPerTeraHashDay(5.02)),
            btc_price: Availability::Available(Money {
                currency: Currency::Usd,
                value: 104_250.0,
            }),
            ..PublicData::default()
        };
        reset_hashrate_stats(&mut data);
        assert_eq!(data.network_hashrate_ehs, Availability::Unavailable);
        assert_eq!(data.avg_fee_btc, Availability::Unavailable);
        assert_eq!(data.avg_fee_percent, Availability::Unavailable);
        assert_eq!(data.hashprice, Availability::Unavailable);
        assert_eq!(data.hashvalue_sat_th_day, Availability::Unavailable);
        assert!(matches!(data.btc_price, Availability::Available(_)));
    }
}
