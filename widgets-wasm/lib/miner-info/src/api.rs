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

use core::time::Duration;

use bmc_wasm_sdk::types::{ElectricPower, Hashrate, MiningEfficiency, Ratio, Temperature};
use bmc_wasm_sdk::ufmt;

use crate::model::{Availability, Constraints, MinerData, ParseResult, TemperatureRange, Verdict};
use mining::gauge::TargetRange;

pub use mining::hashboards::JsonLookup;

pub(crate) struct Details {
    pub uptime: Option<Duration>,
}

pub(crate) struct Stats {
    pub hashrate: Option<Hashrate>,
    pub power: Option<ElectricPower>,
    pub efficiency: Option<MiningEfficiency>,
}

pub(crate) struct Hashboards {
    pub temperature: Option<TemperatureRange>,
    pub mcr: Option<Ratio>,
    pub chip_type: Option<String>,
    pub chip_count: Option<usize>,
}

pub(crate) struct Cooling {
    pub fan_speed: Option<Ratio>,
}

pub(crate) struct Network {
    pub ip_address: Option<String>,
}

// `GetMinerDetailsResponse` is the only reply these read whose BOS+ schema
// requires a field: `bosminer_uptime_s` (bos-main `open/boser/openapi.json`).
// Everything else is optional, including empty `fans`, `hashboards`
// and `networks` arrays: absence is a miner reporting nothing, not a fault.
pub(crate) fn parse_details(json: &impl JsonLookup) -> ParseResult<Details> {
    let uptime = json
        .i64("/bosminer_uptime_s")
        .and_then(|secs| u64::try_from(secs).ok())
        .map(Duration::from_secs);
    ParseResult {
        data: Details { uptime },
        verdict: Verdict::from_reported(uptime.is_some()),
    }
}

pub(crate) fn parse_stats(json: &impl JsonLookup) -> ParseResult<Stats> {
    ParseResult {
        data: Stats {
            hashrate: json
                .f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second")
                .map(Hashrate::from_gigahashes_per_second),
            power: json
                .f64("/power_stats/approximated_consumption/watt")
                .map(ElectricPower::from_watts),
            efficiency: json
                .f64("/power_stats/efficiency/joule_per_terahash")
                .map(MiningEfficiency::from_joules_per_terahash),
        },
        verdict: Verdict::Answer,
    }
}

pub(crate) fn parse_hashboards(json: &impl JsonLookup) -> ParseResult<Hashboards> {
    let board = json.f64("/hashboards/0/board_temp/degree_c");
    let chip = json.f64("/hashboards/0/highest_chip_temp/temperature/degree_c");
    let nominal = json.f64("/hashboards/0/stats/nominal_hashrate/gigahash_per_second");
    let real = json.f64("/hashboards/0/stats/real_hashrate/last_1m/gigahash_per_second");
    let summary = mining::hashboards::sum_chips(json);
    ParseResult {
        data: Hashboards {
            temperature: board.zip(chip).map(|(board, chip)| TemperatureRange {
                board: Temperature::from_celsius(board),
                chip: Temperature::from_celsius(chip),
            }),
            mcr: real
                .zip(nominal)
                .filter(|(_, nominal)| *nominal > 0.0)
                .map(|(real, nominal)| Ratio::from_fraction(real / nominal)),
            chip_type: summary.model,
            chip_count: summary.count,
        },
        verdict: Verdict::Answer,
    }
}

pub(crate) fn parse_cooling(json: &impl JsonLookup) -> ParseResult<Cooling> {
    ParseResult {
        data: Cooling {
            fan_speed: json
                .f64("/fans/0/target_speed_ratio")
                .map(Ratio::from_fraction),
        },
        verdict: Verdict::Answer,
    }
}

pub(crate) fn parse_network(json: &impl JsonLookup) -> ParseResult<Network> {
    ParseResult {
        data: Network {
            ip_address: json.str("/networks/0/address"),
        },
        verdict: Verdict::Answer,
    }
}

pub(crate) fn parse_constraints(json: &impl JsonLookup) -> ParseResult<Constraints> {
    ParseResult {
        data: Constraints {
            hashrate: target_range(
                json,
                "/tuner_constraints/hashrate_target",
                "terahash_per_second",
            ),
        },
        verdict: Verdict::Answer,
    }
}

// Read a `{min,default,max}/<leaf>` target block, present only when all three
// edges are reported.
fn target_range(json: &impl JsonLookup, base: &str, leaf: &str) -> Option<TargetRange> {
    let edge = |name: &str| json.f64(&bmc_wasm_sdk::fmt!("{base}/{name}/{leaf}"));
    Some(TargetRange {
        min: edge("min")?,
        default: edge("default")?,
        max: edge("max")?,
    })
}

// A failed refresh does not come here — the engine keeps the last good reading
// and flags it stale; this clears where the miner's identity or credentials changed.
fn reset_details(data: &mut MinerData) {
    data.uptime = Availability::Unavailable;
}

fn reset_stats(data: &mut MinerData) {
    data.hashrate = Availability::Unavailable;
    data.power = Availability::Unavailable;
    data.efficiency = Availability::Unavailable;
}

fn reset_hashboards(data: &mut MinerData) {
    data.temperature = Availability::Unavailable;
    data.mcr = Availability::Unavailable;
    data.chip_type = Availability::Unavailable;
    data.chip_count = Availability::Unavailable;
}

fn reset_cooling(data: &mut MinerData) {
    data.fan_speed = Availability::Unavailable;
}

fn reset_network(data: &mut MinerData) {
    data.ip_address = Availability::Unavailable;
}

fn reset_constraints(data: &mut MinerData) {
    data.constraints = crate::model::Constraints::default();
}

pub(crate) fn reset_all(data: &mut MinerData) {
    reset_details(data);
    reset_stats(data);
    reset_hashboards(data);
    reset_cooling(data);
    reset_network(data);
    reset_constraints(data);
}

#[cfg(test)]
pub mod tests_support {
    use super::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub struct MapJson {
        pub strings: BTreeMap<&'static str, &'static str>,
        pub ints: BTreeMap<&'static str, i64>,
        pub floats: BTreeMap<&'static str, f64>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).map(|s| (*s).to_owned())
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::MapJson;
    use super::*;

    #[test]
    fn parses_miner_details_uptime() {
        let mut json = MapJson::default();
        json.ints.insert("/bosminer_uptime_s", 187_020);
        assert_eq!(
            parse_details(&json).data.uptime,
            Some(Duration::from_secs(187_020))
        );
    }

    #[test]
    fn parses_stats_efficiency_when_present() {
        let mut json = MapJson::default();
        json.floats
            .insert("/power_stats/efficiency/joule_per_terahash", 21.5);
        assert_eq!(
            parse_stats(&json).data.efficiency,
            Some(MiningEfficiency::from_joules_per_terahash(21.5))
        );
    }

    #[test]
    fn leaves_efficiency_absent_when_the_body_omits_it() {
        let json = MapJson::default();
        assert_eq!(parse_stats(&json).data.efficiency, None);
    }

    /// `bosminer_uptime_s` is the one field the BOS+ schema requires of a reply
    /// these parsers read, so it is the one absence that proves a broken body.
    #[test]
    fn only_a_details_body_without_its_uptime_is_unusable() {
        let empty = MapJson::default();
        assert_eq!(parse_details(&empty).verdict, Verdict::Unusable);

        let mut json = MapJson::default();
        json.ints.insert("/bosminer_uptime_s", 187_020);
        assert_eq!(parse_details(&json).verdict, Verdict::Answer);
    }

    /// Empty fan, hashboard and network arrays are legal, targets
    /// are optional, and every stats field is nullable.
    /// Failing the poll over that silence would stale a healthy miner.
    #[test]
    fn the_endpoints_whose_fields_are_all_optional_still_answer_when_empty() {
        let empty = MapJson::default();
        assert_eq!(parse_stats(&empty).verdict, Verdict::Answer);
        assert_eq!(parse_hashboards(&empty).verdict, Verdict::Answer);
        assert_eq!(parse_cooling(&empty).verdict, Verdict::Answer);
        assert_eq!(parse_network(&empty).verdict, Verdict::Answer);
        assert_eq!(parse_constraints(&empty).verdict, Verdict::Answer);
    }

    /// A parser hands back its endpoint's whole field set, so a dropped field
    /// reads as absent rather than as a reading from a minute ago.
    #[test]
    fn a_body_that_drops_a_field_reports_it_absent() {
        let mut hashrate_only = MapJson::default();
        hashrate_only.floats.insert(
            "/miner_stats/real_hashrate/last_1m/gigahash_per_second",
            122_480.0,
        );
        let parsed = parse_stats(&hashrate_only);
        assert_eq!(parsed.verdict, Verdict::Answer);
        assert!(parsed.data.hashrate.is_some());
        assert_eq!(parsed.data.power, None);
    }

    #[test]
    fn parses_chip_model_and_count_for_single_board() {
        let mut json = MapJson::default();
        json.strings.insert("/hashboards/0/chip_type", "BM1370");
        json.ints.insert("/hashboards/0/chips_count", 108);
        let parsed = parse_hashboards(&json);
        assert_eq!(parsed.data.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(parsed.data.chip_count, Some(108));
    }

    #[test]
    fn sums_chip_count_across_hashboards() {
        let mut json = MapJson::default();
        json.strings.insert("/hashboards/0/chip_type", "BM1370");
        json.ints.insert("/hashboards/0/chips_count", 108);
        json.ints.insert("/hashboards/1/chips_count", 108);
        json.ints.insert("/hashboards/2/chips_count", 108);
        let parsed = parse_hashboards(&json);
        assert_eq!(parsed.data.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(parsed.data.chip_count, Some(324));
    }

    #[test]
    fn leaves_chips_absent_when_the_body_omits_them() {
        let json = MapJson::default();
        let parsed = parse_hashboards(&json);
        assert_eq!(parsed.data.chip_type, None);
        assert_eq!(parsed.data.chip_count, None);
    }

    #[test]
    fn reset_clears_only_its_own_fields() {
        let mut data = MinerData {
            hashrate: Availability::Available(Hashrate::from_terahashes_per_second(4.0)),
            power: Availability::Available(ElectricPower::from_watts(120.0)),
            efficiency: Availability::Available(MiningEfficiency::from_joules_per_terahash(21.5)),
            mcr: Availability::Available(Ratio::from_percent(90.0)),
            fan_speed: Availability::Available(Ratio::from_percent(72.0)),
            ..MinerData::default()
        };
        reset_stats(&mut data);
        assert_eq!(data.hashrate, Availability::Unavailable);
        assert_eq!(data.power, Availability::Unavailable);
        assert_eq!(data.efficiency, Availability::Unavailable);
        // Fields owned by other endpoints are untouched.
        assert_eq!(data.mcr, Availability::Available(Ratio::from_percent(90.0)));
        assert_eq!(
            data.fan_speed,
            Availability::Available(Ratio::from_percent(72.0))
        );
    }

    #[test]
    fn reset_all_clears_stale_miner_values_after_auth_changes() {
        let mut data = MinerData {
            hashrate: Availability::Available(Hashrate::from_terahashes_per_second(4.0)),
            temperature: Availability::Available(TemperatureRange {
                board: Temperature::from_celsius(61.0),
                chip: Temperature::from_celsius(74.0),
            }),
            power: Availability::Available(ElectricPower::from_watts(120.0)),
            efficiency: Availability::Available(MiningEfficiency::from_joules_per_terahash(21.5)),
            mcr: Availability::Available(Ratio::from_percent(90.0)),
            fan_speed: Availability::Available(Ratio::from_percent(72.0)),
            uptime: Availability::Available(Duration::from_secs(187_020)),
            ip_address: Availability::Available("192.168.1.42".to_owned()),
            chip_type: Availability::Available("BM1370".into()),
            chip_count: Availability::Available(146),
            constraints: crate::model::Constraints {
                hashrate: Some(TargetRange {
                    min: 50.0,
                    default: 100.0,
                    max: 120.0,
                }),
            },
        };
        reset_all(&mut data);
        assert_eq!(data, MinerData::default());
    }

    fn full_constraints_json() -> MapJson {
        let mut json = MapJson::default();
        json.floats.insert(
            "/tuner_constraints/hashrate_target/min/terahash_per_second",
            50.0,
        );
        json.floats.insert(
            "/tuner_constraints/hashrate_target/default/terahash_per_second",
            100.0,
        );
        json.floats.insert(
            "/tuner_constraints/hashrate_target/max/terahash_per_second",
            120.0,
        );
        json
    }

    #[test]
    fn parses_the_tuner_hashrate_target() {
        assert_eq!(
            parse_constraints(&full_constraints_json()).data.hashrate,
            Some(TargetRange {
                min: 50.0,
                default: 100.0,
                max: 120.0
            })
        );
    }

    #[test]
    fn constraint_target_is_absent_when_a_leaf_is_missing() {
        let mut json = full_constraints_json();
        json.floats
            .remove("/tuner_constraints/hashrate_target/max/terahash_per_second");
        assert_eq!(
            parse_constraints(&json).data.hashrate,
            None,
            "a target missing one leaf is no target"
        );
    }

    #[test]
    fn parses_cooling_ratio_as_percent() {
        let mut json = MapJson::default();
        json.floats.insert("/fans/0/target_speed_ratio", 0.72);
        let Some(fan_speed) = parse_cooling(&json).data.fan_speed else {
            panic!("BUG: fan speed should be available");
        };
        // The endpoint quotes a ratio; `Ratio` stores one, so nothing is scaled
        // on the way in and the reading still reads as a percent.
        assert!((fan_speed.as_percent() - 72.0).abs() < 1e-9);
    }
}
