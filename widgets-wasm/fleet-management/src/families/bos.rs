// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::ufmt;

use crate::adapter::{DiscoveredDevice, FamilyAdapter};
use crate::device::{DeviceFamily, DeviceId, DeviceIdentity};
use crate::discovery::{JsonLookup, extract_endpoint};
use crate::model::ModelAccumulator;
use crate::telemetry::TelemetryReading;

const EP_STATS: &str = "/miner/stats";
const EP_HASHBOARDS: &str = "/miner/hw/hashboards";
const EP_DETAILS: &str = "/miner/details";

pub const BOS_TELEMETRY_ENDPOINTS: &[&str] = &[EP_STATS, EP_HASHBOARDS, EP_DETAILS];

/// Map the BOS `Platform` enum integer (as serialized over REST) to its
/// stable slug. Mirrors `proto::Platform` in `ii-bos-plus-proto`; an
/// `Unspecified`/`0` or unrecognized value has no slug.
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

#[expect(
    clippy::cast_possible_truncation,
    reason = "TH/s fits in f32 for realistic hashrates"
)]
fn ths_from_ghs(ghs: f64) -> f32 {
    (ghs / 1_000.0) as f32
}

/// BOS advertises `_http._tcp` with the `_bos` subtype. Browsing the subtype
/// directly means every event on this browse is a BOS device.
pub const BOS_SERVICE_TYPES: &[&str] = &["_bos._sub._http._tcp"];

pub struct BosAdapter;

impl FamilyAdapter for BosAdapter {
    fn browse_service_types(&self) -> &'static [&'static str] {
        BOS_SERVICE_TYPES
    }

    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::new(name.clone()),
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
        Some("/auth/login")
    }

    fn login_body(&self, password: &str) -> String {
        bmc_wasm_sdk::fmt!(
            r#"{{"username":"root","password":"{}"}}"#,
            bmc_wasm_sdk::JsonStr(password)
        )
    }

    fn parse_login(&self, json: &dyn JsonLookup) -> Option<String> {
        json.str("/token")
    }

    fn is_auth_error(&self, status: u32) -> bool {
        status == 401 || status == 403
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "sensor values fit in f32 for realistic readings"
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
                if let Some(ghs) =
                    json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second")
                {
                    reading.current_hashrate_ths = Some(ths_from_ghs(ghs));
                }
                if let Some(watt) = json.f64("/power_stats/approximated_consumption/watt") {
                    reading.power_w = Some(watt as f32);
                }
            }
            EP_HASHBOARDS => {
                let mut max: Option<f32> = None;
                let mut i = 0usize;
                loop {
                    let path = bmc_wasm_sdk::fmt!(
                        "/hashboards/{}/highest_chip_temp/temperature/degree_c",
                        i
                    );
                    match json.f64(&path) {
                        Some(c) => {
                            let c = c as f32;
                            max = Some(max.map_or(c, |m| m.max(c)));
                            i += 1;
                        }
                        None => break,
                    }
                }
                reading.temperature_c = max;
            }
            EP_DETAILS => {
                if let Some(uptime) = json
                    .i64("/bosminer_uptime_s")
                    .and_then(|v| u64::try_from(v).ok())
                {
                    reading.uptime_s = Some(uptime);
                }
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
            EP_HASHBOARDS => reading.temperature_c = None,
            EP_DETAILS => reading.uptime_s = None,
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
                let mut total: Option<u32> = None;
                let mut i = 0usize;
                loop {
                    let type_path = bmc_wasm_sdk::fmt!("/hashboards/{}/chip_type", i);
                    let count_path = bmc_wasm_sdk::fmt!("/hashboards/{}/chips_count", i);
                    let chip_type = json.str(&type_path).filter(|s| !s.is_empty());
                    let chips = json.i64(&count_path).and_then(|v| u32::try_from(v).ok());
                    // Stop at the first absent board. This assumes a populated
                    // board never reports both an absent chip type and count;
                    // BOS serializes both for every present hashboard.
                    if chip_type.is_none() && chips.is_none() {
                        break;
                    }
                    if model.chip_type.is_none() {
                        model.chip_type = chip_type;
                    }
                    if let Some(chips) = chips {
                        total = Some(total.unwrap_or(0).saturating_add(chips));
                    }
                    i += 1;
                }
                if total.is_some() {
                    model.chip_count = total;
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
            .expect("device parsed");
        assert_eq!(found.identity.id.as_str(), "miner-a._http._tcp.local.");
        assert_eq!(found.identity.host, "10.0.0.5");
        assert_eq!(found.identity.port, 80);
    }

    #[test]
    fn stamps_family_from_adapter_not_service_type() {
        let found = BosAdapter
            .parse_found(&bos_shaped())
            .expect("device parsed");
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
    fn parses_hashboards_temperature_as_max_chip_across_boards() {
        let mut j = MapJson::default();
        j.floats
            .insert("/hashboards/0/highest_chip_temp/temperature/degree_c", 61.0);
        j.floats
            .insert("/hashboards/1/highest_chip_temp/temperature/degree_c", 67.5);
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/hw/hashboards", &j, &mut r);
        assert_eq!(r.temperature_c, Some(67.5));
    }

    #[test]
    fn parses_details_uptime() {
        let mut j = MapJson::default();
        j.ints.insert("/bosminer_uptime_s", 187_020);
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/details", &j, &mut r);
        assert_eq!(r.uptime_s, Some(187_020));
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
            temperature_c: Some(60.0),
            uptime_s: Some(100),
            nominal_hashrate_ths: None,
        };
        BosAdapter.reset_telemetry("/miner/stats", &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.temperature_c, Some(60.0));
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
        assert_eq!(BosAdapter.credential_header(), None);
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
}
