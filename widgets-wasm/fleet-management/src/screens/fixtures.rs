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

//! Hand-picked fixture summaries for the gallery screens.

use bmc_wasm_sdk::{ElectricPower, Hashrate, MiningEfficiency, Temperature};

use crate::device::DeviceFamily;
use crate::history::{ChartWindow, HistoryDatum};
use crate::screens::dashboard::DashboardViewData;
use crate::screens::device_detail::DeviceDetailData;
use crate::screens::model_detail::{DeviceRow, ModelDetailViewData};
use crate::screens::no_credentials::NoCredentialsData;
use crate::screens::table::{ModelRow, TableViewData};
use crate::summary::DeviceStatus;
use crate::telemetry::DeviceTemp;
use crate::view::device_click_id;

const TABLE_PAGE_SIZE: usize = 4;

/// Wrap raw hashrate values into evenly-spaced, present samples for a story.
fn samples(values: &[f32]) -> Vec<HistoryDatum> {
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| HistoryDatum {
            at: i64::try_from(i).unwrap_or(0) * 60,
            value: Some(v),
        })
        .collect()
}

fn family_of(name: &str) -> Option<DeviceFamily> {
    if name.starts_with("uBOS") {
        Some(DeviceFamily::Ubos)
    } else if name.starts_with("BOS") {
        Some(DeviceFamily::Bos)
    } else if name.starts_with("AxeOS") {
        Some(DeviceFamily::Bitaxe)
    } else {
        None
    }
}

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
        family: family_of(name),
        ok,
        degraded,
        off,
        hashrate: Some(Hashrate::from_terahashes_per_second(f64::from(
            hashrate_ths,
        ))),
        series: samples(&[
            spark_lo,
            spark_lo + 1.0,
            spark_hi,
            spark_hi + 0.5,
            spark_hi,
            spark_hi - 0.4,
            spark_hi,
            spark_hi + 0.2,
            spark_hi,
        ]),
        power: Some(ElectricPower::from_watts(f64::from(power_w))),
        efficiency: Some(MiningEfficiency::from_joules_per_terahash(f64::from(
            efficiency_jth,
        ))),
        avg_temp: Some(Temperature::from_celsius(f64::from(avg_temp_c))),
    }
}

/// The full mock fleet — twelve models, so the list view spans three pages.
fn fleet_models() -> Vec<ModelRow> {
    vec![
        model("BOS BMM", 14, 0, 0, 8.06, 20.54, 3.3, 65.0, 6.0, 8.0),
        model("BOS BFM", 10, 2, 2, 8.06, 20.78, 3.3, 65.0, 7.0, 8.2),
        model(
            "uBOS Forge Miner x4",
            9,
            0,
            2,
            4.02,
            19.50,
            3.3,
            65.0,
            3.0,
            4.1,
        ),
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
pub fn sample_table_page(page: usize) -> TableViewData {
    let all = fleet_models();
    let device_count: usize = all.iter().map(|m| m.ok + m.degraded + m.off).sum();
    let page_count = all.len().div_ceil(TABLE_PAGE_SIZE).max(1);
    let page = page.min(page_count - 1);
    let rows: Vec<ModelRow> = all
        .into_iter()
        .skip(page * TABLE_PAGE_SIZE)
        .take(TABLE_PAGE_SIZE)
        .collect();
    let window = story_window(rows.first().map(|r| r.series.as_slice()));
    TableViewData {
        title: "Dominika's Mining Rig".to_owned(),
        device_count,
        rows,
        window,
        page,
        page_count,
    }
}

/// The window a story's baked series should fill — the whole width.
fn story_window(series: Option<&[HistoryDatum]>) -> ChartWindow {
    ChartWindow::covering(series.unwrap_or(&[]))
}

/// The Figma "Fleet overview" frame values, for the dashboard story.
#[must_use]
pub fn sample_dashboard(auth: usize) -> DashboardViewData {
    let hashrate_series = samples(&[
        7.0, 9.0, 12.0, 15.5, 17.0, 17.6, 18.0, 18.1, 17.9, 18.0, 18.2, 18.1, 17.9, 18.0, 18.1,
        18.0, 17.8, 18.0, 18.1, 16.8, 16.4, 16.9, 17.1, 17.08,
    ]);
    let window = story_window(Some(&hashrate_series));
    DashboardViewData {
        title: "Dominika's Mining Rig".to_owned(),
        device_count: 54,
        ok: 43,
        degraded: 4,
        off: 7,
        auth,
        hashrate: Some(Hashrate::from_terahashes_per_second(17.08)),
        hashrate_series,
        window,
        power: Some(ElectricPower::from_watts(60.0)),
        efficiency: Some(MiningEfficiency::from_joules_per_terahash(10.01)),
        temp_min: Some(Temperature::from_celsius(54.0)),
        temp_avg: Some(Temperature::from_celsius(65.0)),
        temp_max: Some(Temperature::from_celsius(78.0)),
    }
}

/// Ten mock devices (three pages of four) for the model-detail story,
/// each with a baked hashrate series; the down device (0 TH/s) reads flat.
fn model_detail_devices() -> Vec<DeviceRow> {
    let device = |name: &str, hashrate: f32, temp: f32, status: DeviceStatus| DeviceRow {
        hostname: name.to_owned(),
        click_id: device_click_id(name),
        status,
        hashrate: Some(Hashrate::from_terahashes_per_second(f64::from(hashrate))),
        series: samples(&[
            hashrate * 0.95,
            hashrate,
            hashrate * 1.03,
            hashrate * 1.01,
            hashrate,
            hashrate * 0.98,
            hashrate * 1.02,
            hashrate,
        ]),
        power: Some(ElectricPower::from_watts(2.05)),
        efficiency: Some(MiningEfficiency::from_joules_per_terahash(3.3)),
        avg_temp: Some(Temperature::from_celsius(f64::from(temp))),
        min_temp: Some(Temperature::from_celsius(f64::from(temp) - 4.0)),
        max_temp: Some(Temperature::from_celsius(f64::from(temp) + 6.0)),
    };
    // Spread the four statuses across the pages so the story shows each glyph.
    vec![
        device("Miner-Abcde", 1.01, 65.0, DeviceStatus::Ok),
        device("John's Miner", 1.00, 66.0, DeviceStatus::Ok),
        device("Miner - Level 2", 1.02, 64.0, DeviceStatus::Ok),
        device("Miner - Level 2", 0.98, 67.0, DeviceStatus::Degraded),
        device("bmm-123456", 0.83, 65.0, DeviceStatus::Ok),
        device("bmm-789abc", 0.79, 63.0, DeviceStatus::Ok),
        device("bmm-def012", 0.00, 64.0, DeviceStatus::Unreachable),
        device("bmm-345678", 0.81, 66.0, DeviceStatus::Ok),
        device("bmm-9abcde", 0.00, 71.0, DeviceStatus::ApiError),
        device("bmm-f01234", 0.80, 65.0, DeviceStatus::Ok),
    ]
}

/// One page of the mock model detail; `page` is clamped to the valid range.
#[must_use]
pub fn sample_model_detail_view(page: usize) -> ModelDetailViewData {
    let all = model_detail_devices();
    let device_count = all.len();
    let page_count = device_count.div_ceil(TABLE_PAGE_SIZE).max(1);
    let page = page.min(page_count - 1);
    let rows: Vec<DeviceRow> = all
        .into_iter()
        .skip(page * TABLE_PAGE_SIZE)
        .take(TABLE_PAGE_SIZE)
        .collect();
    let window = story_window(rows.first().map(|r| r.series.as_slice()));
    ModelDetailViewData {
        fleet_name: "Dominika's Mining Rig".to_owned(),
        title: "BOS BMM".to_owned(),
        device_count,
        rows,
        window,
        page,
        page_count,
    }
}

fn device_detail_fixture(
    mac: Option<&str>,
    temperature: Option<DeviceTemp>,
    state: DeviceStatus,
) -> DeviceDetailData {
    let hashrate_series = samples(&[1.9, 2.0, 2.1, 2.05, 2.08, 2.0, 2.1, 2.08]);
    let window = story_window(Some(&hashrate_series));
    let delivering = matches!(state, DeviceStatus::Ok | DeviceStatus::Degraded);
    DeviceDetailData {
        fleet_name: "Dominika's Mining Rig".to_owned(),
        model: "Mini Miner".to_owned(),
        hostname: "John's Miner".to_owned(),
        ip: "192.111.18".to_owned(),
        mac: mac.map(str::to_owned),
        state,
        hashrate: delivering.then(|| Hashrate::from_terahashes_per_second(2.08)),
        hashrate_series,
        window,
        nominal_hashrate: Some(Hashrate::from_terahashes_per_second(16.52)),
        power: delivering.then(|| ElectricPower::from_watts(60.0)),
        efficiency: delivering.then(|| MiningEfficiency::from_joules_per_terahash(2.01)),
        uptime_hours: delivering.then_some(12),
        temperature,
    }
}

/// A multi-sensor miner: the temp tile shows the Avg/Min/Max spread.
#[must_use]
pub fn sample_device_detail() -> DeviceDetailData {
    device_detail_fixture(
        Some("02:1A:4B:7C:9D:01"),
        Some(DeviceTemp::Spread {
            min: Temperature::from_celsius(54.0),
            avg: Temperature::from_celsius(65.0),
            max: Temperature::from_celsius(78.0),
        }),
        DeviceStatus::Ok,
    )
}

/// A single-sensor miner (uBOS, no MAC): the temp tile shows one value.
#[must_use]
pub fn sample_device_detail_single() -> DeviceDetailData {
    device_detail_fixture(
        None,
        Some(DeviceTemp::Single(Temperature::from_celsius(65.0))),
        DeviceStatus::Ok,
    )
}

/// A miner present over mDNS but not delivering telemetry (a 503 API): the State
/// tile reads "API error" and the live metrics show the unavailable marker.
#[must_use]
pub fn sample_device_detail_error() -> DeviceDetailData {
    device_detail_fixture(None, None, DeviceStatus::ApiError)
}

#[must_use]
pub fn sample_no_credentials() -> NoCredentialsData {
    NoCredentialsData {
        fleet_name: "Dominika's Mining Rig".to_owned(),
        ssid: "Braiins-Guest".to_owned(),
        url: "http://192.168.1.42".to_owned(),
    }
}
