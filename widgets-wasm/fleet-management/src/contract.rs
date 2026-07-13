// Copyright (C) 2026  Braiins Systems s.r.o.

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
use crate::telemetry::TelemetryReading;

#[test]
fn bos_sim_response_derives_the_intended_device() {
    // bos.rs default: BMM 101, 100 TH/s, sticker 100 TH/s, 3250 W, 65 °C,
    // uptime 187020, two BM1370 boards of 76 chips.
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
        100_000.0,
    );
    stats
        .floats
        .insert("/power_stats/approximated_consumption/watt", 3_250.0);
    let mut boards = MapJson::default();
    boards
        .floats
        .insert("/hashboards/0/highest_chip_temp/temperature/degree_c", 65.0);
    boards.strings.insert("/hashboards/0/chip_type", "BM1370");
    boards.ints.insert("/hashboards/0/chips_count", 76);
    boards
        .floats
        .insert("/hashboards/1/highest_chip_temp/temperature/degree_c", 65.0);
    boards.strings.insert("/hashboards/1/chip_type", "BM1370");
    boards.ints.insert("/hashboards/1/chips_count", 76);
    let mut details = MapJson::default();
    details.ints.insert("/bosminer_uptime_s", 187_020);
    details.ints.insert("/platform", 8);
    details
        .strings
        .insert("/miner_identity/miner_model", "BMM 101");
    details
        .floats
        .insert("/sticker_hashrate/gigahash_per_second", 100_000.0);

    let mut reading = TelemetryReading::default();
    BosAdapter.parse_telemetry("/miner/stats", &stats, &mut reading);
    BosAdapter.parse_telemetry("/miner/hw/hashboards", &boards, &mut reading);
    BosAdapter.parse_telemetry("/miner/details", &details, &mut reading);
    assert_eq!(reading.current_hashrate_ths, Some(100.0));
    assert_eq!(
        reading.nominal_hashrate_ths,
        Some(100.0),
        "sticker_hashrate"
    );
    assert_eq!(reading.power_w, Some(3_250.0));
    assert_eq!(reading.temperature_c, Some(65.0));
    assert_eq!(reading.uptime_s, Some(187_020));

    let mut acc = ModelAccumulator::default();
    BosAdapter.parse_model("/miner/hw/hashboards", &boards, &mut acc);
    BosAdapter.parse_model("/miner/details", &details, &mut acc);
    let model = acc.into_model().expect("BUG: BOS model must materialize");
    assert_eq!(model.id, "stm32mp157c-ii2-bmm1");
    assert_eq!(model.name, "BMM 101");
    assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
    assert_eq!(model.chip_count, Some(152));
    assert_eq!(
        model.nominal_hashrate_ths, None,
        "BOS nominal is on the reading"
    );
}

#[test]
fn ubos_sim_response_derives_the_intended_device_with_catalog_nominal() {
    // ubos.rs default: HashNode, 4 TH/s (4e12 H/s), 200 W (200000 mW), 65 °C,
    // uptime 187020, no API nominal.
    let mut found = MapJson::default();
    found.strings.insert("/service_type", "_ubos._tcp.local.");
    found.strings.insert("/name", "ubos-01._ubos._tcp.local.");
    found.strings.insert("/host", "10.0.0.6");
    found.ints.insert("/port", 8080);
    let discovered = UbosAdapter
        .parse_found(&found)
        .expect("BUG: uBOS discovery must parse");
    assert_eq!(discovered.identity.family, DeviceFamily::Ubos);

    let mut info = MapJson::default();
    info.strings.insert("/name", "HashNode");
    info.floats.insert("/hashrate", 4e12);
    info.floats.insert("/power_out_mw", 200_000.0);
    info.floats.insert("/temperature", 65.0);
    info.ints.insert("/uptime", 187_020);

    let mut reading = TelemetryReading::default();
    UbosAdapter.parse_telemetry("/info", &info, &mut reading);
    assert_eq!(reading.current_hashrate_ths, Some(4.0));
    assert_eq!(
        reading.nominal_hashrate_ths, None,
        "uBOS API reports no nominal"
    );
    assert_eq!(reading.power_w, Some(200.0));
    assert_eq!(reading.temperature_c, Some(65.0));
    assert_eq!(reading.uptime_s, Some(187_020));

    let mut acc = ModelAccumulator::default();
    UbosAdapter.parse_model("/info", &info, &mut acc);
    let model = acc.into_model().expect("BUG: uBOS model must materialize");
    assert_eq!(model.name, "HashNode");
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
    info.strings.insert("/deviceModel", "NerdQAxe++");
    info.strings.insert("/ASICModel", "BM1370");
    info.ints.insert("/asicCount", 4);

    let mut reading = TelemetryReading::default();
    BitaxeAdapter.parse_telemetry("/info", &info, &mut reading);
    assert_eq!(reading.current_hashrate_ths, Some(4.5));
    assert_eq!(reading.nominal_hashrate_ths, Some(4.5), "expectedHashrate");
    assert_eq!(reading.power_w, Some(76.0));
    assert_eq!(reading.temperature_c, Some(62.0));
    assert_eq!(reading.uptime_s, Some(187_020));

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
