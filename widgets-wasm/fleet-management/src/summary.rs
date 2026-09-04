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

use std::collections::BTreeMap;

use bmc_wasm_sdk::types::{ElectricPower, Hashrate, MiningEfficiency, Temperature};

use crate::device::{DeviceFamily, DeviceId, DeviceList, KnownDevice, PollFailure};
use crate::telemetry::{DeviceTemp, TelemetryReading};

/// A miner reading below this fraction of its nominal hashrate is not-ok.
const OK_NOMINAL_FRACTION: f32 = 0.2;
/// Fallback floor when no nominal is known: at least mining something.
const OK_HASHRATE_FLOOR_THS: f32 = 0.1;

/// Ok when the current hashrate is ≥ 20% of nominal (or, with no nominal known,
/// above a small floor). A present device with no reading is not ok.
#[must_use]
pub fn is_ok(reading: &TelemetryReading, nominal_ths: Option<f32>) -> bool {
    let Some(current) = reading.current_hashrate_ths else {
        return false;
    };
    match nominal_ths {
        Some(nominal) if nominal > 0.0 => current >= nominal * OK_NOMINAL_FRACTION,
        _ => current > OK_HASHRATE_FLOOR_THS,
    }
}

/// A reported device's surfaced status: whether it is delivering usable data,
/// and if not, why. Rendered as an icon and label on the device rows and detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    /// Reachable and mining at or above the ok threshold.
    Ok,
    /// Reachable but under-performing — below 20% of nominal, or idle.
    Degraded,
    /// Not delivering telemetry, with no HTTP response at all.
    Unreachable,
    /// Not delivering telemetry: the API answered with an error (e.g. 503).
    ApiError,
    /// Present and answering, but the login was rejected — check credentials.
    AuthError,
}

/// Derive a device's [`DeviceStatus`] from its liveness and last failure. A
/// reachable device is `Ok`/`Degraded` by the 20% rule against its nominal
/// (reading's, else the model catalog's); an unreachable one is `ApiError` when
/// its last pass got an HTTP error, otherwise `Unreachable`.
#[must_use]
pub fn device_status(device: &KnownDevice) -> DeviceStatus {
    if device.reachable {
        let reading = device.telemetry.as_ref().map(|s| &s.reading);
        let nominal = reading
            .and_then(|r| r.nominal_hashrate_ths)
            .or_else(|| device.model.as_ref().and_then(|m| m.nominal_hashrate_ths));
        if reading.is_some_and(|r| is_ok(r, nominal)) {
            DeviceStatus::Ok
        } else {
            DeviceStatus::Degraded
        }
    } else {
        match device.last_failure {
            Some(PollFailure::ApiError) => DeviceStatus::ApiError,
            Some(PollFailure::AuthError) => DeviceStatus::AuthError,
            Some(PollFailure::Unreachable) | None => DeviceStatus::Unreachable,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupSummary {
    pub label: String,
    pub family: Option<DeviceFamily>,
    pub hashrate: Option<Hashrate>,
    pub power: Option<ElectricPower>,
    pub efficiency: Option<MiningEfficiency>,
    pub min_temperature: Option<Temperature>,
    pub avg_temperature: Option<Temperature>,
    pub max_temperature: Option<Temperature>,
    pub total_count: usize,
    pub ok_count: usize,
    /// Not-reachable devices, excluding auth failures (counted separately).
    /// The reachable-but-not-`ok` remainder is degraded.
    pub off_count: usize,
    /// Devices present but rejecting the login — "not authenticating".
    pub auth_error_count: usize,
}

fn fold_group(label: String, devices: &[&KnownDevice]) -> GroupSummary {
    // A model group shares one family; the "Unknown" catch-all may mix families,
    // so it carries none.
    let family = if label == UNKNOWN_GROUP {
        None
    } else {
        devices.first().map(|d| d.identity.family)
    };
    let total_count = devices.len();
    let mut ok_count = 0;
    let mut off_count = 0;
    let mut auth_error_count = 0;

    let mut hashrate_sum = 0.0_f64;
    let mut hashrate_any = false;
    let mut power_sum = 0.0_f64;
    let mut power_any = false;

    // Efficiency counts only devices actually mining, with both readings.
    // An idle (zero-hashrate) device's power would inflate the group J/TH,
    // and a missing-power device's free hashrate would flatter it — excluded.
    let mut eff_hashrate = 0.0_f64;
    let mut eff_power = 0.0_f64;
    let mut eff_any = false;

    let mut temp_sum = 0.0_f64;
    let mut temp_count = 0_usize;
    let mut temp_min = f64::MAX;
    let mut temp_max = f64::MIN;

    for dev in devices {
        // An unreachable device is unknown, not zero: we have no reading,
        // so it contributes to no sum — a group that is entirely down
        // reads N/A across hashrate, power, efficiency, and temperature
        // alike, never a false 0.
        //
        // A mixed group is unchanged (the down ones only ever added +0).
        if !dev.reachable {
            // Count auth failures apart from the offline devices, so they surface
            // as "not authenticating" — a prompt to check creds, not gone.
            if dev.last_failure == Some(PollFailure::AuthError) {
                auth_error_count += 1;
            } else {
                off_count += 1;
            }
            continue;
        }
        let Some(reading) = dev.telemetry.as_ref().map(|s| &s.reading) else {
            continue;
        };
        // Nominal for the 20% rule: the reading's (API), else the model's (catalog).
        let nominal = reading
            .nominal_hashrate_ths
            .or_else(|| dev.model.as_ref().and_then(|m| m.nominal_hashrate_ths));
        if is_ok(reading, nominal) {
            ok_count += 1;
        }
        if let Some(h) = reading.current_hashrate_ths {
            hashrate_sum += f64::from(h);
            hashrate_any = true;
        }
        if let Some(p) = reading.power_w {
            power_sum += f64::from(p);
            power_any = true;
        }
        if let (Some(h), Some(p)) = (reading.current_hashrate_ths, reading.power_w)
            && h > 0.0
        {
            eff_hashrate += f64::from(h);
            eff_power += f64::from(p);
            eff_any = true;
        }
        // Group spread: min of device mins, max of maxes, mean of avgs.
        if let Some((lo, avg, hi)) = reading.temperature.map(DeviceTemp::as_range) {
            temp_sum += avg.as_celsius();
            temp_count += 1;
            temp_min = temp_min.min(lo.as_celsius());
            temp_max = temp_max.max(hi.as_celsius());
        }
    }

    let hashrate = hashrate_any.then(|| Hashrate::from_terahashes_per_second(hashrate_sum));
    let power = power_any.then(|| ElectricPower::from_watts(power_sum));
    let efficiency = (eff_any && eff_hashrate > 0.0)
        .then(|| MiningEfficiency::from_joules_per_terahash(eff_power / eff_hashrate));

    let (min_temperature, avg_temperature, max_temperature) = if temp_count > 0 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "fleet device counts stay within f64's exact integer range"
        )]
        let avg = temp_sum / temp_count as f64;
        (
            Some(Temperature::from_celsius(temp_min)),
            Some(Temperature::from_celsius(avg)),
            Some(Temperature::from_celsius(temp_max)),
        )
    } else {
        (None, None, None)
    };

    GroupSummary {
        label,
        family,
        hashrate,
        power,
        efficiency,
        min_temperature,
        avg_temperature,
        max_temperature,
        total_count,
        ok_count,
        off_count,
        auth_error_count,
    }
}

const UNKNOWN_GROUP: &str = "Unknown";
const TOTAL_LABEL: &str = "Total";

fn family_rank(family: DeviceFamily) -> u8 {
    match family {
        DeviceFamily::Ubos => 0,
        DeviceFamily::Bos => 1,
        DeviceFamily::Bitaxe => 2,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FleetSummary {
    pub total: GroupSummary,
    pub groups: Vec<GroupSummary>,
}

/// The grouping key `summarize` partitions on: family index plus model name,
/// with model-less devices in the family-agnostic catch-all.
fn partition_key(dev: &KnownDevice) -> (Option<usize>, &str) {
    dev.model.as_ref().map_or((None, UNKNOWN_GROUP), |m| {
        (Some(dev.identity.family.index()), m.name.as_str())
    })
}

#[must_use]
pub fn summarize(devices: &DeviceList, filters: &crate::filter::Filters) -> FleetSummary {
    // Only positively identified devices enter the report: AxeOS/uBOS are
    // confirmed at discovery, a base-type BOS only once it answers a poll — so a
    // non-miner `_http._tcp` responder never counts. A confirmed device then folds
    // regardless of reachability (an unreachable one contributes no reading, see
    // `fold_group`); operator model/family filters hide it further.
    let visible: Vec<&KnownDevice> = devices
        .iter()
        .filter(|d| d.is_reported() && filters.is_visible(d.identity.family))
        .collect();

    // Key on family as well as model name so two families sharing a display name
    // don't merge. Model-less devices share the family-agnostic "Unknown" group.
    let mut partitions: BTreeMap<(Option<usize>, &str), Vec<&KnownDevice>> = BTreeMap::new();
    for dev in &visible {
        partitions.entry(partition_key(dev)).or_default().push(dev);
    }

    // Own the label once per group here, not once per device in `partition_key`.
    let mut groups: Vec<GroupSummary> = partitions
        .into_iter()
        .map(|((_, label), devs)| fold_group(label.to_owned(), &devs))
        .collect();
    // Order by family (uBOS, BOS, Bitaxe), alphabetically by model name within
    // a family, with the family-less "Unknown" group pinned last.
    groups.sort_by(|a, b| {
        (
            a.label == UNKNOWN_GROUP,
            a.family.map(family_rank),
            a.label.as_str(),
        )
            .cmp(&(
                b.label == UNKNOWN_GROUP,
                b.family.map(family_rank),
                b.label.as_str(),
            ))
    });

    let total = fold_group(TOTAL_LABEL.to_owned(), &visible);
    FleetSummary { total, groups }
}

/// Per-device rows (id + folded single-device group) for the drilled-into model,
/// sorted by display name (ASCII-case-insensitive, to keep Unicode tables out of
/// the binary). Same filters as the summary, so the family-spanning Unknown
/// catch-all can't resurrect hidden devices.
#[must_use]
pub fn model_detail_rows(
    devices: &DeviceList,
    filters: &crate::filter::Filters,
    family: Option<DeviceFamily>,
    label: &str,
    resolve: impl Fn(&KnownDevice) -> String,
) -> Vec<(DeviceId, GroupSummary, DeviceStatus)> {
    let family_index = family.map(DeviceFamily::index);
    let mut rows: Vec<(DeviceId, GroupSummary, DeviceStatus)> = devices
        .iter()
        .filter(|dev| {
            let (fam, lab) = partition_key(dev);
            dev.is_reported()
                && fam == family_index
                && lab == label
                && filters.is_visible(dev.identity.family)
        })
        // fold_group's "Unknown"-label family sentinel may misfire on a
        // device display-named "Unknown"; model-detail rows never read `family`.
        .map(|dev| {
            (
                dev.identity.id.clone(),
                fold_group(resolve(dev), &[dev]),
                device_status(dev),
            )
        })
        .collect();
    rows.sort_by(|a, b| {
        a.1.label
            .to_ascii_lowercase()
            .cmp(&b.1.label.to_ascii_lowercase())
            .then_with(|| a.1.label.cmp(&b.1.label))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::device::{DeviceFamily, DeviceId, DeviceIdentity, KnownDevice, Membership};
    use crate::filter::Filters;
    use crate::telemetry::TelemetrySnapshot;

    fn device(model: Option<&str>, reading: Option<TelemetryReading>) -> KnownDevice {
        KnownDevice {
            identity: DeviceIdentity {
                id: DeviceId::new("d"),
                family: DeviceFamily::Bos,
                name: "d".to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
            },
            model: model.map(|name| crate::model::MinerModel {
                id: "id".to_owned(),
                name: name.to_owned(),
                chip_type: None,
                chip_count: None,
                nominal_hashrate_ths: None,
            }),
            telemetry: reading.map(|reading| TelemetrySnapshot { reading }),
            reachable: true,
            consecutive_failures: 0,
            membership: Membership::Confirmed,
            last_failure: None,
            unreachable_since: None,
        }
    }

    fn full(hashrate: f32, power: f32, temp: f32) -> TelemetryReading {
        TelemetryReading {
            current_hashrate_ths: Some(hashrate),
            power_w: Some(power),
            temperature: Some(DeviceTemp::Single(Temperature::from_celsius(f64::from(
                temp,
            )))),
            ..TelemetryReading::default()
        }
    }

    /// Extract a metric's canonical-unit magnitude for assertions.
    trait Canonical {
        fn canonical(self) -> f64;
    }
    impl Canonical for Hashrate {
        fn canonical(self) -> f64 {
            self.as_terahashes_per_second()
        }
    }
    impl Canonical for ElectricPower {
        fn canonical(self) -> f64 {
            self.as_watts()
        }
    }
    impl Canonical for Temperature {
        fn canonical(self) -> f64 {
            self.as_celsius()
        }
    }
    impl Canonical for MiningEfficiency {
        fn canonical(self) -> f64 {
            self.as_joules_per_terahash()
        }
    }

    fn raw<Q: Canonical + Copy>(value: Option<Q>) -> Option<f64> {
        value.map(Canonical::canonical)
    }

    fn reading(hashrate: Option<f32>) -> TelemetryReading {
        TelemetryReading {
            current_hashrate_ths: hashrate,
            ..TelemetryReading::default()
        }
    }

    #[test]
    fn ok_predicate_without_nominal_uses_the_floor() {
        assert!(
            !is_ok(&reading(Some(0.1)), None),
            "exactly the floor is not ok"
        );
        assert!(
            is_ok(&reading(Some(0.11)), None),
            "just above the floor is ok"
        );
        assert!(
            !is_ok(&reading(None), None),
            "no hashrate reading is not ok"
        );
        assert!(!is_ok(&reading(Some(0.0)), None), "zero hashrate is not ok");
    }

    #[test]
    fn ok_predicate_with_nominal_uses_the_twenty_percent_rule() {
        // Nominal 100 TH/s → ok at or above 20 TH/s, not-ok below.
        assert!(
            !is_ok(&reading(Some(8.0)), Some(100.0)),
            "8 is below 20% of 100"
        );
        assert!(
            is_ok(&reading(Some(25.0)), Some(100.0)),
            "25 is above 20% of 100"
        );
        assert!(
            is_ok(&reading(Some(100.0)), Some(100.0)),
            "full nominal is ok"
        );
        assert!(
            !is_ok(&reading(None), Some(100.0)),
            "no reading is not ok even with a nominal"
        );
    }

    #[test]
    fn fold_sums_hashrate_and_power_and_counts() {
        let a = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let b = device(Some("M"), Some(full(2.0, 20.0, 50.0)));
        let group = fold_group("M".to_owned(), &[&a, &b]);
        assert_eq!(raw(group.hashrate), Some(3.0));
        assert_eq!(raw(group.power), Some(50.0));
        assert_eq!(group.total_count, 2);
        assert_eq!(group.ok_count, 2);
    }

    #[test]
    fn fold_counts_a_straggler_below_twenty_percent_of_nominal_as_not_ok() {
        // 8 vs 100 TH/s nominal is below the 20% floor → not ok; 100 is ok.
        let straggler = device(
            Some("M"),
            Some(TelemetryReading {
                current_hashrate_ths: Some(8.0),
                nominal_hashrate_ths: Some(100.0),
                ..TelemetryReading::default()
            }),
        );
        let healthy = device(
            Some("M"),
            Some(TelemetryReading {
                current_hashrate_ths: Some(100.0),
                nominal_hashrate_ths: Some(100.0),
                ..TelemetryReading::default()
            }),
        );
        let group = fold_group("M".to_owned(), &[&straggler, &healthy]);
        assert_eq!(group.total_count, 2);
        assert_eq!(group.ok_count, 1, "only the healthy miner is ok");
    }

    #[test]
    fn fold_uses_the_model_catalog_nominal_when_the_reading_lacks_one() {
        // uBOS-style: no reading nominal, but the model carries a catalog one
        // (100 TH/s) → the 20% rule applies via the model (8 straggles, 30 ok).
        let mut straggler = device(Some("M"), Some(reading(Some(8.0))));
        straggler
            .model
            .as_mut()
            .expect("BUG: device built with a model")
            .nominal_hashrate_ths = Some(100.0);
        let mut healthy = device(Some("M"), Some(reading(Some(30.0))));
        healthy
            .model
            .as_mut()
            .expect("BUG: device built with a model")
            .nominal_hashrate_ths = Some(100.0);
        let group = fold_group("M".to_owned(), &[&straggler, &healthy]);
        assert_eq!(
            group.ok_count, 1,
            "only the healthy device clears 20% of the catalog nominal"
        );
    }

    #[test]
    fn fold_efficiency_is_total_power_over_total_hashrate() {
        // Two devices of very different efficiency; the group J/TH must be
        // Σpower / Σhashrate (50/3 ≈ 16.67), not the mean of the per-device
        // ratios ((30/1 + 20/2)/2 = 20).
        let a = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let b = device(Some("M"), Some(full(2.0, 20.0, 50.0)));
        let group = fold_group("M".to_owned(), &[&a, &b]);
        let eff = raw(group.efficiency).expect("BUG: efficiency available");
        assert!((eff - 50.0 / 3.0).abs() < 1e-6, "got {eff}");
    }

    #[test]
    fn fold_excludes_missing_power_from_efficiency_but_not_from_display_power() {
        // Device b has hashrate but no power: it must not enter the efficiency
        // hashrate denominator (which would drag J/TH down), yet its absence
        // leaves the display power sum to device a alone.
        let a = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let b = device(
            Some("M"),
            Some(TelemetryReading {
                current_hashrate_ths: Some(5.0),
                ..TelemetryReading::default()
            }),
        );
        let group = fold_group("M".to_owned(), &[&a, &b]);
        let eff = raw(group.efficiency).expect("BUG: efficiency available");
        assert!((eff - 30.0).abs() < 1e-6, "got {eff}");
        assert_eq!(raw(group.power), Some(30.0));
        assert_eq!(raw(group.hashrate), Some(6.0));
    }

    #[test]
    fn fold_excludes_idle_device_from_efficiency_but_keeps_its_power() {
        // Idle b (0 TH/s, 17 W): its power stays out of the efficiency (30 J/TH,
        // the active miner alone) but still shows in the group's total (47 W).
        let a = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let b = device(Some("M"), Some(full(0.0, 17.0, 40.0)));
        let group = fold_group("M".to_owned(), &[&a, &b]);
        let eff = raw(group.efficiency).expect("BUG: efficiency available");
        assert!((eff - 30.0).abs() < 1e-6, "got {eff}");
        assert_eq!(raw(group.power), Some(47.0));
    }

    #[test]
    fn fold_efficiency_ignores_an_idle_devices_power() {
        // Reported case: idle miner (0 TH/s, 8 W) + active miner (1 TH/s, 32 W).
        // The idle 8 W is excluded, so efficiency is the active miner's 32 J/TH,
        // not the naive total-power / total-hashrate (which would read 40).
        let idle = device(Some("M"), Some(full(0.0, 8.0, 35.0)));
        let active = device(Some("M"), Some(full(1.0, 32.0, 60.0)));
        let group = fold_group("M".to_owned(), &[&idle, &active]);
        let eff = raw(group.efficiency).expect("BUG: efficiency available");
        assert!((eff - 32.0).abs() < 1e-6, "got {eff}");
        assert_eq!(
            raw(group.power),
            Some(40.0),
            "idle power still shows in the group total"
        );
    }

    #[test]
    fn fold_temperature_is_min_mean_max() {
        let a = device(Some("M"), Some(full(1.0, 30.0, 40.0)));
        let b = device(Some("M"), Some(full(1.0, 30.0, 50.0)));
        let c = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let group = fold_group("M".to_owned(), &[&a, &b, &c]);
        assert_eq!(raw(group.min_temperature), Some(40.0));
        assert_eq!(raw(group.avg_temperature), Some(50.0));
        assert_eq!(raw(group.max_temperature), Some(60.0));
    }

    #[test]
    fn fold_all_missing_is_unavailable() {
        let a = device(Some("M"), None);
        let group = fold_group("M".to_owned(), &[&a]);
        assert_eq!(group.hashrate, None);
        assert_eq!(group.power, None);
        assert_eq!(group.efficiency, None);
        assert_eq!(group.min_temperature, None);
        assert_eq!(group.total_count, 1);
        assert_eq!(group.ok_count, 0);
    }

    use crate::device::DeviceList;

    fn list(entries: &[(&str, Option<&str>, Option<TelemetryReading>, bool)]) -> DeviceList {
        let mut list = DeviceList::new();
        for (i, (name, group, reading, reachable)) in entries.iter().enumerate() {
            let mut id_str = String::from("dev-");
            units::format::push_int(&mut id_str, i as u64);
            let id = DeviceId::new(id_str);
            let identity = DeviceIdentity {
                id: id.clone(),
                family: DeviceFamily::Bos,
                name: (*name).to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
            };
            list.upsert(identity);
            if let Some(group) = group {
                list.apply_model(
                    &id,
                    crate::model::MinerModel {
                        id: "id".to_owned(),
                        name: (*group).to_owned(),
                        chip_type: None,
                        chip_count: None,
                        nominal_hashrate_ths: None,
                    },
                );
            }
            // apply_telemetry sets reachability; a reachable device with no
            // reading stays as upserted (reachable, telemetry None).
            if let Some(reading) = reading {
                list.apply_telemetry(&id, reading.clone(), *reachable);
            }
        }
        list
    }

    #[test]
    fn groups_split_by_model_name_with_unknown_last() {
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            (
                "b",
                Some("Bitaxe Gamma 601"),
                Some(full(1.0, 17.0, 55.0)),
                true,
            ),
            ("c", None, Some(full(0.5, 10.0, 45.0)), true),
            ("d", Some("BMM 101"), Some(full(1.0, 33.0, 62.0)), true),
        ]);
        let summary = summarize(&l, &Filters::default());
        let labels: Vec<&str> = summary.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["BMM 101", "Bitaxe Gamma 601", "Unknown"]);
        assert_eq!(summary.groups[0].total_count, 2);
    }

    fn discovered(id: &str, family: DeviceFamily, host: &str, port: u16) -> DeviceIdentity {
        DeviceIdentity {
            id: DeviceId::new(id),
            family,
            name: id.to_owned(),
            host: host.to_owned(),
            port,
        }
    }

    #[test]
    fn a_candidate_is_excluded_until_it_answers_a_poll() {
        // A base-type sighting that hasn't answered (a non-miner `_http._tcp`
        // responder, or a real BOS mid-boot) is a candidate — hidden from the
        // report until an answered poll confirms it.
        let mut list = DeviceList::new();
        let id = DeviceId::new("bos/x");
        list.upsert(discovered("bos/x", DeviceFamily::Bos, "192.168.1.136", 80));
        assert_eq!(
            summarize(&list, &Filters::default()).total.total_count,
            0,
            "an unconfirmed candidate must not be counted"
        );
        list.record_pass(&id, reading(Some(5.0)), true);
        assert_eq!(
            summarize(&list, &Filters::default()).total.total_count,
            1,
            "answering a poll admits it to the report"
        );
    }

    #[test]
    fn an_identified_but_unanswered_device_is_still_reported() {
        // A positively family-identified device (uBOS on its own type) is shown
        // even before it answers, so an erroring miner surfaces rather than hides.
        let mut list = DeviceList::new();
        let id = DeviceId::new("ubos/ubos-01");
        list.upsert(discovered(
            "ubos/ubos-01",
            DeviceFamily::Ubos,
            "10.0.0.4",
            8080,
        ));
        assert_eq!(summarize(&list, &Filters::default()).total.total_count, 0);
        list.identify(&id);
        assert_eq!(
            summarize(&list, &Filters::default()).total.total_count,
            1,
            "an identified device is reported even with no telemetry"
        );
    }

    #[test]
    fn device_status_reflects_liveness_and_failure_kind() {
        use crate::device::PollFailure;
        use crate::telemetry::TelemetrySnapshot;

        let base = || KnownDevice {
            identity: discovered("d", DeviceFamily::Bos, "10.0.0.1", 80),
            model: None,
            telemetry: None,
            reachable: false,
            consecutive_failures: 0,
            membership: Membership::Identified,
            last_failure: None,
            unreachable_since: None,
        };
        let with_reading = |ths: f32| {
            let mut d = base();
            d.reachable = true;
            d.telemetry = Some(TelemetrySnapshot {
                reading: reading(Some(ths)),
            });
            d
        };

        assert_eq!(device_status(&with_reading(5.0)), DeviceStatus::Ok);
        assert_eq!(device_status(&with_reading(0.0)), DeviceStatus::Degraded);

        let mut api_error = base();
        api_error.last_failure = Some(PollFailure::ApiError);
        assert_eq!(device_status(&api_error), DeviceStatus::ApiError);

        let mut no_response = base();
        no_response.last_failure = Some(PollFailure::Unreachable);
        assert_eq!(device_status(&no_response), DeviceStatus::Unreachable);

        // A freshly identified device with no recorded failure reads as unreachable.
        assert_eq!(device_status(&base()), DeviceStatus::Unreachable);
    }

    #[test]
    fn groups_order_by_family_ubos_then_bos_then_bitaxe() {
        // Alphabetically the labels are BMM, Bitaxe, UMM; ordering by family
        // must override that to uBOS, then BOS, then Bitaxe.
        let specs = [
            ("a", DeviceFamily::Bitaxe, "Bitaxe Gamma 601"),
            ("b", DeviceFamily::Bos, "BMM 101"),
            ("c", DeviceFamily::Ubos, "UMM 200"),
        ];
        let mut l = DeviceList::new();
        for (i, (name, family, model_name)) in specs.iter().enumerate() {
            let mut id_str = String::from("dev-");
            units::format::push_int(&mut id_str, i as u64);
            let id = DeviceId::new(id_str);
            l.upsert(DeviceIdentity {
                id: id.clone(),
                family: *family,
                name: (*name).to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
            });
            l.apply_model(
                &id,
                crate::model::MinerModel {
                    id: "id".to_owned(),
                    name: (*model_name).to_owned(),
                    chip_type: None,
                    chip_count: None,
                    nominal_hashrate_ths: None,
                },
            );
            l.apply_telemetry(&id, full(1.0, 30.0, 60.0), true);
        }
        let summary = summarize(&l, &Filters::default());
        let labels: Vec<&str> = summary.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["UMM 200", "BMM 101", "Bitaxe Gamma 601"]);
    }

    #[test]
    fn same_model_name_under_two_families_does_not_merge() {
        // A display name shared across families must yield one group per family,
        // each carrying its own family, not a single silently-merged group.
        let specs = [
            ("a", DeviceFamily::Bos, "BMM 101"),
            ("b", DeviceFamily::Ubos, "BMM 101"),
        ];
        let mut l = DeviceList::new();
        for (i, (name, family, model_name)) in specs.iter().enumerate() {
            let mut id_str = String::from("dev-");
            units::format::push_int(&mut id_str, i as u64);
            let id = DeviceId::new(id_str);
            l.upsert(DeviceIdentity {
                id: id.clone(),
                family: *family,
                name: (*name).to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
            });
            l.apply_model(
                &id,
                crate::model::MinerModel {
                    id: "id".to_owned(),
                    name: (*model_name).to_owned(),
                    chip_type: None,
                    chip_count: None,
                    nominal_hashrate_ths: None,
                },
            );
            l.apply_telemetry(&id, full(1.0, 30.0, 60.0), true);
        }
        let summary = summarize(&l, &Filters::default());
        assert_eq!(summary.groups.len(), 2, "must not merge across families");
        assert!(summary.groups.iter().all(|g| g.label == "BMM 101"));
        assert_eq!(
            summary.groups.iter().filter_map(|g| g.family).count(),
            2,
            "each group carries its own family"
        );
        assert_eq!(
            summary.groups[0].family,
            Some(DeviceFamily::Ubos),
            "uBOS ranks before BOS"
        );
    }

    #[test]
    fn unreachable_device_is_counted_but_excluded_from_the_sums() {
        // The unreachable device carries a (stale) reading but contributes to no
        // sum: the group total still counts it, yet hashrate/power/temp reflect
        // only the reachable miner — never the down one folded in as a zero.
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("BMM 101"), Some(full(9.0, 90.0, 80.0)), false),
        ]);
        let summary = summarize(&l, &Filters::default());
        assert_eq!(summary.groups.len(), 1);
        let g = &summary.groups[0];
        assert_eq!(g.total_count, 2, "both devices are known and counted");
        assert_eq!(g.ok_count, 1, "only the reachable miner is mining");
        assert_eq!(g.off_count, 1, "the unreachable device is off");
        assert_eq!(raw(g.hashrate), Some(1.0), "only the reachable miner's");
        assert_eq!(raw(g.power), Some(30.0), "only the reachable miner's");
        assert_eq!(
            raw(g.max_temperature),
            Some(60.0),
            "unreachable temp omitted"
        );
    }

    #[test]
    fn group_of_only_unreachable_devices_reads_unavailable_not_zero() {
        // An all-down group has no reading to fold, so every metric is N/A — a
        // false 0 would read as "mining at zero" rather than "we can't see them".
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(9.0, 90.0, 80.0)), false),
            ("b", Some("BMM 101"), Some(full(8.0, 80.0, 70.0)), false),
        ]);
        let summary = summarize(&l, &Filters::default());
        let g = &summary.groups[0];
        assert_eq!(g.total_count, 2);
        assert_eq!(g.ok_count, 0, "all not mining");
        assert_eq!(g.off_count, 2, "both devices are off");
        assert_eq!(g.hashrate, None, "no reading, not a false 0");
        assert_eq!(g.power, None, "no reading, not a false 0");
        assert_eq!(g.max_temperature, None);
    }

    #[test]
    fn reachable_idle_miner_keeps_its_power_and_is_not_ok() {
        // A reachable but idle miner (0 TH/s, real power) must keep its real
        // power: the zeroing keys on `reachable`, never on a zero/absent reading.
        let l = list(&[("a", Some("BMM 101"), Some(full(0.0, 8.0, 40.0)), true)]);
        let summary = summarize(&l, &Filters::default());
        let g = &summary.groups[0];
        assert_eq!(g.total_count, 1);
        assert_eq!(g.ok_count, 0, "0 TH/s is below the mining floor");
        assert_eq!(g.off_count, 0, "reachable idle is degraded, not off");
        assert_eq!(raw(g.power), Some(8.0), "reachable idle power is preserved");
        assert_eq!(raw(g.max_temperature), Some(40.0));
    }

    #[test]
    fn total_additive_fields_equal_the_group_sums() {
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            (
                "b",
                Some("Bitaxe Gamma 601"),
                Some(full(2.0, 20.0, 40.0)),
                true,
            ),
        ]);
        let summary = summarize(&l, &Filters::default());
        assert_eq!(raw(summary.total.hashrate), Some(3.0));
        assert_eq!(raw(summary.total.power), Some(50.0));
        assert_eq!(summary.total.total_count, 2);
        assert_eq!(summary.total.ok_count, 2);
    }

    #[test]
    fn total_temperature_is_global_min_max_and_mean() {
        // Group A: temps 40, 60 (mean 50). Group B: temp 90.
        // total min=40, max=90, mean=(40+60+90)/3=63.33 — NOT the mean of the
        // group means ((50+90)/2=70).
        let l = list(&[
            ("a", Some("A"), Some(full(1.0, 30.0, 40.0)), true),
            ("a2", Some("A"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("B"), Some(full(1.0, 30.0, 90.0)), true),
        ]);
        let summary = summarize(&l, &Filters::default());
        assert_eq!(raw(summary.total.min_temperature), Some(40.0));
        assert_eq!(raw(summary.total.max_temperature), Some(90.0));
        let avg = raw(summary.total.avg_temperature).expect("BUG: avg available");
        assert!((avg - (40.0 + 60.0 + 90.0) / 3.0).abs() < 1e-6, "got {avg}");
        assert!(
            (avg - 70.0).abs() > 1.0,
            "must not be the mean of group means"
        );
    }

    #[test]
    fn total_efficiency_is_recomputed_globally_not_averaged() {
        // Group A: 1 TH/s @ 30 W (30 J/TH). Group B: 3 TH/s @ 30 W (10 J/TH).
        // Global Σpower/Σhashrate = 60/4 = 15 J/TH — NOT the mean of the group
        // efficiencies ((30+10)/2 = 20).
        let l = list(&[
            ("a", Some("A"), Some(full(1.0, 30.0, 50.0)), true),
            ("b", Some("B"), Some(full(3.0, 30.0, 50.0)), true),
        ]);
        let summary = summarize(&l, &Filters::default());
        let eff = raw(summary.total.efficiency).expect("BUG: efficiency available");
        assert!((eff - 15.0).abs() < 1e-6, "got {eff}");
    }

    fn family_list(specs: &[(&str, DeviceFamily, &str, TelemetryReading)]) -> DeviceList {
        let mut l = DeviceList::new();
        for (i, (name, family, model_name, reading)) in specs.iter().enumerate() {
            let mut id_str = String::from("dev-");
            units::format::push_int(&mut id_str, i as u64);
            let id = DeviceId::new(id_str);
            l.upsert(DeviceIdentity {
                id: id.clone(),
                family: *family,
                name: (*name).to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
            });
            l.apply_model(
                &id,
                crate::model::MinerModel {
                    id: "id".to_owned(),
                    name: (*model_name).to_owned(),
                    chip_type: None,
                    chip_count: None,
                    nominal_hashrate_ths: None,
                },
            );
            l.apply_telemetry(&id, reading.clone(), true);
        }
        l
    }

    #[test]
    fn disabled_family_drops_its_groups_and_total_contribution() {
        // A fleet spanning BOS and Bitaxe; disabling Bitaxe must erase its
        // group from the breakdown and remove its hashrate/power from the total.
        let l = family_list(&[
            ("a", DeviceFamily::Bos, "BMM 101", full(1.0, 30.0, 60.0)),
            (
                "b",
                DeviceFamily::Bitaxe,
                "NerdQAxe++",
                full(4.0, 70.0, 55.0),
            ),
        ]);
        let filters = Filters {
            axeos_enabled: false,
        };
        let summary = summarize(&l, &filters);
        let labels: Vec<&str> = summary.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["BMM 101"], "the disabled family's group is gone");
        assert_eq!(raw(summary.total.hashrate), Some(1.0));
        assert_eq!(raw(summary.total.power), Some(30.0));
    }

    #[test]
    fn model_detail_rows_fold_each_device_separately() {
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("BMM 101"), Some(full(2.0, 20.0, 50.0)), true),
            ("c", Some("Other"), Some(full(9.0, 90.0, 70.0)), true),
        ]);
        let rows = model_detail_rows(
            &l,
            &Filters::default(),
            Some(DeviceFamily::Bos),
            "BMM 101",
            |d| d.identity.name.clone(),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1.label, "a");
        assert_eq!(raw(rows[0].1.hashrate), Some(1.0));
        assert_eq!(rows[0].1.total_count, 1);
        assert_eq!(rows[0].1.ok_count, 1);
        assert_eq!(raw(rows[1].1.hashrate), Some(2.0));
    }

    #[test]
    fn model_detail_rows_sort_by_display_name_case_insensitively() {
        let l = list(&[
            ("x", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("y", Some("BMM 101"), Some(full(2.0, 20.0, 50.0)), true),
        ]);
        let rows = model_detail_rows(
            &l,
            &Filters::default(),
            Some(DeviceFamily::Bos),
            "BMM 101",
            |d| {
                if d.identity.name == "x" {
                    "bravo".to_owned()
                } else {
                    "Alpha".to_owned()
                }
            },
        );
        let labels: Vec<&str> = rows.iter().map(|r| r.1.label.as_str()).collect();
        assert_eq!(labels, ["Alpha", "bravo"]);
    }

    #[test]
    fn model_detail_rows_for_the_unknown_group_take_modelless_devices() {
        let l = list(&[
            ("a", None, Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("BMM 101"), Some(full(2.0, 20.0, 50.0)), true),
        ]);
        let rows = model_detail_rows(&l, &Filters::default(), None, "Unknown", |d| {
            d.identity.name.clone()
        });
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.label, "a");
    }

    #[test]
    fn model_detail_rows_require_the_family_to_match() {
        let l = family_list(&[
            ("a", DeviceFamily::Bos, "BMM 101", full(1.0, 30.0, 60.0)),
            ("b", DeviceFamily::Ubos, "BMM 101", full(2.0, 20.0, 50.0)),
        ]);
        let rows = model_detail_rows(
            &l,
            &Filters::default(),
            Some(DeviceFamily::Bos),
            "BMM 101",
            |d| d.identity.name.clone(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.label, "a");
    }

    #[test]
    fn model_detail_rows_hide_a_disabled_familys_modelless_devices() {
        // The Unknown partition spans families; disabling AxeOS must hide its
        // model-less device from the drill-in even though the uBOS one keeps
        // the group alive in the summary.
        let mut l = DeviceList::new();
        for (i, (name, family)) in [("a", DeviceFamily::Bitaxe), ("b", DeviceFamily::Ubos)]
            .iter()
            .enumerate()
        {
            let mut id_str = String::from("dev-");
            units::format::push_int(&mut id_str, i as u64);
            let id = DeviceId::new(id_str);
            l.upsert(DeviceIdentity {
                id: id.clone(),
                family: *family,
                name: (*name).to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
            });
            l.apply_telemetry(&id, full(1.0, 30.0, 60.0), true);
        }
        let filters = Filters {
            axeos_enabled: false,
        };
        let rows = model_detail_rows(&l, &filters, None, "Unknown", |d| d.identity.name.clone());
        let labels: Vec<&str> = rows.iter().map(|r| r.1.label.as_str()).collect();
        assert_eq!(labels, ["b"], "the disabled family's device must not leak");
    }

    #[test]
    fn model_detail_row_of_an_unreachable_device_reads_unavailable() {
        let l = list(&[("a", Some("BMM 101"), Some(full(9.0, 90.0, 80.0)), false)]);
        let rows = model_detail_rows(
            &l,
            &Filters::default(),
            Some(DeviceFamily::Bos),
            "BMM 101",
            |d| d.identity.name.clone(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.hashrate, None, "no reading, not a false 0");
        assert_eq!(rows[0].1.power, None, "no reading, not a false 0");
        assert_eq!(rows[0].1.ok_count, 0);
        assert_eq!(rows[0].1.max_temperature, None);
    }
}
