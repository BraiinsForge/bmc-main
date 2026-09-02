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

use bmc_wasm_sdk::{ElectricPower, Hashrate, MiningEfficiency, Ratio, Temperature, ufmt};

use crate::model::{Availability, MinerData, TemperatureRange};
use mining::gauge::TargetRange;

pub use mining::hashboards::JsonLookup;

// Each `parse_*` returns whether it stored any of its fields.
// A 2xx that yields no field is an unusable reply (unparsable body, or valid JSON of the wrong shape)
// the caller treats as a failed refresh rather than banking it fresh.
pub(crate) fn parse_details(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    let Some(uptime) = json
        .i64("/bosminer_uptime_s")
        .and_then(|v| u64::try_from(v).ok())
    else {
        return false;
    };
    data.uptime = Availability::Available(Duration::from_secs(uptime));
    true
}

pub(crate) fn parse_stats(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    let mut stored = false;
    if let Some(ghs) = json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second") {
        data.hashrate = Availability::Available(Hashrate::from_gigahashes_per_second(ghs));
        stored = true;
    }
    if let Some(power) = json.f64("/power_stats/approximated_consumption/watt") {
        data.power = Availability::Available(ElectricPower::from_watts(power));
        stored = true;
    }
    if let Some(efficiency) = json.f64("/power_stats/efficiency/joule_per_terahash") {
        data.efficiency =
            Availability::Available(MiningEfficiency::from_joules_per_terahash(efficiency));
        stored = true;
    }
    stored
}

pub(crate) fn parse_hashboards(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    let mut stored = false;
    let board = json.f64("/hashboards/0/board_temp/degree_c");
    let chip = json.f64("/hashboards/0/highest_chip_temp/temperature/degree_c");
    if let (Some(board_c), Some(chip_c)) = (board, chip) {
        data.temperature = Availability::Available(TemperatureRange {
            board: Temperature::from_celsius(board_c),
            chip: Temperature::from_celsius(chip_c),
        });
        stored = true;
    }
    let nominal = json.f64("/hashboards/0/stats/nominal_hashrate/gigahash_per_second");
    let real = json.f64("/hashboards/0/stats/real_hashrate/last_1m/gigahash_per_second");
    if let (Some(real), Some(nominal)) = (real, nominal)
        && nominal > 0.0
    {
        data.mcr = Availability::Available(Ratio::from_fraction(real / nominal));
        stored = true;
    }
    let summary = mining::hashboards::sum_chips(json);
    if let Some(model) = summary.model {
        data.chip_type = Availability::Available(model);
        stored = true;
    }
    if let Some(count) = summary.count {
        data.chip_count = Availability::Available(count);
        stored = true;
    }
    stored
}

pub(crate) fn parse_cooling(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    let Some(ratio) = json.f64("/fans/0/target_speed_ratio") else {
        return false;
    };
    data.fan_speed = Availability::Available(Ratio::from_fraction(ratio));
    true
}

pub(crate) fn parse_network(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    let Some(ip) = json.str("/networks/0/address") else {
        return false;
    };
    data.ip_address = Availability::Available(ip);
    true
}

pub(crate) fn parse_constraints(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    data.constraints.hashrate = target_range(
        json,
        "/tuner_constraints/hashrate_target",
        "terahash_per_second",
    );
    data.constraints.hashrate.is_some()
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
        let mut data = MinerData::default();
        parse_details(&json, &mut data);
        assert_eq!(
            data.uptime,
            Availability::Available(Duration::from_secs(187_020))
        );
    }

    #[test]
    fn parses_stats_efficiency_when_present() {
        let mut json = MapJson::default();
        json.floats
            .insert("/power_stats/efficiency/joule_per_terahash", 21.5);
        let mut data = MinerData::default();
        parse_stats(&json, &mut data);
        assert_eq!(
            data.efficiency,
            Availability::Available(MiningEfficiency::from_joules_per_terahash(21.5))
        );
    }

    #[test]
    fn leaves_efficiency_unavailable_when_absent() {
        let json = MapJson::default();
        let mut data = MinerData::default();
        parse_stats(&json, &mut data);
        assert_eq!(data.efficiency, Availability::Unavailable);
    }

    #[test]
    fn parse_reports_whether_it_stored_any_field() {
        // A shapeless 2xx (empty / wrong-shape JSON) stores nothing → false,
        // so the caller retries instead of banking it as a fresh success.
        let empty = MapJson::default();
        let mut data = MinerData::default();
        assert!(!parse_details(&empty, &mut data));
        assert!(!parse_stats(&empty, &mut data));
        assert!(!parse_hashboards(&empty, &mut data));
        assert!(!parse_cooling(&empty, &mut data));
        assert!(!parse_network(&empty, &mut data));
        assert!(!parse_constraints(&empty, &mut data));

        let mut json = MapJson::default();
        json.floats.insert(
            "/miner_stats/real_hashrate/last_1m/gigahash_per_second",
            122_480.0,
        );
        assert!(parse_stats(&json, &mut data));
    }

    #[test]
    fn parses_chip_model_and_count_for_single_board() {
        let mut json = MapJson::default();
        json.strings.insert("/hashboards/0/chip_type", "BM1370");
        json.ints.insert("/hashboards/0/chips_count", 108);
        let mut data = MinerData::default();
        parse_hashboards(&json, &mut data);
        assert_eq!(data.chip_type, Availability::Available("BM1370".into()));
        assert_eq!(data.chip_count, Availability::Available(108));
    }

    #[test]
    fn sums_chip_count_across_hashboards() {
        let mut json = MapJson::default();
        json.strings.insert("/hashboards/0/chip_type", "BM1370");
        json.ints.insert("/hashboards/0/chips_count", 108);
        json.ints.insert("/hashboards/1/chips_count", 108);
        json.ints.insert("/hashboards/2/chips_count", 108);
        let mut data = MinerData::default();
        parse_hashboards(&json, &mut data);
        assert_eq!(data.chip_type, Availability::Available("BM1370".into()));
        assert_eq!(data.chip_count, Availability::Available(324));
    }

    #[test]
    fn leaves_chips_unavailable_when_absent() {
        let json = MapJson::default();
        let mut data = MinerData::default();
        parse_hashboards(&json, &mut data);
        assert_eq!(data.chip_type, Availability::Unavailable);
        assert_eq!(data.chip_count, Availability::Unavailable);
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
        let mut data = MinerData::default();
        parse_constraints(&full_constraints_json(), &mut data);
        assert_eq!(
            data.constraints.hashrate,
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
        let mut data = MinerData::default();
        assert!(
            !parse_constraints(&json, &mut data),
            "a target missing one leaf stores nothing"
        );
        assert_eq!(data.constraints.hashrate, None);
    }

    #[test]
    fn parses_cooling_ratio_as_percent() {
        let mut json = MapJson::default();
        json.floats.insert("/fans/0/target_speed_ratio", 0.72);
        let mut data = MinerData::default();
        parse_cooling(&json, &mut data);
        let Availability::Available(fan_speed) = data.fan_speed else {
            panic!("BUG: fan speed should be available");
        };
        // The endpoint quotes a ratio; `Ratio` stores one, so nothing is scaled
        // on the way in and the reading still reads as a percent.
        assert!((fan_speed.as_percent() - 72.0).abs() < 1e-9);
    }
}
