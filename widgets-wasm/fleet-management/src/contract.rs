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

//! Contract tests: feed each family's `bmc-netsim` response shapes through its
//! real adapter and assert the derived device matches the sim. Fixtures mirror
//! `bmc-netsim/src/devices/{bos,ubos,axeos}.rs` — keep them in step; the drift
//! caught is widget-side (separate workspaces, no shared crate).

use crate::adapter::FamilyAdapter;
use crate::device::DeviceFamily;
use crate::discovery::tests_support::MapJson;
use crate::families::bitaxe::BitaxeAdapter;
use crate::families::bos::BosAdapter;
use crate::families::ubos::UbosAdapter;
use crate::model::ModelAccumulator;
use crate::telemetry::{DeviceTemp, TelemetryReading};
use bmc_wasm_sdk::types::Temperature;

#[test]
fn bos_sim_response_derives_the_intended_device() {
    // bos.rs default: Braiins Mini Miner BMM 101, 1 TH/s, sticker 1 TH/s, 32 W,
    // 65 °C, uptime 187020, two BM1370 boards (76 chips) spread, MAC on details.
    let mut found = MapJson::default();
    found.strings.insert("/service_type", "_http._tcp.local.");
    found.strings.insert("/name", "bos-01._http._tcp.local.");
    found.strings.insert("/host", "10.0.0.5");
    found.ints.insert("/port", 80);
    let discovered = BosAdapter
        .parse_found(&found)
        .expect("BUG: BOS discovery must parse");
    assert_eq!(discovered.identity.family, DeviceFamily::Bos);
    assert_eq!(discovered.identity.port, 80);

    let mut stats = MapJson::default();
    stats.floats.insert(
        "/miner_stats/real_hashrate/last_1m/gigahash_per_second",
        1_000.0,
    );
    stats
        .floats
        .insert("/power_stats/approximated_consumption/watt", 32.0);
    let mut boards = MapJson::default();
    boards
        .floats
        .insert("/hashboards/0/highest_chip_temp/temperature/degree_c", 60.0);
    boards.strings.insert("/hashboards/0/chip_type", "BM1370");
    boards.ints.insert("/hashboards/0/chips_count", 76);
    boards
        .floats
        .insert("/hashboards/1/highest_chip_temp/temperature/degree_c", 70.0);
    boards.strings.insert("/hashboards/1/chip_type", "BM1370");
    boards.ints.insert("/hashboards/1/chips_count", 76);
    let mut details = MapJson::default();
    details.ints.insert("/bosminer_uptime_s", 187_020);
    details.ints.insert("/platform", 8);
    details.strings.insert("/mac_address", "02:AB:CD:EF:01:23");
    details
        .strings
        .insert("/miner_identity/miner_model", "Braiins Mini Miner BMM 101");
    details
        .floats
        .insert("/sticker_hashrate/gigahash_per_second", 1_000.0);

    let mut reading = TelemetryReading::default();
    BosAdapter.parse_telemetry("/miner/stats", &stats, &mut reading);
    BosAdapter.parse_telemetry("/miner/hw/hashboards", &boards, &mut reading);
    BosAdapter.parse_telemetry("/miner/details", &details, &mut reading);
    assert_eq!(reading.current_hashrate_ths, Some(1.0));
    assert_eq!(reading.nominal_hashrate_ths, Some(1.0), "sticker_hashrate");
    assert_eq!(reading.power_w, Some(32.0));
    assert_eq!(
        reading.temperature,
        Some(DeviceTemp::Spread {
            min: Temperature::from_celsius(60.0),
            avg: Temperature::from_celsius(65.0),
            max: Temperature::from_celsius(70.0),
        }),
        "two hashboards spread around the 65 °C baseline",
    );
    assert_eq!(reading.uptime_s, Some(187_020));
    assert_eq!(reading.mac.as_deref(), Some("02:AB:CD:EF:01:23"));

    let mut acc = ModelAccumulator::default();
    BosAdapter.parse_model("/miner/hw/hashboards", &boards, &mut acc);
    BosAdapter.parse_model("/miner/details", &details, &mut acc);
    let model = acc.into_model().expect("BUG: BOS model must materialize");
    assert_eq!(model.id, "stm32mp157c-ii2-bmm1");
    assert_eq!(model.name, "Braiins Mini Miner BMM 101");
    assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
    assert_eq!(model.chip_count, Some(152));
    assert_eq!(
        model.nominal_hashrate_ths, None,
        "BOS nominal is on the reading"
    );
}

#[test]
fn bos_version_response_passes_the_discovery_fingerprint() {
    // Mirrors bos.rs: the public `GET /api/v1/version` body the widget probes.
    let mut version = MapJson::default();
    version.ints.insert("/major", 1);
    version.ints.insert("/minor", 6);
    version.ints.insert("/patch", 0);
    assert!(
        crate::families::bos::is_version_response(&version),
        "the sim's version body must fingerprint as BOS",
    );
    assert!(
        !crate::families::bos::is_version_response(&MapJson::default()),
        "a bare _http._tcp responder must not pass the fingerprint",
    );
}

#[test]
fn ubos_sim_response_derives_the_intended_device_with_catalog_nominal() {
    // ubos.rs default: Braiins Forge Miner x4, 4.8 TH/s (4.8e12 H/s),
    // 76 W (76000 mW), 65 °C, uptime 187020, no API nominal.
    let mut found = MapJson::default();
    found.strings.insert("/service_type", "_ubos._tcp.local.");
    found
        .strings
        .insert("/name", "bos-libre-01._ubos._tcp.local.");
    found.strings.insert("/host", "10.0.0.6");
    found.ints.insert("/port", 8080);
    let discovered = UbosAdapter
        .parse_found(&found)
        .expect("BUG: uBOS discovery must parse");
    assert_eq!(discovered.identity.family, DeviceFamily::Ubos);

    let mut info = MapJson::default();
    info.strings.insert("/name", "Braiins Forge Miner x4");
    info.floats.insert("/hashrate", 4.8e12);
    info.floats.insert("/power_out_mw", 76_000.0);
    info.floats.insert("/temperature", 65.0);
    info.ints.insert("/uptime", 187_020);

    let mut reading = TelemetryReading::default();
    UbosAdapter.parse_telemetry("/info", &info, &mut reading);
    assert_eq!(reading.current_hashrate_ths, Some(4.8));
    assert_eq!(
        reading.nominal_hashrate_ths, None,
        "uBOS API reports no nominal"
    );
    assert_eq!(reading.power_w, Some(76.0));
    assert_eq!(
        reading.temperature,
        Some(DeviceTemp::Single(Temperature::from_celsius(65.0))),
        "uBOS single sensor",
    );
    assert_eq!(reading.uptime_s, Some(187_020));
    assert_eq!(reading.mac, None, "uBOS API carries no MAC");

    let mut acc = ModelAccumulator::default();
    UbosAdapter.parse_model("/info", &info, &mut acc);
    let model = acc.into_model().expect("BUG: uBOS model must materialize");
    assert_eq!(model.name, "Braiins Forge Miner x4");
    assert_eq!(
        model.nominal_hashrate_ths,
        Some(4.8),
        "catalog supplies the uBOS nominal the API omits"
    );
}

#[test]
fn axeos_sim_response_derives_the_intended_device() {
    // axeos.rs default: NerdQAxe++, 4.5 TH/s (4500 GH/s), expected 4.5, 76 W,
    // 62 °C, uptime 187020, BM1370 ×4.
    let mut found = MapJson::default();
    found.strings.insert("/service_type", "_http._tcp.local.");
    found.strings.insert("/name", "axeos-01._http._tcp.local.");
    found.strings.insert("/host", "10.0.0.7");
    found.ints.insert("/port", 80);
    found.strings.insert("/txt/family", "NerdQAxe");
    found.strings.insert("/txt/board", "++");
    found.strings.insert("/txt/asic", "BM1370");
    found.strings.insert("/txt/asic_count", "4");
    let discovered = BitaxeAdapter
        .parse_found(&found)
        .expect("BUG: AxeOS discovery must parse");
    assert_eq!(discovered.identity.family, DeviceFamily::Bitaxe);

    let mut info = MapJson::default();
    info.floats.insert("/hashRate", 4_500.0);
    info.floats.insert("/expectedHashrate", 4_500.0);
    info.floats.insert("/power", 76.0);
    info.floats.insert("/temp", 62.0);
    info.ints.insert("/uptimeSeconds", 187_020);
    info.strings.insert("/macAddr", "02:AB:CD:EF:04:56");
    info.strings.insert("/deviceModel", "NerdQAxe++");
    info.strings.insert("/ASICModel", "BM1370");
    info.ints.insert("/asicCount", 4);

    let mut reading = TelemetryReading::default();
    BitaxeAdapter.parse_telemetry("/info", &info, &mut reading);
    assert_eq!(reading.current_hashrate_ths, Some(4.5));
    assert_eq!(reading.nominal_hashrate_ths, Some(4.5), "expectedHashrate");
    assert_eq!(reading.power_w, Some(76.0));
    assert_eq!(
        reading.temperature,
        Some(DeviceTemp::Single(Temperature::from_celsius(62.0))),
        "AxeOS single `temp` sensor",
    );
    assert_eq!(reading.uptime_s, Some(187_020));
    assert_eq!(reading.mac.as_deref(), Some("02:AB:CD:EF:04:56"));

    let mut acc = ModelAccumulator::default();
    BitaxeAdapter.parse_model("/info", &info, &mut acc);
    let model = acc.into_model().expect("BUG: AxeOS model must materialize");
    assert_eq!(model.id, "NerdQAxe++");
    assert_eq!(model.name, "NerdQAxe++");
    assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
    assert_eq!(model.chip_count, Some(4));
    assert_eq!(
        model.nominal_hashrate_ths, None,
        "AxeOS nominal rides on the reading, not the model"
    );
}
