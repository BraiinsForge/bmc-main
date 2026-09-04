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

//! Build the redesigned screens' view data from the live fleet summary
//! and its recorded hashrate history.

use bmc_wasm_sdk::types::Hashrate;

use crate::device::{DeviceId, KnownDevice};
use crate::history::{ChartWindow, HistoryDatum, HistoryView};
use crate::screens::dashboard::DashboardViewData;
use crate::screens::device_detail::DeviceDetailData;
use crate::screens::model_detail::{DeviceRow, ModelDetailViewData};
use crate::screens::table::{ModelRow, TableViewData};
use crate::summary::{DeviceStatus, FleetSummary, GroupSummary, device_status};
use crate::view::device_click_id;

/// Model groups per page in the list view.
const TABLE_PAGE_SIZE: usize = 4;
/// Device rows per page in the model-detail view.
const MODEL_DETAIL_PAGE_SIZE: usize = 4;

/// Reachable-but-not-ok devices: `total - ok - off - auth`.
fn degraded(g: &GroupSummary) -> usize {
    g.total_count
        .saturating_sub(g.ok_count)
        .saturating_sub(g.off_count)
        .saturating_sub(g.auth_error_count)
}

impl DashboardViewData {
    /// The grid overview, from the fleet total and its recorded history.
    #[must_use]
    pub fn from_summary(
        summary: &FleetSummary,
        fleet_name: &str,
        history: &HistoryView<'_>,
        window: ChartWindow,
    ) -> Self {
        let t = &summary.total;
        Self {
            title: fleet_name.to_owned(),
            device_count: t.total_count,
            ok: t.ok_count,
            degraded: degraded(t),
            off: t.off_count,
            auth: t.auth_error_count,
            hashrate: t.hashrate,
            hashrate_series: history.total_series(),
            window,
            power: t.power,
            efficiency: t.efficiency,
            temp_min: t.min_temperature,
            temp_avg: t.avg_temperature,
            temp_max: t.max_temperature,
        }
    }
}

impl TableViewData {
    /// One page of the list view, from the per-model groups and their history.
    #[must_use]
    pub fn from_summary(
        summary: &FleetSummary,
        fleet_name: &str,
        page: usize,
        history: &HistoryView<'_>,
        window: ChartWindow,
    ) -> Self {
        let groups = &summary.groups;
        let page_count = groups.len().div_ceil(TABLE_PAGE_SIZE).max(1);
        let page = page.min(page_count - 1);
        let rows = groups
            .iter()
            .skip(page * TABLE_PAGE_SIZE)
            .take(TABLE_PAGE_SIZE)
            .map(|g| ModelRow {
                name: g.label.clone(),
                family: g.family,
                ok: g.ok_count,
                degraded: degraded(g),
                // The compact row has no room for a fourth count, so auth failures
                // fold into off here — both are "present, not delivering".
                // The dashboard and device detail keep them apart.
                off: g.off_count + g.auth_error_count,
                hashrate: g.hashrate,
                series: history.model_series(g.family, &g.label),
                power: g.power,
                efficiency: g.efficiency,
                avg_temp: g.avg_temperature,
            })
            .collect();
        Self {
            title: fleet_name.to_owned(),
            device_count: summary.total.total_count,
            rows,
            window,
            page,
            page_count,
        }
    }
}

impl ModelDetailViewData {
    /// One page of the model's device rows, from `model_detail_rows`'
    /// (id, group) pairs and each device's recorded history.
    #[must_use]
    pub(crate) fn from_summary(
        fleet_name: &str,
        title: &str,
        rows: &[(DeviceId, GroupSummary, DeviceStatus)],
        page: usize,
        history: &HistoryView<'_>,
        window: ChartWindow,
    ) -> Self {
        let page_count = rows.len().div_ceil(MODEL_DETAIL_PAGE_SIZE).max(1);
        let page = page.min(page_count - 1);
        let devices = rows
            .iter()
            .skip(page * MODEL_DETAIL_PAGE_SIZE)
            .take(MODEL_DETAIL_PAGE_SIZE)
            .map(|(id, g, status)| DeviceRow {
                hostname: g.label.clone(),
                click_id: device_click_id(id.as_str()),
                status: *status,
                hashrate: g.hashrate,
                series: history.device_series(id),
                power: g.power,
                efficiency: g.efficiency,
                avg_temp: g.avg_temperature,
                min_temp: g.min_temperature,
                max_temp: g.max_temperature,
            })
            .collect();
        Self {
            fleet_name: fleet_name.to_owned(),
            title: title.to_owned(),
            device_count: rows.len(),
            rows: devices,
            window,
            page,
            page_count,
        }
    }
}

/// Whole hours from a second count, for the uptime tile.
#[expect(clippy::integer_division, reason = "uptime is shown in whole hours")]
fn seconds_to_hours(seconds: u64) -> u64 {
    seconds / 3600
}

impl DeviceDetailData {
    /// One device's detail: the fold-of-one group gives hashrate/power/efficiency,
    /// the raw reading gives MAC/uptime/nominal/temperature. Live measurements are
    /// dropped unless the device is delivering, so a stale reading never reads
    /// as a live one off an unreachable machine.
    #[must_use]
    pub(crate) fn from_device(
        fleet_name: &str,
        model: &str,
        group: &GroupSummary,
        device: &KnownDevice,
        hashrate_series: Vec<HistoryDatum>,
        window: ChartWindow,
    ) -> Self {
        let reading = device.telemetry.as_ref().map(|s| &s.reading);
        let nominal_ths = reading
            .and_then(|r| r.nominal_hashrate_ths)
            .or_else(|| device.model.as_ref().and_then(|m| m.nominal_hashrate_ths));
        let state = device_status(device);
        let delivering = matches!(state, DeviceStatus::Ok | DeviceStatus::Degraded);
        Self {
            fleet_name: fleet_name.to_owned(),
            model: model.to_owned(),
            hostname: group.label.clone(),
            ip: device.identity.host.clone(),
            mac: reading.and_then(|r| r.mac.clone()),
            state,
            hashrate: delivering.then_some(group.hashrate).flatten(),
            hashrate_series,
            window,
            nominal_hashrate: nominal_ths
                .map(|n| Hashrate::from_terahashes_per_second(f64::from(n))),
            power: delivering.then_some(group.power).flatten(),
            efficiency: delivering.then_some(group.efficiency).flatten(),
            uptime_hours: delivering
                .then(|| reading.and_then(|r| r.uptime_s))
                .flatten()
                .map(seconds_to_hours),
            temperature: delivering
                .then(|| reading.and_then(|r| r.temperature))
                .flatten(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceFamily, DeviceId, DeviceIdentity, Membership};
    use crate::history::{ChartSpan, HashrateHistory};
    use crate::telemetry::{DeviceTemp, TelemetryReading, TelemetrySnapshot};
    use bmc_wasm_sdk::types::{ElectricPower, Hashrate, MiningEfficiency, Temperature};

    fn grp(label: &str, total: usize, ok: usize, off: usize) -> GroupSummary {
        GroupSummary {
            label: label.to_owned(),
            family: Some(DeviceFamily::Bos),
            hashrate: Some(Hashrate::from_terahashes_per_second(10.0)),
            power: Some(ElectricPower::from_watts(30.0)),
            efficiency: Some(MiningEfficiency::from_joules_per_terahash(3.0)),
            min_temperature: Some(Temperature::from_celsius(60.0)),
            avg_temperature: Some(Temperature::from_celsius(65.0)),
            max_temperature: Some(Temperature::from_celsius(70.0)),
            total_count: total,
            ok_count: ok,
            off_count: off,
            auth_error_count: 0,
        }
    }

    #[test]
    fn degraded_is_the_reachable_not_ok_remainder() {
        assert_eq!(degraded(&grp("m", 10, 6, 2)), 2);
        assert_eq!(degraded(&grp("m", 5, 5, 0)), 0);
        assert_eq!(degraded(&grp("m", 3, 0, 3)), 0);
    }

    #[test]
    fn table_row_folds_auth_into_off_so_counts_sum_to_the_total() {
        let mut g = grp("m", 10, 5, 2);
        g.auth_error_count = 3;
        let summary = FleetSummary {
            total: g.clone(),
            groups: vec![g],
        };
        let history = HashrateHistory::default();
        let view = history.view(ChartSpan::OneHour);
        let win = ChartWindow::covering(&[]);
        let row = &TableViewData::from_summary(&summary, "Rig", 0, &view, win).rows[0];
        assert_eq!(row.off, 5, "2 off + 3 auth failures");
        assert_eq!(row.degraded, 0, "auth is not degraded");
        assert_eq!(row.ok + row.degraded + row.off, 10, "counts add up again");
    }

    #[test]
    fn table_paginates_and_clamps_the_page() {
        let groups: Vec<GroupSummary> = (0..9_u64)
            .map(|i| {
                let mut label = String::from("m");
                units::format::push_int(&mut label, i);
                grp(&label, 1, 1, 0)
            })
            .collect();
        let summary = FleetSummary {
            total: grp("Total", 9, 9, 0),
            groups,
        };
        let history = HashrateHistory::default();
        let view = history.view(ChartSpan::OneHour);
        let win = ChartWindow::covering(&[]);
        let first = TableViewData::from_summary(&summary, "Rig", 0, &view, win);
        assert_eq!(first.page_count, 3, "9 groups, 4 per page");
        assert_eq!(first.rows.len(), TABLE_PAGE_SIZE);
        let last = TableViewData::from_summary(&summary, "Rig", 99, &view, win);
        assert_eq!(last.page, 2, "an out-of-range page clamps to the last");
        assert_eq!(last.rows.len(), 1);
    }

    #[test]
    fn model_detail_paginates_devices_and_maps_the_click_id() {
        let rows: Vec<(DeviceId, GroupSummary, DeviceStatus)> = (0..6_u64)
            .map(|i| {
                let mut name = String::from("dev-");
                units::format::push_int(&mut name, i);
                (
                    DeviceId::new(name.clone()),
                    grp(&name, 1, 1, 0),
                    DeviceStatus::Ok,
                )
            })
            .collect();
        let history = HashrateHistory::default();
        let view = history.view(ChartSpan::OneHour);
        let win = ChartWindow::covering(&[]);
        let first = ModelDetailViewData::from_summary("Rig", "BMM 101", &rows, 0, &view, win);
        assert_eq!(first.device_count, 6);
        assert_eq!(first.page_count, 2, "6 devices, 4 per page");
        assert_eq!(first.rows.len(), MODEL_DETAIL_PAGE_SIZE);
        assert_eq!(first.rows[0].click_id, "device:dev-0");
        let last = ModelDetailViewData::from_summary("Rig", "BMM 101", &rows, 9, &view, win);
        assert_eq!(last.page, 1, "an out-of-range page clamps to the last");
        assert_eq!(last.rows.len(), 2);
    }

    fn device(
        model: Option<crate::model::MinerModel>,
        reading: Option<TelemetryReading>,
    ) -> KnownDevice {
        KnownDevice {
            identity: DeviceIdentity {
                id: DeviceId::new("d"),
                family: DeviceFamily::Bos,
                name: "d".to_owned(),
                host: "10.0.0.9".to_owned(),
                port: 80,
            },
            model,
            telemetry: reading.map(|reading| TelemetrySnapshot { reading }),
            reachable: true,
            consecutive_failures: 0,
            membership: Membership::Confirmed,
            last_failure: None,
            unreachable_since: None,
        }
    }

    #[test]
    fn device_detail_maps_the_group_and_the_raw_reading() {
        let reading = TelemetryReading {
            current_hashrate_ths: Some(15.0),
            nominal_hashrate_ths: Some(16.0),
            uptime_s: Some(43_200),
            temperature: Some(DeviceTemp::Spread {
                min: Temperature::from_celsius(54.0),
                avg: Temperature::from_celsius(65.0),
                max: Temperature::from_celsius(78.0),
            }),
            mac: Some("aa:bb:cc:dd:ee:ff".to_owned()),
            ..TelemetryReading::default()
        };
        let data = DeviceDetailData::from_device(
            "Rig",
            "Mini Miner",
            &grp("John's Miner", 1, 1, 0),
            &device(None, Some(reading)),
            vec![],
            ChartWindow::covering(&[]),
        );
        assert_eq!(data.hostname, "John's Miner");
        assert_eq!(data.ip, "10.0.0.9");
        assert_eq!(data.mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(
            data.state,
            DeviceStatus::Ok,
            "reachable and above 20% of nominal"
        );
        assert_eq!(data.uptime_hours, Some(12), "43200 s / 3600");
        assert_eq!(
            data.nominal_hashrate,
            Some(Hashrate::from_terahashes_per_second(16.0))
        );
        assert_eq!(
            data.temperature,
            Some(DeviceTemp::Spread {
                min: Temperature::from_celsius(54.0),
                avg: Temperature::from_celsius(65.0),
                max: Temperature::from_celsius(78.0),
            })
        );
    }

    #[test]
    fn device_detail_falls_back_to_catalog_nominal_and_marks_unreachable() {
        let model = crate::model::MinerModel {
            id: "id".to_owned(),
            name: "Braiins Forge Miner x4".to_owned(),
            chip_type: None,
            chip_count: None,
            nominal_hashrate_ths: Some(4.5),
        };
        // Unreachable with no telemetry: state comes from the device, not the group.
        let mut dev = device(Some(model), None);
        dev.reachable = false;
        let data = DeviceDetailData::from_device(
            "Rig",
            "Braiins Forge Miner x4",
            &grp("bmm", 1, 0, 1),
            &dev,
            vec![],
            ChartWindow::covering(&[]),
        );
        assert_eq!(data.state, DeviceStatus::Unreachable);
        assert_eq!(data.mac, None);
        // Live measurements read absent — never a fabricated zero —
        // off a non-delivering device.
        assert_eq!(data.uptime_hours, None);
        assert_eq!(data.temperature, None);
        assert_eq!(data.hashrate, None);
        assert_eq!(data.power, None);
        assert_eq!(data.efficiency, None);
        assert_eq!(
            data.nominal_hashrate,
            Some(Hashrate::from_terahashes_per_second(4.5)),
            "nameplate persists — a spec, not a live reading"
        );
    }
}
