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
use mining::hashboards::JsonLookup;
use units::availability::Availability;

use crate::model::{
    BitcoinData, DayHistory, DifficultyStats, Freshness, HASHES_PER_TERAHASH, HashrateStats,
    PriceStats, Resource, Series, TERAHASHES_PER_EXAHASH, TERAHASHES_PER_PETAHASH,
};

const BASE_URL: &str = "https://nexus.braiinsforge.com/api/v1/data/bitcoin";
const MAX_HISTORY_POINTS: usize = 512;

#[must_use]
pub fn url(resource: Resource) -> String {
    bmc_wasm_sdk::fmt!("{}/{}", BASE_URL, resource.name())
}

fn unsigned(json: &(impl JsonLookup + ?Sized), path: &str) -> Option<u64> {
    json.i64(path).and_then(|value| u64::try_from(value).ok())
}

fn finite(json: &(impl JsonLookup + ?Sized), path: &str) -> Option<f64> {
    json.f64(path).filter(|value| value.is_finite())
}

fn finite_before_scaling(
    json: &(impl JsonLookup + ?Sized),
    path: &str,
    multiplier: f64,
) -> Option<f64> {
    let value = finite(json, path)?;
    (value * multiplier).is_finite().then_some(value)
}

fn finite_hashrate_ehs(json: &(impl JsonLookup + ?Sized), path: &str) -> Option<f64> {
    let value = finite(json, path)?;
    let terahashes = value * TERAHASHES_PER_EXAHASH;
    (terahashes.is_finite() && (terahashes * HASHES_PER_TERAHASH).is_finite()).then_some(value)
}

fn freshness(json: &(impl JsonLookup + ?Sized), received_at_secs: i64) -> Option<Freshness> {
    let ttl_secs = unsigned(json, "/ttl_secs")?;
    if ttl_secs == 0 {
        return None;
    }
    let cache_age_secs = unsigned(json, "/cache_age_secs")?;
    let payload_unix_secs =
        Some(received_at_secs.saturating_sub(i64::try_from(cache_age_secs).unwrap_or(i64::MAX)));
    Some(Freshness {
        payload_unix_secs,
        ttl_secs,
    })
}

fn series(json: &(impl JsonLookup + ?Sized), prefix: &str) -> Option<Series> {
    let mut values = Vec::new();
    for index in 0..MAX_HISTORY_POINTS {
        let Some(value) = json.f64(&bmc_wasm_sdk::fmt!("{}/{}/y", prefix, index)) else {
            break;
        };
        if !value.is_finite() {
            return None;
        }
        values.push(value);
    }
    let min = values.iter().copied().reduce(f64::min)?;
    let max = values.iter().copied().reduce(f64::max)?;
    (max - min).is_finite().then_some(Series { values })
}

#[must_use]
pub fn parse(
    resource: Resource,
    json: &(impl JsonLookup + ?Sized),
    data: &mut BitcoinData,
    parse_date: &impl Fn(&str) -> Option<i64>,
    received_at_secs: i64,
) -> Option<Freshness> {
    let freshness = freshness(json, received_at_secs)?;
    match resource {
        Resource::Info => parse_info(json, data, parse_date)?,
        Resource::History => parse_history(json, data)?,
    }
    Some(freshness)
}

fn parse_info(
    json: &(impl JsonLookup + ?Sized),
    data: &mut BitcoinData,
    parse_date: &impl Fn(&str) -> Option<i64>,
) -> Option<()> {
    let price_stats = PriceStats {
        price: Some(finite(json, "/data/btc_price")?),
        change_24h_percent: Some(finite(json, "/data/btc_price_change_24h")?),
    };
    let difficulty_stats = DifficultyStats {
        difficulty: Some(finite(json, "/data/difficulty")?),
        previous_adjustment_percent: Some(
            finite_before_scaling(json, "/data/previous_adjustment", 100.0)? * 100.0,
        ),
        estimated_adjustment_percent: Some(
            finite_before_scaling(json, "/data/estimated_adjustment", 100.0)? * 100.0,
        ),
        estimated_adjustment_at: Some(
            json.str("/data/estimated_adjustment_date")
                .and_then(|value| parse_date(&value))?,
        ),
        epoch_block: Some(unsigned(json, "/data/blocks_this_epoch")?),
        epoch_block_time_secs: Some(unsigned(json, "/data/epoch_block_time")?),
    };
    let hashrate_stats = HashrateStats {
        current_ehs: Some(finite_hashrate_ehs(json, "/data/network_hashrate")?),
        avg_fees_btc: Some(finite(json, "/data/avg_fees_per_block")?),
        fees_percent: Some(finite(json, "/data/fees_percent")?),
        hashprice_per_th_day: Some(finite_before_scaling(
            json,
            "/data/hashprice",
            TERAHASHES_PER_PETAHASH,
        )?),
        revenue: Some(finite(json, "/data/total_mining_revenue")?),
    };
    let latest_block = unsigned(json, "/data/block_height")?;
    let blocks_24h = unsigned(json, "/data/blocks_last_24h")?;

    data.price_stats = Availability::Available(price_stats);
    data.difficulty_stats = Availability::Available(difficulty_stats);
    data.hashrate_stats = Availability::Available(hashrate_stats);
    data.latest_block = Availability::Available(latest_block);
    data.blocks_24h = Availability::Available(blocks_24h);
    Some(())
}

fn parse_history(json: &(impl JsonLookup + ?Sized), data: &mut BitcoinData) -> Option<()> {
    let price = series(json, "/data/btc_price")?;
    let hashrate = series(json, "/data/hashrate")?;
    let difficulty = series(json, "/data/difficulty")?;

    data.day_history = Availability::Available(DayHistory { price, hashrate });
    data.year_history = Availability::Available(difficulty);
    Some(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct MapJson {
        strings: BTreeMap<String, String>,
        ints: BTreeMap<String, i64>,
        floats: BTreeMap<String, f64>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).cloned()
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }

        fn has(&self, path: &str) -> bool {
            let under = |key: &String| key.starts_with(path);
            self.strings.keys().any(under)
                || self.ints.keys().any(under)
                || self.floats.keys().any(under)
        }
    }

    fn info_json() -> MapJson {
        let mut json = MapJson::default();
        for (path, value) in [
            ("/ttl_secs", 60),
            ("/cache_age_secs", 12),
            ("/data/block_height", 914_038),
            ("/data/blocks_this_epoch", 1_293),
            ("/data/epoch_block_time", 559),
            ("/data/blocks_last_24h", 151),
        ] {
            json.ints.insert(path.to_owned(), value);
        }
        for (path, value) in [
            ("/data/btc_price", 169_420.0),
            ("/data/btc_price_change_24h", 5.31),
            ("/data/difficulty", 129.7e12),
            ("/data/previous_adjustment", -0.0281),
            ("/data/estimated_adjustment", 0.104),
            ("/data/network_hashrate", 877.8),
            ("/data/hashprice", 0.0562),
            ("/data/avg_fees_per_block", 0.021),
            ("/data/fees_percent", 0.66),
            ("/data/total_mining_revenue", 49.35e6),
        ] {
            json.floats.insert(path.to_owned(), value);
        }
        json.strings.insert(
            "/data/estimated_adjustment_date".to_owned(),
            "2026-09-02T12:00:00Z".to_owned(),
        );
        json
    }

    fn history_json() -> MapJson {
        let mut json = MapJson::default();
        json.ints.insert("/ttl_secs".to_owned(), 600);
        json.ints.insert("/cache_age_secs".to_owned(), 5);
        for (series, values) in [
            ("btc_price", [1.0, 2.0]),
            ("hashrate", [3.0, 4.0]),
            ("difficulty", [5.0, 6.0]),
        ] {
            for (index, value) in values.into_iter().enumerate() {
                json.floats
                    .insert(bmc_wasm_sdk::fmt!("/data/{}/{}/y", series, index), value);
            }
        }
        json
    }

    #[test]
    fn urls_address_the_two_nexus_resources() {
        assert_eq!(
            url(Resource::Info),
            "https://nexus.braiinsforge.com/api/v1/data/bitcoin/mining-info"
        );
        assert_eq!(
            url(Resource::History),
            "https://nexus.braiinsforge.com/api/v1/data/bitcoin/mining-history"
        );
    }

    #[test]
    fn complete_info_is_stored_atomically() {
        let mut data = BitcoinData::default();
        let parsed = parse(
            Resource::Info,
            &info_json(),
            &mut data,
            &|value| (value == "2026-09-02T12:00:00Z").then_some(1_777_806_400),
            1_000,
        );
        assert_eq!(
            parsed,
            Some(Freshness {
                payload_unix_secs: Some(988),
                ttl_secs: 60,
            })
        );
        let stats = data
            .difficulty_stats
            .as_option()
            .expect("BUG: complete Nexus info was stored");
        assert_eq!(stats.previous_adjustment_percent, Some(-2.81));
        assert_eq!(stats.estimated_adjustment_percent, Some(10.4));
        assert_eq!(data.blocks_24h, Availability::Available(151));
    }

    #[test]
    fn incomplete_info_does_not_replace_any_field() {
        let mut json = info_json();
        json.floats.remove("/data/hashprice");
        let mut data = BitcoinData::default();
        assert_eq!(
            parse(Resource::Info, &json, &mut data, &|_| Some(0), 1_000),
            None
        );
        assert_eq!(data, BitcoinData::default());
    }

    #[test]
    fn missing_freshness_fields_do_not_replace_complete_info() {
        let mut data = BitcoinData::default();
        parse(Resource::Info, &info_json(), &mut data, &|_| Some(0), 1_000)
            .expect("BUG: complete Nexus info must seed the freshness rejection test");
        let complete = data.clone();

        for missing in ["/ttl_secs", "/cache_age_secs"] {
            let mut json = info_json();
            json.ints.remove(missing);
            json.floats.insert("/data/btc_price".to_owned(), 1.0);
            assert_eq!(
                parse(Resource::Info, &json, &mut data, &|_| Some(0), 1_000),
                None
            );
            assert_eq!(data, complete);
        }
    }

    #[test]
    fn non_finite_or_overflowing_info_does_not_replace_complete_info() {
        let mut data = BitcoinData::default();
        parse(Resource::Info, &info_json(), &mut data, &|_| Some(0), 1_000)
            .expect("BUG: complete Nexus info must seed the numeric rejection test");
        let complete = data.clone();

        for (path, value) in [
            ("/data/btc_price", f64::INFINITY),
            ("/data/previous_adjustment", f64::MAX),
            ("/data/network_hashrate", 1e300),
            ("/data/hashprice", f64::MAX),
        ] {
            let mut json = info_json();
            json.floats.insert(path.to_owned(), value);
            assert_eq!(
                parse(Resource::Info, &json, &mut data, &|_| Some(0), 1_000),
                None
            );
            assert_eq!(data, complete);
        }
    }

    #[test]
    fn history_requires_all_three_populated_series() {
        let mut json = history_json();
        let mut data = BitcoinData::default();
        assert!(parse(Resource::History, &json, &mut data, &|_| None, 1_000).is_some());
        assert_eq!(
            data.year_history
                .as_option()
                .expect("BUG: difficulty history was stored")
                .values,
            vec![5.0, 6.0]
        );

        json.floats.remove("/data/hashrate/0/y");
        let before = data.clone();
        assert_eq!(
            parse(Resource::History, &json, &mut data, &|_| None, 1_000),
            None
        );
        assert_eq!(data, before);
    }

    #[test]
    fn invalid_history_values_do_not_replace_complete_history() {
        let mut data = BitcoinData::default();
        parse(
            Resource::History,
            &history_json(),
            &mut data,
            &|_| None,
            1_000,
        )
        .expect("BUG: complete Nexus history must seed the numeric rejection test");
        let complete = data.clone();
        for values in [[f64::INFINITY, 4.0], [-f64::MAX, f64::MAX]] {
            let mut json = history_json();
            for (index, value) in values.into_iter().enumerate() {
                json.floats
                    .insert(bmc_wasm_sdk::fmt!("/data/hashrate/{}/y", index), value);
            }
            assert_eq!(
                parse(Resource::History, &json, &mut data, &|_| None, 1_000),
                None
            );
            assert_eq!(data, complete);
        }
    }
}
