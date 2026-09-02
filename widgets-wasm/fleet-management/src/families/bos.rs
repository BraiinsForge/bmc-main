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

use bmc_wasm_sdk::{Hashrate, Temperature, ufmt};

use crate::adapter::{DiscoveredDevice, FamilyAdapter};
use crate::device::{DeviceFamily, DeviceId, DeviceIdentity};
use crate::discovery::{JsonLookup, extract_endpoint};
use crate::model::ModelAccumulator;
use crate::telemetry::{DeviceTemp, TelemetryReading, hashrate, measurement};

const EP_STATS: &str = mining::bos::STATS_PATH;
const EP_HASHBOARDS: &str = mining::bos::HASHBOARDS_PATH;
const EP_DETAILS: &str = mining::bos::DETAILS_PATH;

/// Upper bound on hashboard indices to scan. The JSON lookup exposes no array
/// length, so the loops probe a fixed range and skip gaps rather than stopping
/// at the first absent board (a failed or disabled board leaves a hole).
const MAX_HASHBOARDS: usize = 16;

pub const BOS_TELEMETRY_ENDPOINTS: &[&str] = &[EP_STATS, EP_HASHBOARDS, EP_DETAILS];

/// Map the BOS `Platform` enum integer (as serialized over REST) to its stable slug.
/// Mirrors `proto::Platform` in `ii-bos-plus-proto`;
/// an `Unspecified`/`0` or unrecognized value has no slug.
#[must_use]
fn platform_slug(platform: i64) -> Option<&'static str> {
    match platform {
        1 => Some("am1-s9"),
        2 => Some("am2-s17"),
        3 => Some("am3-bbb"),
        4 => Some("am3-aml"),
        5 => Some("stm32mp157c-ii1-am2"),
        6 => Some("cvitek-bm1-am2"),
        7 => Some("zynq-bm3-am2"),
        8 => Some("stm32mp157c-ii2-bmm1"),
        _ => None,
    }
}

/// BOS advertises `_http._tcp` with the `_bos` subtype.
/// Browsing the subtype directly means every event on this browse is a BOS device.
pub const BOS_SERVICE_TYPES: &[&str] = &["_bos._sub._http._tcp"];

/// Whether `GET /api/v1/version` answers like BOS+ — integer `{major, minor,
/// patch}`. Discovery fingerprints a base-type candidate with this predicate
/// before sending credentials. Spoofable — a benign-host filter (printers,
/// NAS), not a boundary against a host impersonating BOS.
#[must_use]
pub fn is_version_response(json: &dyn JsonLookup) -> bool {
    json.i64("/major").is_some() && json.i64("/minor").is_some() && json.i64("/patch").is_some()
}

pub struct BosAdapter;

impl FamilyAdapter for BosAdapter {
    fn browse_service_types(&self) -> &'static [&'static str] {
        BOS_SERVICE_TYPES
    }

    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::for_family(DeviceFamily::Bos, &name),
                family: DeviceFamily::Bos,
                name,
                host,
                port,
            },
            model_hint: None,
        })
    }

    fn api_base_path(&self) -> &'static str {
        "/api/v1"
    }

    fn telemetry_endpoints(&self) -> &'static [&'static str] {
        BOS_TELEMETRY_ENDPOINTS
    }

    fn auth_endpoint(&self) -> Option<&'static str> {
        Some(mining::bos::LOGIN_PATH)
    }

    fn login_body(&self, password: &str) -> String {
        mining::bos::login_body(password)
    }

    fn parse_login(&self, json: &dyn JsonLookup) -> Option<String> {
        mining::bos::parse_token(json)
    }

    fn is_auth_error(&self, status: u32) -> bool {
        status == 401 || status == 403
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "the tiny board count is exact in f64"
    )]
    fn parse_telemetry(
        &self,
        endpoint: &str,
        json: &dyn JsonLookup,
        reading: &mut TelemetryReading,
    ) {
        self.reset_telemetry(endpoint, reading);
        match endpoint {
            EP_STATS => {
                if let Some(ghps) =
                    json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second")
                {
                    reading.current_hashrate_ths =
                        hashrate(Hashrate::from_gigahashes_per_second(ghps));
                }
                if let Some(watt) = json.f64("/power_stats/approximated_consumption/watt") {
                    reading.power_w = measurement(watt);
                }
            }
            EP_HASHBOARDS => {
                // One temperature per hashboard.
                let mut min = f64::MAX;
                let mut max = f64::MIN;
                let mut sum = 0.0_f64;
                let mut count = 0_usize;
                for i in 0..MAX_HASHBOARDS {
                    let path = bmc_wasm_sdk::fmt!(
                        "/hashboards/{}/highest_chip_temp/temperature/degree_c",
                        i
                    );
                    if let Some(c) = json.f64(&path) {
                        min = min.min(c);
                        max = max.max(c);
                        sum += c;
                        count += 1;
                    }
                }
                reading.temperature = match count {
                    0 => None,
                    1 => Some(DeviceTemp::Single(Temperature::from_celsius(sum))),
                    _ => Some(DeviceTemp::Spread {
                        min: Temperature::from_celsius(min),
                        avg: Temperature::from_celsius(sum / count as f64),
                        max: Temperature::from_celsius(max),
                    }),
                };
            }
            EP_DETAILS => {
                if let Some(uptime) = json
                    .i64("/bosminer_uptime_s")
                    .and_then(|v| u64::try_from(v).ok())
                {
                    reading.uptime_s = Some(uptime);
                }
                if let Some(ghps) = json.f64("/sticker_hashrate/gigahash_per_second") {
                    reading.nominal_hashrate_ths =
                        hashrate(Hashrate::from_gigahashes_per_second(ghps));
                }
                reading.mac = json.str("/mac_address").filter(|s| !s.is_empty());
            }
            _ => {}
        }
    }

    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading) {
        match endpoint {
            EP_STATS => {
                reading.current_hashrate_ths = None;
                reading.power_w = None;
            }
            EP_HASHBOARDS => reading.temperature = None,
            EP_DETAILS => {
                reading.uptime_s = None;
                reading.nominal_hashrate_ths = None;
                reading.mac = None;
            }
            _ => {}
        }
    }

    fn parse_model(&self, endpoint: &str, json: &dyn JsonLookup, model: &mut ModelAccumulator) {
        match endpoint {
            EP_DETAILS => {
                if let Some(slug) = json.i64("/platform").and_then(platform_slug) {
                    model.id = Some(slug.to_owned());
                }
                if let Some(name) = json
                    .str("/miner_identity/miner_model")
                    .filter(|s| !s.is_empty())
                {
                    model.name = Some(name);
                }
            }
            EP_HASHBOARDS => {
                let summary = mining::hashboards::sum_chips(json);
                if model.chip_type.is_none() {
                    model.chip_type = summary.model;
                }
                if let Some(count) = summary.count {
                    model.chip_count = Some(count);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::tests_support::MapJson;
    use crate::telemetry::TelemetryReading;

    fn bos_shaped() -> MapJson {
        let mut json = MapJson::default();
        // A subtype browse delivers the BASE service type, not `_bos`.
        json.strings.insert("/service_type", "_http._tcp.local.");
        json.strings.insert("/name", "miner-a._http._tcp.local.");
        json.strings.insert("/host", "10.0.0.5");
        json.ints.insert("/port", 80);
        json
    }

    #[test]
    fn browses_the_bos_subtype() {
        assert_eq!(BosAdapter.browse_service_types(), &["_bos._sub._http._tcp"]);
    }

    #[test]
    fn parses_a_bos_device_from_a_found_event() {
        let found = BosAdapter
            .parse_found(&bos_shaped())
            .expect("BUG: device parsed");
        assert_eq!(found.identity.id.as_str(), "bos/miner-a._http._tcp.local.");
        assert_eq!(found.identity.host, "10.0.0.5");
        assert_eq!(found.identity.port, 80);
    }

    #[test]
    fn stamps_family_from_adapter_not_service_type() {
        let found = BosAdapter
            .parse_found(&bos_shaped())
            .expect("BUG: device parsed");
        assert_eq!(found.identity.family, DeviceFamily::Bos);
    }

    #[test]
    fn rejects_event_missing_host() {
        let mut json = bos_shaped();
        json.strings.remove("/host");
        assert_eq!(BosAdapter.parse_found(&json), None);
    }

    fn stats_json() -> MapJson {
        let mut j = MapJson::default();
        j.floats.insert(
            "/miner_stats/real_hashrate/last_1m/gigahash_per_second",
            122_480.0,
        );
        j.floats
            .insert("/power_stats/approximated_consumption/watt", 3_250.0);
        j
    }

    #[test]
    fn parses_stats_into_hashrate_and_power() {
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/stats", &stats_json(), &mut r);
        assert_eq!(r.current_hashrate_ths, Some(122.48));
        assert_eq!(r.power_w, Some(3_250.0));
    }

    #[test]
    fn parses_hashboards_temperature_as_a_spread_across_boards() {
        let mut j = MapJson::default();
        j.floats
            .insert("/hashboards/0/highest_chip_temp/temperature/degree_c", 61.0);
        j.floats
            .insert("/hashboards/1/highest_chip_temp/temperature/degree_c", 67.5);
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/hw/hashboards", &j, &mut r);
        assert_eq!(
            r.temperature,
            Some(DeviceTemp::Spread {
                min: Temperature::from_celsius(61.0),
                avg: Temperature::from_celsius(64.25),
                max: Temperature::from_celsius(67.5),
            }),
        );
    }

    #[test]
    fn hashboard_temperature_scans_past_a_missing_board() {
        // Board 0 failed (absent); boards 1 and 2 report. The spread must
        // come from the present boards, not stop at the gap on board 0.
        let mut j = MapJson::default();
        j.floats
            .insert("/hashboards/1/highest_chip_temp/temperature/degree_c", 61.0);
        j.floats
            .insert("/hashboards/2/highest_chip_temp/temperature/degree_c", 70.0);
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/hw/hashboards", &j, &mut r);
        assert_eq!(
            r.temperature,
            Some(DeviceTemp::Spread {
                min: Temperature::from_celsius(61.0),
                avg: Temperature::from_celsius(65.5),
                max: Temperature::from_celsius(70.0),
            }),
        );
    }

    #[test]
    fn parses_details_uptime_and_sticker_nominal() {
        let mut j = MapJson::default();
        j.ints.insert("/bosminer_uptime_s", 187_020);
        j.floats
            .insert("/sticker_hashrate/gigahash_per_second", 100_000.0);
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/details", &j, &mut r);
        assert_eq!(r.uptime_s, Some(187_020));
        assert_eq!(r.nominal_hashrate_ths, Some(100.0));
    }

    #[test]
    fn details_reset_clears_uptime_and_nominal() {
        let mut r = TelemetryReading {
            uptime_s: Some(100),
            nominal_hashrate_ths: Some(100.0),
            ..TelemetryReading::default()
        };
        BosAdapter.reset_telemetry("/miner/details", &mut r);
        assert_eq!(r.uptime_s, None);
        assert_eq!(r.nominal_hashrate_ths, None);
    }

    #[test]
    fn parse_clears_owned_field_that_vanished_from_response() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(99.0),
            power_w: Some(10.0),
            ..TelemetryReading::default()
        };
        BosAdapter.parse_telemetry("/miner/stats", &MapJson::default(), &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
    }

    #[test]
    fn reset_clears_only_the_endpoints_own_fields() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(50.0),
            power_w: Some(20.0),
            temperature: Some(DeviceTemp::Single(Temperature::from_celsius(60.0))),
            uptime_s: Some(100),
            ..TelemetryReading::default()
        };
        BosAdapter.reset_telemetry("/miner/stats", &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(
            r.temperature,
            Some(DeviceTemp::Single(Temperature::from_celsius(60.0)))
        );
        assert_eq!(r.uptime_s, Some(100));
    }

    #[test]
    fn parses_login_token() {
        let mut j = MapJson::default();
        j.strings.insert("/token", "abc123");
        assert_eq!(BosAdapter.parse_login(&j), Some("abc123".to_owned()));
    }

    #[test]
    fn login_without_token_is_none() {
        assert_eq!(BosAdapter.parse_login(&MapJson::default()), None);
    }

    #[test]
    fn flags_401_and_403_as_auth_errors() {
        assert!(BosAdapter.is_auth_error(401));
        assert!(BosAdapter.is_auth_error(403));
        assert!(!BosAdapter.is_auth_error(200));
        assert!(!BosAdapter.is_auth_error(500));
    }

    #[test]
    fn bos_advertises_a_login_endpoint() {
        assert_eq!(BosAdapter.auth_endpoint(), Some("/auth/login"));
    }

    #[test]
    fn bos_has_no_proactive_credential_header() {
        assert_eq!(BosAdapter.credential_header("root", "root"), None);
    }

    #[test]
    fn platform_slug_maps_every_known_platform() {
        assert_eq!(platform_slug(1), Some("am1-s9"));
        assert_eq!(platform_slug(2), Some("am2-s17"));
        assert_eq!(platform_slug(3), Some("am3-bbb"));
        assert_eq!(platform_slug(4), Some("am3-aml"));
        assert_eq!(platform_slug(5), Some("stm32mp157c-ii1-am2"));
        assert_eq!(platform_slug(6), Some("cvitek-bm1-am2"));
        assert_eq!(platform_slug(7), Some("zynq-bm3-am2"));
        assert_eq!(platform_slug(8), Some("stm32mp157c-ii2-bmm1"));
    }

    #[test]
    fn platform_slug_rejects_unspecified_and_unknown() {
        assert_eq!(platform_slug(0), None);
        assert_eq!(platform_slug(99), None);
    }

    #[test]
    fn parses_details_into_id_and_name() {
        let mut j = MapJson::default();
        j.ints.insert("/platform", 8);
        j.strings.insert("/miner_identity/miner_model", "BMM 101");
        let mut acc = ModelAccumulator::default();
        BosAdapter.parse_model("/miner/details", &j, &mut acc);
        assert_eq!(acc.id.as_deref(), Some("stm32mp157c-ii2-bmm1"));
        assert_eq!(acc.name.as_deref(), Some("BMM 101"));
    }

    #[test]
    fn details_without_miner_model_leaves_name_none() {
        let mut j = MapJson::default();
        j.ints.insert("/platform", 2);
        let mut acc = ModelAccumulator::default();
        BosAdapter.parse_model("/miner/details", &j, &mut acc);
        assert_eq!(acc.id.as_deref(), Some("am2-s17"));
        assert_eq!(acc.name, None);
    }

    #[test]
    fn parses_hashboards_into_chip_type_and_summed_count() {
        let mut j = MapJson::default();
        j.strings.insert("/hashboards/0/chip_type", "BM1370");
        j.ints.insert("/hashboards/0/chips_count", 76);
        j.strings.insert("/hashboards/1/chip_type", "BM1368");
        j.ints.insert("/hashboards/1/chips_count", 70);
        let mut acc = ModelAccumulator::default();
        BosAdapter.parse_model("/miner/hw/hashboards", &j, &mut acc);
        assert_eq!(acc.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(acc.chip_count, Some(146));
    }

    #[test]
    fn chip_count_sums_boards_past_a_missing_one() {
        // Board 0 omitted (failed/disabled), boards 1 and 2 present. The total
        // must include both present boards rather than stopping at the gap.
        let mut j = MapJson::default();
        j.strings.insert("/hashboards/1/chip_type", "BM1370");
        j.ints.insert("/hashboards/1/chips_count", 76);
        j.strings.insert("/hashboards/2/chip_type", "BM1370");
        j.ints.insert("/hashboards/2/chips_count", 70);
        let mut acc = ModelAccumulator::default();
        BosAdapter.parse_model("/miner/hw/hashboards", &j, &mut acc);
        assert_eq!(acc.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(acc.chip_count, Some(146));
    }
}
