// Copyright (C) 2026  Braiins Systems s.r.o.

//! Hand-picked fixture summaries for the storybook screens.

use units::availability::Availability;
use units::units::{DegreeCelsius, JoulePerTeraHash, TeraHashPerSecond, Watt};

use crate::device::DeviceFamily;
use crate::screens::dashboard::DashboardVm;
use crate::screens::table::{FleetTableVm, ModelRow};
use crate::summary::{FleetSummary, GroupSummary};

const TABLE_PAGE_SIZE: usize = 4;

#[expect(
    clippy::too_many_arguments,
    reason = "a flat fixture builder reads clearer than nested structs"
)]
fn model(
    name: &str,
    ok: usize,
    degraded: usize,
    off: usize,
    hashrate_ths: f32,
    power_w: f32,
    efficiency_jth: f32,
    avg_temp_c: f32,
    spark_lo: f32,
    spark_hi: f32,
) -> ModelRow {
    ModelRow {
        name: name.to_owned(),
        ok,
        degraded,
        off,
        hashrate_ths,
        series: vec![
            spark_lo,
            spark_lo + 1.0,
            spark_hi,
            spark_hi + 0.5,
            spark_hi,
            spark_hi - 0.4,
            spark_hi,
            spark_hi + 0.2,
            spark_hi,
        ],
        power_w,
        efficiency_jth,
        avg_temp_c,
    }
}

/// The full mock fleet — twelve models, so the list view spans three pages.
fn fleet_models() -> Vec<ModelRow> {
    vec![
        model("BOS BMM", 14, 0, 0, 8.06, 20.54, 3.3, 65.0, 6.0, 8.0),
        model("BOS BFM", 10, 2, 2, 8.06, 20.78, 3.3, 65.0, 7.0, 8.2),
        model("uBOS HashNode", 9, 0, 2, 4.02, 19.50, 3.3, 65.0, 3.0, 4.1),
        model(
            "AxeOS NerdQaxe++",
            10,
            2,
            3,
            4.02,
            18.44,
            3.3,
            65.0,
            3.5,
            4.0,
        ),
        model(
            "BOS Miner S21",
            12,
            0,
            1,
            21.0,
            15.80,
            3.0,
            62.0,
            18.0,
            21.0,
        ),
        model("BOS Miner S19", 8, 1, 0, 13.5, 32.10, 3.2, 68.0, 11.0, 13.5),
        model("uBOS Compact", 6, 1, 1, 3.10, 21.40, 3.5, 70.0, 2.4, 3.1),
        model("AxeOS Ultra", 5, 0, 0, 5.60, 17.20, 3.1, 64.0, 4.8, 5.6),
        model("BOS Pro T21", 9, 2, 0, 19.2, 16.90, 3.1, 66.0, 16.0, 19.2),
        model("uBOS Nano", 4, 0, 2, 2.05, 24.10, 3.7, 72.0, 1.6, 2.1),
        model("AxeOS Max", 7, 1, 1, 6.80, 16.50, 3.0, 63.0, 5.6, 6.8),
        model("BOS Edge M31", 11, 0, 0, 11.4, 28.30, 3.4, 67.0, 9.0, 11.4),
    ]
}

/// One page of the mock fleet table; `page` is clamped to the valid range.
#[must_use]
pub fn sample_table_page(page: usize) -> FleetTableVm {
    let all = fleet_models();
    let device_count: usize = all.iter().map(|m| m.ok + m.degraded + m.off).sum();
    let page_count = all.len().div_ceil(TABLE_PAGE_SIZE).max(1);
    let page = page.min(page_count - 1);
    let rows = all
        .into_iter()
        .skip(page * TABLE_PAGE_SIZE)
        .take(TABLE_PAGE_SIZE)
        .collect();
    FleetTableVm {
        title: "Dominika's Mining Rig".to_owned(),
        device_count,
        rows,
        page,
        page_count,
    }
}

/// The Figma "Fleet overview" frame values, for the dashboard story.
#[must_use]
pub fn sample_dashboard() -> DashboardVm {
    DashboardVm {
        title: "Dominika's Mining Rig".to_owned(),
        device_count: 54,
        ok: 43,
        degraded: 4,
        off: 7,
        hashrate_ths: 17.08,
        hashrate_series: vec![
            7.0, 9.0, 12.0, 15.5, 17.0, 17.6, 18.0, 18.1, 17.9, 18.0, 18.2, 18.1, 17.9, 18.0, 18.1,
            18.0, 17.8, 18.0, 18.1, 16.8, 16.4, 16.9, 17.1, 17.08,
        ],
        power_w: 60.0,
        efficiency_jth: 10.01,
        temp_min_c: 54.0,
        temp_avg_c: 65.0,
        temp_max_c: 78.0,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "a flat fixture builder reads clearer than nested structs"
)]
fn group(
    label: &str,
    family: Option<DeviceFamily>,
    hashrate_ths: f64,
    power_w: f64,
    efficiency_jth: f64,
    temps_c: (f64, f64, f64),
    total: usize,
    ok: usize,
) -> GroupSummary {
    let (min_c, avg_c, max_c) = temps_c;
    GroupSummary {
        label: label.to_owned(),
        family,
        hashrate: Availability::Available(TeraHashPerSecond(hashrate_ths)),
        power: Availability::Available(Watt(power_w)),
        efficiency: Availability::Available(JoulePerTeraHash(efficiency_jth)),
        min_temperature: Availability::Available(DegreeCelsius(min_c)),
        avg_temperature: Availability::Available(DegreeCelsius(avg_c)),
        max_temperature: Availability::Available(DegreeCelsius(max_c)),
        total_count: total,
        ok_count: ok,
    }
}

/// A healthy mixed fleet spanning all three families, one straggler apart.
#[must_use]
pub fn sample_fleet() -> FleetSummary {
    let groups = vec![
        group(
            "BMM 101",
            Some(DeviceFamily::Bos),
            378.0,
            10_200.0,
            27.0,
            (58.0, 63.0, 70.0),
            3,
            3,
        ),
        group(
            "UMM 200",
            Some(DeviceFamily::Ubos),
            4.5,
            100.0,
            22.0,
            (60.0, 62.0, 65.0),
            1,
            1,
        ),
        group(
            "Bitaxe Gamma 601",
            Some(DeviceFamily::Bitaxe),
            3.2,
            51.0,
            16.0,
            (52.0, 55.0, 61.0),
            4,
            3,
        ),
    ];
    let total = group(
        "Total",
        None,
        385.7,
        10_351.0,
        26.8,
        (52.0, 62.0, 70.0),
        8,
        7,
    );
    FleetSummary { total, groups }
}
