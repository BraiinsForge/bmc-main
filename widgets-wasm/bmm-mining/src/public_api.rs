// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::miner_api::JsonLookup;
use crate::model::{Availability, Currency, Money, PublicData};

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
        data.btc_change_24h_percent = Availability::Available(change);
    }
}

pub(crate) fn parse_block(json: &impl JsonLookup, data: &mut PublicData) {
    if let Some(height) = json.i64("/0/height").and_then(|v| u64::try_from(v).ok()) {
        data.block_height = Availability::Available(height);
    }
}

pub(crate) fn parse_difficulty_stats(json: &impl JsonLookup, data: &mut PublicData) {
    if let Some(prev) = json.f64("/previous_adjustment") {
        data.prev_diff_adjust_percent = Availability::Available(prev * 100.0);
    }
    if let Some(est) = json.f64("/estimated_adjustment") {
        data.est_diff_adjust_percent = Availability::Available(est * 100.0);
    }
    if let Some(epoch) = json.f64("/block_epoch") {
        data.epoch_progress_percent = Availability::Available(epoch / 2016.0 * 100.0);
    }
}

pub(crate) fn parse_hashrate_stats(
    json: &impl JsonLookup,
    currency: Currency,
    data: &mut PublicData,
) {
    if let Some(hashrate) = json.f64("/current_hashrate") {
        data.network_hashrate_ehs = Availability::Available(hashrate);
    }
    if let Some(avg_fee) = json.f64("/avg_fees_per_block") {
        data.avg_fee_btc = Availability::Available(avg_fee);
    }
    if let Some(fee_percent) = json.f64("/fees_percent") {
        data.avg_fee_percent = Availability::Available(fee_percent);
    }
    if let Some(hashprice) = json.f64("/hash_price_currency") {
        data.hashprice = Availability::Available(Money {
            currency,
            value: hashprice,
        });
    }
    if let Some(hashvalue_btc) = json.f64("/hash_value") {
        data.hashvalue_sat_th_day = Availability::Available(hashvalue_btc * SAT_IN_BTC);
    }
}

pub(crate) fn price_stats_url(currency: Currency) -> String {
    format!(
        "https://public-api.braiins.com/v1/price-stats?currency={}",
        currency_code(currency)
    )
}

pub(crate) fn block_url(currency: Currency) -> String {
    format!(
        "https://public-api.braiins.com/v2/blocks?limit=1&currency={}",
        currency_code(currency)
    )
}

pub(crate) fn difficulty_url(currency: Currency) -> String {
    format!(
        "https://public-api.braiins.com/v1/difficulty-stats?currency={}",
        currency_code(currency)
    )
}

pub(crate) fn hashrate_url(currency: Currency) -> String {
    format!(
        "https://public-api.braiins.com/v2/hashrate-stats?currency={}",
        currency_code(currency)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::miner_api::tests_support::MapJson;

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
        json.floats.insert("/hash_value", 0.0000000502);
        let mut data = PublicData::default();
        parse_hashrate_stats(&json, Currency::Usd, &mut data);
        let Availability::Available(value) = data.hashvalue_sat_th_day else {
            panic!("BUG: hashvalue should be available");
        };
        assert!((value - 5.02).abs() < 1e-9);
    }
}
