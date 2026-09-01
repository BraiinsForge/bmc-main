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

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the JsonLookup str/i64 methods are unused on the wasm build but kept for parity with the shared lookup trait"
    )
)]

use bmc_wasm_sdk::{Hashrate, ufmt};
use mining::gauge::TargetRange;

pub(crate) trait JsonLookup {
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "kept for parity with the shared lookup trait; the auth path consumes it later"
        )
    )]
    fn str(&self, path: &str) -> Option<String>;
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "kept for parity with the shared lookup trait; integer fields are consumed later"
        )
    )]
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
}

#[cfg(target_arch = "wasm32")]
pub(crate) use mining::bos::endpoint;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MinerData {
    pub(crate) hashrate_ths: Option<f64>,
    pub(crate) power_w: Option<f64>,
    pub(crate) constraints: Constraints,
}

// Tuner min/default/max targets that anchor the two gauge rings: `hashrate`
// drives the outer ring and the shared state, `power` the inner ring. Each is
// `Some` only when the endpoint reports all three of its leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Constraints {
    pub(crate) hashrate: Option<TargetRange>,
    pub(crate) power: Option<TargetRange>,
}

// Each `parse_*` returns whether it stored any field, so a 2xx
// that yields nothing (unparsable body / wrong shape) reads
// as a failed refresh instead of banking fresh.
pub(crate) fn parse_stats(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    let mut stored = false;
    if let Some(ghps) = json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second") {
        data.hashrate_ths =
            Some(Hashrate::from_gigahashes_per_second(ghps).as_terahashes_per_second());
        stored = true;
    }
    if let Some(power) = json.f64("/power_stats/approximated_consumption/watt") {
        data.power_w = Some(power);
        stored = true;
    }
    stored
}

pub(crate) fn parse_constraints(json: &impl JsonLookup, data: &mut MinerData) -> bool {
    data.constraints.hashrate = target_range(
        json,
        "/tuner_constraints/hashrate_target",
        "terahash_per_second",
    );
    data.constraints.power = target_range(json, "/tuner_constraints/power_target", "watt");
    data.constraints.hashrate.is_some() || data.constraints.power.is_some()
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

#[cfg(target_arch = "wasm32")]
impl JsonLookup for bmc_wasm_sdk::json::JsonDoc {
    fn str(&self, path: &str) -> Option<String> {
        self.str(path)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        self.i64(path)
    }

    fn f64(&self, path: &str) -> Option<f64> {
        self.f64(path)
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct MapJson {
        pub(crate) strings: BTreeMap<&'static str, &'static str>,
        pub(crate) ints: BTreeMap<&'static str, i64>,
        pub(crate) floats: BTreeMap<&'static str, f64>,
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
    fn parses_stats_hashrate_and_power() {
        let mut json = MapJson::default();
        json.floats.insert(
            "/miner_stats/real_hashrate/last_1m/gigahash_per_second",
            122_480.0,
        );
        json.floats
            .insert("/power_stats/approximated_consumption/watt", 41.0);
        let mut data = MinerData::default();
        assert!(parse_stats(&json, &mut data));
        assert_eq!(data.hashrate_ths, Some(122.48));
        assert_eq!(data.power_w, Some(41.0));
    }

    #[test]
    fn parse_reports_no_stored_field_on_shapeless_reply() {
        let empty = MapJson::default();
        let mut data = MinerData::default();
        assert!(!parse_stats(&empty, &mut data));
        assert!(!parse_constraints(&empty, &mut data));
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
        json.floats
            .insert("/tuner_constraints/power_target/min/watt", 1_000.0);
        json.floats
            .insert("/tuner_constraints/power_target/default/watt", 3_000.0);
        json.floats
            .insert("/tuner_constraints/power_target/max/watt", 3_500.0);
        json
    }

    #[test]
    fn parses_tuner_constraints_for_hashrate_and_power() {
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
        assert_eq!(
            data.constraints.power,
            Some(TargetRange {
                min: 1_000.0,
                default: 3_000.0,
                max: 3_500.0
            })
        );
    }

    #[test]
    fn constraint_target_is_absent_when_a_leaf_is_missing() {
        let mut json = full_constraints_json();
        json.floats
            .remove("/tuner_constraints/power_target/max/watt");
        let mut data = MinerData::default();
        parse_constraints(&json, &mut data);
        assert!(data.constraints.hashrate.is_some());
        assert_eq!(data.constraints.power, None);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AuthState {
    #[default]
    NoToken,
    LoggingIn,
    Authenticated(String),
    // A login attempt completed and was rejected. Distinct from `LoggingIn`; the
    // login poll keeps retrying underneath.
    Failed,
}

impl AuthState {
    pub(crate) fn token(&self) -> Option<&str> {
        match self {
            Self::Authenticated(token) => Some(token),
            Self::NoToken | Self::LoggingIn | Self::Failed => None,
        }
    }

    pub(crate) fn auth_header(&self) -> Option<String> {
        self.token()
            .map(|token| bmc_wasm_sdk::fmt!("Authorization: {token}"))
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn builds_bos_auth_header() {
        let mut auth = AuthState::default();
        assert_eq!(auth, AuthState::NoToken);
        assert_eq!(auth.auth_header(), None);
        assert_eq!(AuthState::LoggingIn.auth_header(), None);
        assert_eq!(AuthState::Failed.auth_header(), None);
        auth = AuthState::Authenticated("abc".to_owned());
        assert_eq!(auth.auth_header(), Some("Authorization: abc".to_owned()));
        auth = AuthState::NoToken;
        assert_eq!(auth.token(), None);
    }
}
