// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::ufmt;

use crate::adapter::{DiscoveredDevice, FamilyAdapter};
use crate::device::{DeviceFamily, DeviceId, DeviceIdentity};
use crate::discovery::{JsonLookup, extract_endpoint};
use crate::telemetry::TelemetryReading;

const EP_STATS: &str = "/miner/stats";
const EP_HASHBOARDS: &str = "/miner/hw/hashboards";
const EP_DETAILS: &str = "/miner/details";

pub const BOS_TELEMETRY_ENDPOINTS: &[&str] = &[EP_STATS, EP_HASHBOARDS, EP_DETAILS];

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
}
