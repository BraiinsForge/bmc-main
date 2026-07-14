// Copyright (C) 2026  Braiins Systems s.r.o.

//! Build the redesigned screens' view data from the live fleet summary
//! and its recorded hashrate history.

use crate::device::DeviceId;
use crate::history::HashrateHistory;
use crate::screens::dashboard::DashboardViewData;
use crate::screens::model_detail::{DeviceRow, ModelDetailViewData};
use crate::screens::table::{ModelRow, TableViewData};
use crate::summary::{FleetSummary, GroupSummary};
use crate::view::device_click_id;

/// Model groups per page in the list view.
const TABLE_PAGE_SIZE: usize = 4;
/// Device rows per page in the model-detail view.
const MODEL_DETAIL_PAGE_SIZE: usize = 4;

/// Reachable-but-not-ok devices: `total - ok - off`.
fn degraded(g: &GroupSummary) -> usize {
    g.total_count
        .saturating_sub(g.ok_count)
        .saturating_sub(g.off_count)
}

impl DashboardViewData {
    /// The grid overview, from the fleet total and its recorded history.
    #[must_use]
    pub fn from_summary(
        summary: &FleetSummary,
        fleet_name: &str,
        history: &HashrateHistory,
    ) -> Self {
        let t = &summary.total;
        Self {
            title: fleet_name.to_owned(),
            device_count: t.total_count,
            ok: t.ok_count,
            degraded: degraded(t),
            off: t.off_count,
            hashrate: t.hashrate.unwrap_or_default(),
            hashrate_series: history.total_series(),
            power: t.power.unwrap_or_default(),
            efficiency: t.efficiency.unwrap_or_default(),
            temp_min: t.min_temperature.unwrap_or_default(),
            temp_avg: t.avg_temperature.unwrap_or_default(),
            temp_max: t.max_temperature.unwrap_or_default(),
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
        history: &HashrateHistory,
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
                off: g.off_count,
                hashrate: g.hashrate.unwrap_or_default(),
                series: history.model_series(g.family, &g.label),
                power: g.power.unwrap_or_default(),
                efficiency: g.efficiency.unwrap_or_default(),
                avg_temp: g.avg_temperature.unwrap_or_default(),
            })
            .collect();
        Self {
            title: fleet_name.to_owned(),
            device_count: summary.total.total_count,
            rows,
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
        title: &str,
        rows: &[(DeviceId, GroupSummary)],
        page: usize,
        history: &HashrateHistory,
    ) -> Self {
        let page_count = rows.len().div_ceil(MODEL_DETAIL_PAGE_SIZE).max(1);
        let page = page.min(page_count - 1);
        let devices = rows
            .iter()
            .skip(page * MODEL_DETAIL_PAGE_SIZE)
            .take(MODEL_DETAIL_PAGE_SIZE)
            .map(|(id, g)| DeviceRow {
                hostname: g.label.clone(),
                click_id: device_click_id(id.as_str()),
                hashrate: g.hashrate.unwrap_or_default(),
                series: history.device_series(id),
                power: g.power.unwrap_or_default(),
                efficiency: g.efficiency.unwrap_or_default(),
                avg_temp: g.avg_temperature.unwrap_or_default(),
                min_temp: g.min_temperature.unwrap_or_default(),
                max_temp: g.max_temperature.unwrap_or_default(),
            })
            .collect();
        Self {
            title: title.to_owned(),
            device_count: rows.len(),
            rows: devices,
            page,
            page_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceFamily, DeviceId};
    use bmc_wasm_sdk::{ElectricPower, Hashrate, MiningEfficiency, Temperature};

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
        }
    }

    #[test]
    fn degraded_is_the_reachable_not_ok_remainder() {
        assert_eq!(degraded(&grp("m", 10, 6, 2)), 2);
        assert_eq!(degraded(&grp("m", 5, 5, 0)), 0);
        assert_eq!(degraded(&grp("m", 3, 0, 3)), 0);
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
        let first = TableViewData::from_summary(&summary, "Rig", 0, &history);
        assert_eq!(first.page_count, 3, "9 groups, 4 per page");
        assert_eq!(first.rows.len(), TABLE_PAGE_SIZE);
        let last = TableViewData::from_summary(&summary, "Rig", 99, &history);
        assert_eq!(last.page, 2, "an out-of-range page clamps to the last");
        assert_eq!(last.rows.len(), 1);
    }

    #[test]
    fn model_detail_paginates_devices_and_maps_the_click_id() {
        let rows: Vec<(DeviceId, GroupSummary)> = (0..6_u64)
            .map(|i| {
                let mut name = String::from("dev-");
                units::format::push_int(&mut name, i);
                (DeviceId::new(name.clone()), grp(&name, 1, 1, 0))
            })
            .collect();
        let history = HashrateHistory::default();
        let first = ModelDetailViewData::from_summary("BMM 101", &rows, 0, &history);
        assert_eq!(first.device_count, 6);
        assert_eq!(first.page_count, 2, "6 devices, 4 per page");
        assert_eq!(first.rows.len(), MODEL_DETAIL_PAGE_SIZE);
        assert_eq!(first.rows[0].click_id, "device:dev-0");
        let last = ModelDetailViewData::from_summary("BMM 101", &rows, 9, &history);
        assert_eq!(last.page, 1, "an out-of-range page clamps to the last");
        assert_eq!(last.rows.len(), 2);
    }
}
