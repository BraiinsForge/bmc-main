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

use base64::prelude::{BASE64_STANDARD, Engine as _};
use bmc_wasm_sdk::types::{ElectricPower, Hashrate, Temperature};
use bmc_wasm_sdk::ufmt;

use crate::adapter::{DiscoveredDevice, FamilyAdapter};
use crate::device::{DeviceFamily, DeviceId, DeviceIdentity};
use crate::discovery::{JsonLookup, extract_endpoint};
use crate::model::ModelAccumulator;
use crate::telemetry::{DeviceTemp, TelemetryReading, hashrate, measurement};

// Legacy endpoint. Stale against the firmware's move to `/api/system/info`;
// retarget once that endpoint settles. See BDK-625.
const EP_INFO: &str = "/info";

pub const UBOS_TELEMETRY_ENDPOINTS: &[&str] = &[EP_INFO];

/// uBOS advertises the full service type `_ubos._tcp` (not a subtype of
/// `_http._tcp` like BOS), so every event on this browse is a uBOS device.
pub const UBOS_SERVICE_TYPES: &[&str] = &["_ubos._tcp"];

pub struct UbosAdapter;

impl FamilyAdapter for UbosAdapter {
    fn browse_service_types(&self) -> &'static [&'static str] {
        UBOS_SERVICE_TYPES
    }

    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::for_family(DeviceFamily::Ubos, &name),
                family: DeviceFamily::Ubos,
                name,
                host,
                port,
            },
            model_hint: None,
        })
    }

    fn api_base_path(&self) -> &'static str {
        "/api"
    }

    fn telemetry_endpoints(&self) -> &'static [&'static str] {
        UBOS_TELEMETRY_ENDPOINTS
    }

    // HTTP Basic auth from the operator-configured uBOS credentials;
    // the default `root:root` base64-encodes to `cm9vdDpyb290`.
    fn credential_header(&self, username: &str, password: &str) -> Option<String> {
        let encoded = BASE64_STANDARD.encode(bmc_wasm_sdk::fmt!("{}:{}", username, password));
        Some(bmc_wasm_sdk::fmt!("Authorization: Basic {}", encoded))
    }

    // No login endpoint, so a rejected Basic credential is a 401/403 on telemetry.
    fn is_auth_error(&self, status: u32) -> bool {
        status == 401 || status == 403
    }

    fn parse_telemetry(
        &self,
        endpoint: &str,
        json: &dyn JsonLookup,
        reading: &mut TelemetryReading,
    ) {
        self.reset_telemetry(endpoint, reading);
        if endpoint == EP_INFO {
            if let Some(hps) = json.f64("/hashrate") {
                reading.current_hashrate_ths = hashrate(Hashrate::from_hashes_per_second(hps));
            }
            if let Some(mw) = json.f64("/power_out_mw") {
                reading.power_w = measurement(ElectricPower::from_milliwatts(mw).as_watts());
            }
            // uBOS has one board sensor.
            if let Some(c) = json.f64("/temperature") {
                reading.temperature = Some(DeviceTemp::Single(Temperature::from_celsius(c)));
            }
            if let Some(uptime) = json.i64("/uptime").and_then(|v| u64::try_from(v).ok()) {
                reading.uptime_s = Some(uptime);
            }
        }
    }

    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading) {
        if endpoint == EP_INFO {
            reading.current_hashrate_ths = None;
            reading.power_w = None;
            reading.temperature = None;
            reading.uptime_s = None;
        }
    }

    fn parse_model(&self, endpoint: &str, json: &dyn JsonLookup, model: &mut ModelAccumulator) {
        // uBOS exposes no platform identifier, so the product name doubles as
        // the grouping id; setting both lets `ModelAccumulator::into_model`
        // yield a model, which requires id and name.
        if endpoint == EP_INFO
            && let Some(name) = json.str("/name").filter(|s| !s.is_empty())
        {
            model.id = Some(name.clone());
            model.name = Some(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::tests_support::MapJson;

    fn ubos_found() -> MapJson {
        let mut json = MapJson::default();
        json.strings.insert("/service_type", "_ubos._tcp.local.");
        json.strings.insert("/name", "bmm-01._ubos._tcp.local.");
        json.strings.insert("/host", "192.168.89.109");
        json.ints.insert("/port", 8080);
        json
    }

    #[test]
    fn browses_the_full_ubos_service_type() {
        assert_eq!(UbosAdapter.browse_service_types(), &["_ubos._tcp"]);
    }

    #[test]
    fn parses_a_ubos_device_and_stamps_family() {
        let found = UbosAdapter
            .parse_found(&ubos_found())
            .expect("BUG: device parsed");
        assert_eq!(found.identity.id.as_str(), "ubos/bmm-01._ubos._tcp.local.");
        assert_eq!(found.identity.host, "192.168.89.109");
        assert_eq!(found.identity.port, 8080);
        assert_eq!(found.identity.family, DeviceFamily::Ubos);
    }

    #[test]
    fn rejects_event_missing_port() {
        let mut json = ubos_found();
        json.ints.remove("/port");
        assert_eq!(UbosAdapter.parse_found(&json), None);
    }

    fn info_json() -> MapJson {
        let mut j = MapJson::default();
        j.floats.insert("/hashrate", 1_071_197_262_612.0);
        j.floats.insert("/power_out_mw", 35_000.0);
        j.floats.insert("/temperature", 59.0);
        j.ints.insert("/uptime", 382);
        j
    }

    #[test]
    fn parses_info_into_all_four_readings() {
        let mut r = TelemetryReading::default();
        UbosAdapter.parse_telemetry("/info", &info_json(), &mut r);
        let hr = r.current_hashrate_ths.expect("BUG: hashrate present");
        assert!((hr - 1.071_197_3).abs() < 1e-4, "got {hr}");
        assert_eq!(r.power_w, Some(35.0));
        assert_eq!(
            r.temperature,
            Some(DeviceTemp::Single(Temperature::from_celsius(59.0)))
        );
        assert_eq!(r.uptime_s, Some(382));
    }

    #[test]
    fn parse_clears_owned_field_that_vanished_from_response() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(99.0),
            power_w: Some(10.0),
            temperature: Some(DeviceTemp::Single(Temperature::from_celsius(50.0))),
            uptime_s: Some(123),
            ..TelemetryReading::default()
        };
        UbosAdapter.parse_telemetry("/info", &MapJson::default(), &mut r);
        assert_eq!(r, TelemetryReading::default());
    }

    #[test]
    fn reset_clears_the_endpoints_fields() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(1.0),
            power_w: Some(35.0),
            temperature: Some(DeviceTemp::Single(Temperature::from_celsius(59.0))),
            uptime_s: Some(382),
            nominal_hashrate_ths: Some(7.0),
            ..TelemetryReading::default()
        };
        UbosAdapter.reset_telemetry("/info", &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.temperature, None);
        assert_eq!(r.uptime_s, None);
        assert_eq!(r.nominal_hashrate_ths, Some(7.0));
    }

    #[test]
    fn credential_header_is_basic_root_root() {
        assert_eq!(
            UbosAdapter.credential_header("root", "root"),
            Some("Authorization: Basic cm9vdDpyb290".to_owned())
        );
    }

    #[test]
    fn credential_header_encodes_configured_credentials() {
        assert_eq!(
            UbosAdapter.credential_header("admin", "s3cret"),
            Some("Authorization: Basic YWRtaW46czNjcmV0".to_owned())
        );
    }

    #[test]
    fn parse_model_keys_model_by_name_so_it_materializes() {
        let mut j = MapJson::default();
        j.strings.insert("/name", "BMM Adapter W5500");
        let mut acc = ModelAccumulator::default();
        UbosAdapter.parse_model("/info", &j, &mut acc);
        assert_eq!(acc.id.as_deref(), Some("BMM Adapter W5500"));
        assert_eq!(acc.name.as_deref(), Some("BMM Adapter W5500"));
        assert_eq!(acc.chip_type, None);
        assert_eq!(acc.chip_count, None);

        let model = acc
            .into_model()
            .expect("BUG: uBOS model materializes from the name alone");
        assert_eq!(model.id, "BMM Adapter W5500");
        assert_eq!(model.name, "BMM Adapter W5500");
        assert_eq!(model.chip_type, None);
        assert_eq!(model.chip_count, None);
        assert_eq!(model.nominal_hashrate_ths, None);
    }

    #[test]
    fn recognizes_a_basic_auth_rejection() {
        assert!(UbosAdapter.is_auth_error(401));
        assert!(UbosAdapter.is_auth_error(403));
        assert!(!UbosAdapter.is_auth_error(200));
        assert!(!UbosAdapter.is_auth_error(503));
    }

    #[test]
    fn parse_model_ignores_empty_name() {
        let mut j = MapJson::default();
        j.strings.insert("/name", "");
        let mut acc = ModelAccumulator::default();
        UbosAdapter.parse_model("/info", &j, &mut acc);
        assert_eq!(acc.id, None);
        assert_eq!(acc.name, None);
        assert_eq!(acc.into_model(), None);
    }
}
