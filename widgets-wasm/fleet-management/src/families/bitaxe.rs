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
use crate::model::{MinerModel, ModelAccumulator};
use crate::telemetry::{DeviceTemp, TelemetryReading, hashrate, measurement};

const EP_INFO: &str = "/info";

pub const BITAXE_TELEMETRY_ENDPOINTS: &[&str] = &[EP_INFO];
pub const BITAXE_SERVICE_TYPES: &[&str] = &["_axeos._sub._http._tcp"];

pub struct BitaxeAdapter;

fn non_empty(json: &dyn JsonLookup, path: &str) -> Option<String> {
    json.str(path).filter(|s| !s.is_empty())
}

fn txt_model_hint(json: &dyn JsonLookup) -> Option<MinerModel> {
    let family = non_empty(json, "/txt/family");
    let board = non_empty(json, "/txt/board");
    let (id, name) = match (family.as_deref(), board.as_deref()) {
        (Some(family), Some(board)) => (
            bmc_wasm_sdk::fmt!("axeos:{family}:{board}"),
            bmc_wasm_sdk::fmt!("Bitaxe {family} {board}"),
        ),
        (None, Some(board)) => (
            bmc_wasm_sdk::fmt!("axeos:{board}"),
            bmc_wasm_sdk::fmt!("Bitaxe board {board}"),
        ),
        (Some(family), None) => (
            bmc_wasm_sdk::fmt!("axeos:{family}"),
            bmc_wasm_sdk::fmt!("Bitaxe {family}"),
        ),
        (None, None) => return None,
    };
    let chip_count = non_empty(json, "/txt/asic_count").and_then(|s| s.parse::<usize>().ok());
    Some(MinerModel {
        id,
        name,
        chip_type: non_empty(json, "/txt/asic"),
        chip_count,
        nominal_hashrate_ths: None,
    })
}

impl FamilyAdapter for BitaxeAdapter {
    fn browse_service_types(&self) -> &'static [&'static str] {
        BITAXE_SERVICE_TYPES
    }

    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::for_family(DeviceFamily::Bitaxe, &name),
                family: DeviceFamily::Bitaxe,
                name,
                host,
                port,
            },
            model_hint: txt_model_hint(json),
        })
    }

    fn api_base_path(&self) -> &'static str {
        "/api/system"
    }

    fn telemetry_endpoints(&self) -> &'static [&'static str] {
        BITAXE_TELEMETRY_ENDPOINTS
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "the sensor count is exact in f64"
    )]
    fn parse_telemetry(
        &self,
        endpoint: &str,
        json: &dyn JsonLookup,
        reading: &mut TelemetryReading,
    ) {
        self.reset_telemetry(endpoint, reading);
        if endpoint == EP_INFO {
            // AxeOS reports -1 for a sensor it has not yet read, notably right
            // after boot ("not yet known"); `measurement` drops that sentinel
            // with the rest of the unusable figures, so it never reaches a total.
            if let Some(ghps) = json.f64("/hashRate") {
                reading.current_hashrate_ths = hashrate(Hashrate::from_gigahashes_per_second(ghps));
            }
            if let Some(ghps) = json.f64("/expectedHashrate") {
                reading.nominal_hashrate_ths = hashrate(Hashrate::from_gigahashes_per_second(ghps));
            }
            if let Some(watts) = json.f64("/power") {
                reading.power_w = measurement(watts);
            }
            // ASIC temp, plus temp2 on multi-sensor boards.
            let mut min = f64::MAX;
            let mut max = f64::MIN;
            let mut sum = 0.0_f64;
            let mut count = 0_usize;
            for path in ["/temp", "/temp2"] {
                if let Some(c) = json.f64(path).filter(|v| *v >= 0.0) {
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
            reading.mac = non_empty(json, "/macAddr");
            if let Some(uptime) = json
                .i64("/uptimeSeconds")
                .and_then(|v| u64::try_from(v).ok())
            {
                reading.uptime_s = Some(uptime);
            }
        }
    }

    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading) {
        if endpoint == EP_INFO {
            reading.current_hashrate_ths = None;
            reading.nominal_hashrate_ths = None;
            reading.power_w = None;
            reading.temperature = None;
            reading.uptime_s = None;
            reading.mac = None;
        }
    }

    fn parse_model(&self, endpoint: &str, json: &dyn JsonLookup, model: &mut ModelAccumulator) {
        if endpoint != EP_INFO {
            return;
        }
        if let Some(device_model) = non_empty(json, "/deviceModel") {
            model.id = Some(device_model.clone());
            model.name = Some(device_model);
        } else if let Some(board) = non_empty(json, "/boardVersion") {
            model.id = Some(bmc_wasm_sdk::fmt!("axeos-board:{board}"));
            model.name = Some(bmc_wasm_sdk::fmt!("Bitaxe board {board}"));
        }
        if let Some(asic) = non_empty(json, "/ASICModel") {
            model.chip_type = Some(asic);
        }
        if let Some(chip_count) = json.i64("/asicCount").and_then(|v| usize::try_from(v).ok()) {
            model.chip_count = Some(chip_count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::tests_support::MapJson;

    fn axeos_found() -> MapJson {
        let mut json = MapJson::default();
        json.strings.insert("/service_type", "_http._tcp.local.");
        json.strings
            .insert("/name", "Bitaxe Gamma 602 (A1B2)._http._tcp.local.");
        json.strings.insert("/host", "192.168.1.42");
        json.ints.insert("/port", 80);
        json.strings.insert("/txt/family", "Gamma");
        json.strings.insert("/txt/board", "602");
        json.strings.insert("/txt/asic", "BM1370");
        json.strings.insert("/txt/asic_count", "1");
        json
    }

    #[test]
    fn browses_the_axeos_subtype() {
        assert_eq!(
            BitaxeAdapter.browse_service_types(),
            &["_axeos._sub._http._tcp"]
        );
    }

    #[test]
    fn parses_a_bitaxe_device_and_stamps_family() {
        let found = BitaxeAdapter
            .parse_found(&axeos_found())
            .expect("BUG: valid AxeOS discovery event must parse");
        assert_eq!(
            found.identity.id.as_str(),
            "bitaxe/Bitaxe Gamma 602 (A1B2)._http._tcp.local."
        );
        assert_eq!(found.identity.host, "192.168.1.42");
        assert_eq!(found.identity.port, 80);
        assert_eq!(found.identity.family, DeviceFamily::Bitaxe);
    }

    #[test]
    fn rejects_event_missing_host() {
        let mut json = axeos_found();
        json.strings.remove("/host");
        assert_eq!(BitaxeAdapter.parse_found(&json), None);
    }

    #[test]
    fn rejects_event_missing_port() {
        let mut json = axeos_found();
        json.ints.remove("/port");
        assert_eq!(BitaxeAdapter.parse_found(&json), None);
    }

    #[test]
    fn parses_txt_model_hint_from_discovery() {
        let found = BitaxeAdapter
            .parse_found(&axeos_found())
            .expect("BUG: valid AxeOS discovery event must parse");
        let model = found
            .model_hint
            .expect("BUG: TXT family and board must create a model hint");
        assert_eq!(model.id, "axeos:Gamma:602");
        assert_eq!(model.name, "Bitaxe Gamma 602");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, Some(1));
        assert_eq!(model.nominal_hashrate_ths, None);
    }

    #[test]
    fn malformed_txt_omits_only_model_hint() {
        let mut json = axeos_found();
        json.strings.remove("/txt/family");
        json.strings.remove("/txt/board");
        json.strings.insert("/txt/asic_count", "not-a-number");
        let found = BitaxeAdapter
            .parse_found(&json)
            .expect("BUG: malformed TXT must not reject a valid endpoint");
        assert_eq!(found.identity.host, "192.168.1.42");
        assert_eq!(found.model_hint, None);
    }

    fn info_json() -> MapJson {
        let mut j = MapJson::default();
        j.floats.insert("/hashRate", 1_071.197_262_612);
        j.floats.insert("/expectedHashrate", 1_200.0);
        j.floats.insert("/power", 35.5);
        j.floats.insert("/temp", 59.0);
        j.ints.insert("/uptimeSeconds", 382);
        j
    }

    #[test]
    fn parses_system_info_into_readings() {
        let mut r = TelemetryReading::default();
        BitaxeAdapter.parse_telemetry("/info", &info_json(), &mut r);
        let hr = r
            .current_hashrate_ths
            .expect("BUG: hashRate must produce current hashrate");
        assert!((hr - 1.071_197_3).abs() < 1e-4, "got {hr}");
        let nominal = r
            .nominal_hashrate_ths
            .expect("BUG: expectedHashrate must produce nominal");
        assert!((nominal - 1.2).abs() < 1e-4, "got {nominal}");
        assert_eq!(r.power_w, Some(35.5));
        assert_eq!(
            r.temperature,
            Some(DeviceTemp::Single(Temperature::from_celsius(59.0)))
        );
        assert_eq!(r.uptime_s, Some(382));
    }

    #[test]
    fn negative_uptime_is_ignored() {
        let mut j = info_json();
        j.ints.insert("/uptimeSeconds", -1);
        let mut r = TelemetryReading::default();
        BitaxeAdapter.parse_telemetry("/info", &j, &mut r);
        assert_eq!(r.uptime_s, None);
    }

    #[test]
    fn negative_sensor_readings_are_ignored() {
        let mut j = info_json();
        j.floats.insert("/hashRate", -1.0);
        j.floats.insert("/expectedHashrate", -1.0);
        j.floats.insert("/power", -1.0);
        j.floats.insert("/temp", -1.0);
        let mut r = TelemetryReading::default();
        BitaxeAdapter.parse_telemetry("/info", &j, &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.nominal_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.temperature, None);
        assert_eq!(r.uptime_s, Some(382));
    }

    #[test]
    fn parse_clears_owned_fields_that_vanished_from_response() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(99.0),
            power_w: Some(10.0),
            temperature: Some(DeviceTemp::Single(Temperature::from_celsius(50.0))),
            uptime_s: Some(123),
            ..TelemetryReading::default()
        };
        BitaxeAdapter.parse_telemetry("/info", &MapJson::default(), &mut r);
        assert_eq!(r, TelemetryReading::default());
    }

    #[test]
    fn reset_clears_the_endpoint_fields() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(1.0),
            power_w: Some(35.0),
            temperature: Some(DeviceTemp::Single(Temperature::from_celsius(59.0))),
            uptime_s: Some(382),
            nominal_hashrate_ths: Some(7.0),
            ..TelemetryReading::default()
        };
        BitaxeAdapter.reset_telemetry("/info", &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.nominal_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.temperature, None);
        assert_eq!(r.uptime_s, None);
    }

    #[test]
    fn parse_model_prefers_device_model() {
        let mut j = MapJson::default();
        j.strings.insert("/deviceModel", "NerdQAxe+");
        j.strings.insert("/boardVersion", "602");
        j.strings.insert("/ASICModel", "BM1370");
        j.ints.insert("/asicCount", 4);
        let mut acc = ModelAccumulator::default();
        BitaxeAdapter.parse_model("/info", &j, &mut acc);
        let model = acc
            .into_model()
            .expect("BUG: deviceModel must create a complete model");
        assert_eq!(model.id, "NerdQAxe+");
        assert_eq!(model.name, "NerdQAxe+");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, Some(4));
        assert_eq!(model.nominal_hashrate_ths, None);
    }

    #[test]
    fn parse_model_falls_back_to_board_version() {
        let mut j = MapJson::default();
        j.strings.insert("/boardVersion", "602");
        j.strings.insert("/ASICModel", "BM1370");
        let mut acc = ModelAccumulator::default();
        BitaxeAdapter.parse_model("/info", &j, &mut acc);
        let model = acc
            .into_model()
            .expect("BUG: boardVersion must create a complete model");
        assert_eq!(model.id, "axeos-board:602");
        assert_eq!(model.name, "Bitaxe board 602");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, None);
    }
}
