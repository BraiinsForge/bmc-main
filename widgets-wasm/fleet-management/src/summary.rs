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

use units::availability::Availability;
use units::units::{DegreeCelsius, JoulePerTeraHash, TeraHashPerSecond, Watt};

use crate::device::{DeviceFamily, DeviceList, KnownDevice};
use crate::telemetry::TelemetryReading;

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

#[derive(Debug, Clone, PartialEq)]
pub struct GroupSummary {
    pub label: String,
    pub family: Option<DeviceFamily>,
    pub hashrate: Availability<TeraHashPerSecond>,
    pub power: Availability<Watt>,
    pub efficiency: Availability<JoulePerTeraHash>,
    pub min_temperature: Availability<DegreeCelsius>,
    pub avg_temperature: Availability<DegreeCelsius>,
    pub max_temperature: Availability<DegreeCelsius>,
    pub total_count: usize,
    pub ok_count: usize,
}

fn fold_group(label: String, devices: &[&KnownDevice]) -> GroupSummary {
    // A model group shares one family; the catch-all "Unknown" group may mix
    // families, so it carries none and is pinned last when ordering.
    let family = if label == UNKNOWN_GROUP {
        None
    } else {
        devices.first().map(|d| d.identity.family)
    };
    let total_count = devices.len();
    let mut ok_count = 0;

    let mut hashrate_sum = 0.0_f64;
    let mut hashrate_any = false;
    let mut power_sum = 0.0_f64;
    let mut power_any = false;

    // Efficiency ranges over devices reporting BOTH a hashrate (zero allowed)
    // and a power; a device missing power is excluded so unattributable free
    // hashrate cannot make efficiency look artificially good.
    let mut eff_hashrate = 0.0_f64;
    let mut eff_power = 0.0_f64;
    let mut eff_any = false;

    let mut temp_sum = 0.0_f64;
    let mut temp_count = 0_usize;
    let mut temp_min = f64::MAX;
    let mut temp_max = f64::MIN;

    for dev in devices {
        // An unreachable device folds as a zero producer: hashrate 0, power 0,
        // temperature absent. It is present in the sums (so the group reads
        // 0.00 TH/s / 0 W rather than N/A) but never mining, and it drops out
        // of the temperature range. Efficiency is left untouched: a 0/0 device
        // is a no-op on Σpower / Σhashrate. Keyed on reachability, not on the
        // reading, so a reachable idle miner (0 TH/s, real power) keeps it.
        if !dev.reachable {
            hashrate_any = true;
            power_any = true;
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
        if let (Some(h), Some(p)) = (reading.current_hashrate_ths, reading.power_w) {
            eff_hashrate += f64::from(h);
            eff_power += f64::from(p);
            eff_any = true;
        }
        if let Some(t) = reading.temperature_c {
            let t = f64::from(t);
            temp_sum += t;
            temp_count += 1;
            temp_min = temp_min.min(t);
            temp_max = temp_max.max(t);
        }
    }

    let hashrate = availability(hashrate_any, || TeraHashPerSecond(hashrate_sum));
    let power = availability(power_any, || Watt(power_sum));
    let efficiency = availability(eff_any && eff_hashrate > 0.0, || {
        JoulePerTeraHash(eff_power / eff_hashrate)
    });

    let (min_temperature, avg_temperature, max_temperature) = if temp_count > 0 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "fleet device counts stay within f64's exact integer range"
        )]
        let avg = temp_sum / temp_count as f64;
        (
            Availability::Available(DegreeCelsius(temp_min)),
            Availability::Available(DegreeCelsius(avg)),
            Availability::Available(DegreeCelsius(temp_max)),
        )
    } else {
        (
            Availability::Unavailable,
            Availability::Unavailable,
            Availability::Unavailable,
        )
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
    }
}

fn availability<Q>(present: bool, value: impl FnOnce() -> Q) -> Availability<Q> {
    if present {
        Availability::Available(value())
    } else {
        Availability::Unavailable
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
fn partition_key(dev: &KnownDevice) -> (Option<usize>, String) {
    dev.model.as_ref().map_or_else(
        || (None, UNKNOWN_GROUP.to_owned()),
        |m| (Some(dev.identity.family.index()), m.name.clone()),
    )
}

#[must_use]
pub fn summarize(devices: &DeviceList, filters: &crate::filter::Filters) -> FleetSummary {
    // Every known device is shown — avahi-discovered or manual — regardless of
    // reachability. An unreachable device is folded as a zero producer (see
    // `fold_group`) so it counts toward the group total without inflating its
    // metrics. Only the operator's model/family filters can hide a device.
    let visible: Vec<&KnownDevice> = devices
        .iter()
        .filter(|d| filters.is_visible(d.identity.family, d.model.as_ref()))
        .collect();

    // Key on family as well as model name so two families that happen to share
    // a display name (e.g. a "BMM"-class device running both BOS and uBOS) stay
    // separate groups instead of silently merging. Model-less devices share the
    // single family-agnostic "Unknown" catch-all.
    let mut partitions: BTreeMap<(Option<usize>, String), Vec<&KnownDevice>> = BTreeMap::new();
    for dev in &visible {
        partitions.entry(partition_key(dev)).or_default().push(dev);
    }

    let mut groups: Vec<GroupSummary> = partitions
        .into_iter()
        .map(|((_, label), devs)| fold_group(label, &devs))
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

/// Per-device rows for the drilled-into model group: each matching device
/// folded as a single-device group labeled by its resolved display name,
/// sorted by that name (ASCII-case-insensitive, keeping Unicode case tables
/// out of the wasm binary). Rows apply the same operator filters as the
/// summary, so a partition spanning families (the Unknown catch-all) cannot
/// resurrect hidden devices.
#[must_use]
pub fn detail_rows(
    devices: &DeviceList,
    filters: &crate::filter::Filters,
    family: Option<DeviceFamily>,
    label: &str,
    resolve: impl Fn(&KnownDevice) -> String,
) -> Vec<GroupSummary> {
    let family_index = family.map(DeviceFamily::index);
    let mut rows: Vec<GroupSummary> = devices
        .iter()
        .filter(|dev| {
            let (fam, lab) = partition_key(dev);
            fam == family_index
                && lab == label
                && filters.is_visible(dev.identity.family, dev.model.as_ref())
        })
        // fold_group's "Unknown"-label family sentinel may misfire on a
        // device display-named "Unknown"; detail rows never read `family`.
        .map(|dev| fold_group(resolve(dev), &[dev]))
        .collect();
    rows.sort_by(|a, b| {
        a.label
            .to_ascii_lowercase()
            .cmp(&b.label.to_ascii_lowercase())
            .then_with(|| a.label.cmp(&b.label))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::device::{DeviceFamily, DeviceId, DeviceIdentity, DeviceSource, KnownDevice};
    use crate::filter::Filters;
    use crate::telemetry::TelemetrySnapshot;
    use units::units::Quantity;

    fn device(model: Option<&str>, reading: Option<TelemetryReading>) -> KnownDevice {
        KnownDevice {
            identity: DeviceIdentity {
                id: DeviceId::new("d"),
                family: DeviceFamily::Bos,
                name: "d".to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
                source: DeviceSource::Discovered,
            },
            model: model.map(|name| crate::model::MinerModel {
                id: "id".to_owned(),
                name: name.to_owned(),
                chip_type: None,
                chip_count: None,
                nominal_hashrate_ths: None,
            }),
            telemetry: reading.map(|reading| TelemetrySnapshot {
                reading,
                refreshed_seq: 1,
            }),
            last_seen_seq: 1,
            reachable: true,
            consecutive_failures: 0,
        }
    }

    fn full(hashrate: f32, power: f32, temp: f32) -> TelemetryReading {
        TelemetryReading {
            current_hashrate_ths: Some(hashrate),
            power_w: Some(power),
            temperature_c: Some(temp),
            ..TelemetryReading::default()
        }
    }

    fn raw(value: &Availability<impl Quantity>) -> Option<f64> {
        value.as_option().map(|q| q.raw())
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
        assert_eq!(raw(&group.hashrate), Some(3.0));
        assert_eq!(raw(&group.power), Some(50.0));
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
        let eff = raw(&group.efficiency).expect("BUG: efficiency available");
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
        let eff = raw(&group.efficiency).expect("BUG: efficiency available");
        assert!((eff - 30.0).abs() < 1e-6, "got {eff}");
        assert_eq!(raw(&group.power), Some(30.0));
        assert_eq!(raw(&group.hashrate), Some(6.0));
    }

    #[test]
    fn fold_includes_powered_zero_hashrate_device_in_efficiency() {
        // Device b is idle (0 TH/s) but still drawing power: its 17 W enters
        // the efficiency numerator so the result matches total power / total hashrate.
        let a = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let b = device(Some("M"), Some(full(0.0, 17.0, 40.0)));
        let group = fold_group("M".to_owned(), &[&a, &b]);
        let eff = raw(&group.efficiency).expect("BUG: efficiency available");
        assert!((eff - 47.0).abs() < 1e-6, "got {eff}");
        assert_eq!(raw(&group.power), Some(47.0));
    }

    #[test]
    fn fold_efficiency_matches_total_power_over_total_hashrate_with_idle_device() {
        // Reported case: idle miner (0 TH/s, 8 W) + active miner (1 TH/s, 32 W).
        // Headline: 40 W total, 1 TH/s total. Efficiency must be 40 J/TH.
        let idle = device(Some("M"), Some(full(0.0, 8.0, 35.0)));
        let active = device(Some("M"), Some(full(1.0, 32.0, 60.0)));
        let group = fold_group("M".to_owned(), &[&idle, &active]);
        let total_power = raw(&group.power).expect("BUG: power available");
        let total_hashrate = raw(&group.hashrate).expect("BUG: hashrate available");
        let eff = raw(&group.efficiency).expect("BUG: efficiency available");
        assert!((eff - 40.0).abs() < 1e-6, "got {eff}");
        assert!(
            (eff - total_power / total_hashrate).abs() < 1e-6,
            "eff must equal total_power/total_hashrate"
        );
    }

    #[test]
    fn fold_temperature_is_min_mean_max() {
        let a = device(Some("M"), Some(full(1.0, 30.0, 40.0)));
        let b = device(Some("M"), Some(full(1.0, 30.0, 50.0)));
        let c = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let group = fold_group("M".to_owned(), &[&a, &b, &c]);
        assert_eq!(raw(&group.min_temperature), Some(40.0));
        assert_eq!(raw(&group.avg_temperature), Some(50.0));
        assert_eq!(raw(&group.max_temperature), Some(60.0));
    }

    #[test]
    fn fold_all_missing_is_unavailable() {
        let a = device(Some("M"), None);
        let group = fold_group("M".to_owned(), &[&a]);
        assert_eq!(group.hashrate, Availability::Unavailable);
        assert_eq!(group.power, Availability::Unavailable);
        assert_eq!(group.efficiency, Availability::Unavailable);
        assert_eq!(group.min_temperature, Availability::Unavailable);
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
                source: DeviceSource::Discovered,
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
                list.apply_telemetry(&id, *reading, *reachable);
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
                source: DeviceSource::Discovered,
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
                source: DeviceSource::Discovered,
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
    fn unreachable_device_counts_in_total_as_a_zero_producer() {
        // The unreachable device still carries a (stale) full reading, but it
        // must fold as hashrate 0 / power 0 / temp None: it counts toward the
        // group total, is not mining, and adds nothing to hashrate or power.
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("BMM 101"), Some(full(9.0, 90.0, 80.0)), false),
        ]);
        let summary = summarize(&l, &Filters::default());
        assert_eq!(summary.groups.len(), 1);
        let g = &summary.groups[0];
        assert_eq!(g.total_count, 2, "both devices are known and counted");
        assert_eq!(g.ok_count, 1, "only the reachable miner is mining");
        assert_eq!(raw(&g.hashrate), Some(1.0), "unreachable adds 0 hashrate");
        assert_eq!(raw(&g.power), Some(30.0), "unreachable adds 0 power");
        assert_eq!(
            raw(&g.max_temperature),
            Some(60.0),
            "unreachable temp omitted"
        );
    }

    #[test]
    fn group_of_only_unreachable_devices_reads_zero_not_unavailable() {
        // An all-down group shows 0.00 TH/s / 0 W (present zeros, so the red
        // status count is meaningful) and an unavailable temperature.
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(9.0, 90.0, 80.0)), false),
            ("b", Some("BMM 101"), Some(full(8.0, 80.0, 70.0)), false),
        ]);
        let summary = summarize(&l, &Filters::default());
        let g = &summary.groups[0];
        assert_eq!(g.total_count, 2);
        assert_eq!(g.ok_count, 0, "all not mining");
        assert_eq!(raw(&g.hashrate), Some(0.0), "present zero, not N/A");
        assert_eq!(raw(&g.power), Some(0.0), "present zero, not N/A");
        assert_eq!(g.max_temperature, Availability::Unavailable);
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
        assert_eq!(
            raw(&g.power),
            Some(8.0),
            "reachable idle power is preserved"
        );
        assert_eq!(raw(&g.max_temperature), Some(40.0));
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
        assert_eq!(raw(&summary.total.hashrate), Some(3.0));
        assert_eq!(raw(&summary.total.power), Some(50.0));
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
        assert_eq!(raw(&summary.total.min_temperature), Some(40.0));
        assert_eq!(raw(&summary.total.max_temperature), Some(90.0));
        let avg = raw(&summary.total.avg_temperature).expect("BUG: avg available");
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
        let eff = raw(&summary.total.efficiency).expect("BUG: efficiency available");
        assert!((eff - 15.0).abs() < 1e-6, "got {eff}");
    }

    #[test]
    fn blacklist_drops_a_model_group_and_excludes_it_from_totals() {
        // Two models present; blacklisting one must remove its breakdown group
        // and leave the total reflecting only the surviving model.
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("NerdQAxe++"), Some(full(4.0, 70.0, 55.0)), true),
        ]);
        let filters = Filters {
            blacklist: vec!["nerdqaxe".to_owned()],
            ..Default::default()
        };
        let summary = summarize(&l, &filters);
        let labels: Vec<&str> = summary.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["BMM 101"], "the blacklisted group is gone");
        assert_eq!(raw(&summary.total.hashrate), Some(1.0));
        assert_eq!(raw(&summary.total.power), Some(30.0));
    }

    #[test]
    fn whitelist_keeps_only_matching_model_groups() {
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("NerdQAxe++"), Some(full(4.0, 70.0, 55.0)), true),
        ]);
        let filters = Filters {
            whitelist: vec!["bmm101".to_owned()],
            ..Default::default()
        };
        let summary = summarize(&l, &filters);
        let labels: Vec<&str> = summary.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["BMM 101"], "only the whitelisted model survives");
        assert_eq!(raw(&summary.total.hashrate), Some(1.0));
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
                source: DeviceSource::Discovered,
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
            l.apply_telemetry(&id, *reading, true);
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
            ..Default::default()
        };
        let summary = summarize(&l, &filters);
        let labels: Vec<&str> = summary.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["BMM 101"], "the disabled family's group is gone");
        assert_eq!(raw(&summary.total.hashrate), Some(1.0));
        assert_eq!(raw(&summary.total.power), Some(30.0));
    }

    #[test]
    fn detail_rows_fold_each_device_separately() {
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("BMM 101"), Some(full(2.0, 20.0, 50.0)), true),
            ("c", Some("Other"), Some(full(9.0, 90.0, 70.0)), true),
        ]);
        let rows = detail_rows(
            &l,
            &Filters::default(),
            Some(DeviceFamily::Bos),
            "BMM 101",
            |d| d.identity.name.clone(),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "a");
        assert_eq!(raw(&rows[0].hashrate), Some(1.0));
        assert_eq!(rows[0].total_count, 1);
        assert_eq!(rows[0].ok_count, 1);
        assert_eq!(raw(&rows[1].hashrate), Some(2.0));
    }

    #[test]
    fn detail_rows_sort_by_display_name_case_insensitively() {
        let l = list(&[
            ("x", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("y", Some("BMM 101"), Some(full(2.0, 20.0, 50.0)), true),
        ]);
        let rows = detail_rows(
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
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Alpha", "bravo"]);
    }

    #[test]
    fn detail_rows_for_the_unknown_group_take_modelless_devices() {
        let l = list(&[
            ("a", None, Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("BMM 101"), Some(full(2.0, 20.0, 50.0)), true),
        ]);
        let rows = detail_rows(&l, &Filters::default(), None, "Unknown", |d| {
            d.identity.name.clone()
        });
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "a");
    }

    #[test]
    fn detail_rows_require_the_family_to_match() {
        let l = family_list(&[
            ("a", DeviceFamily::Bos, "BMM 101", full(1.0, 30.0, 60.0)),
            ("b", DeviceFamily::Ubos, "BMM 101", full(2.0, 20.0, 50.0)),
        ]);
        let rows = detail_rows(
            &l,
            &Filters::default(),
            Some(DeviceFamily::Bos),
            "BMM 101",
            |d| d.identity.name.clone(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "a");
    }

    #[test]
    fn detail_rows_hide_a_disabled_familys_modelless_devices() {
        // The Unknown partition spans families; disabling BOS must hide its
        // model-less device from the drill-in even though the uBOS one keeps
        // the group alive in the summary.
        let mut l = DeviceList::new();
        for (i, (name, family)) in [("a", DeviceFamily::Bos), ("b", DeviceFamily::Ubos)]
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
                source: DeviceSource::Discovered,
            });
            l.apply_telemetry(&id, full(1.0, 30.0, 60.0), true);
        }
        let filters = Filters {
            bos_enabled: false,
            ..Default::default()
        };
        let rows = detail_rows(&l, &filters, None, "Unknown", |d| d.identity.name.clone());
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["b"], "the disabled family's device must not leak");
    }

    #[test]
    fn detail_row_of_an_unreachable_device_reads_zero() {
        let l = list(&[("a", Some("BMM 101"), Some(full(9.0, 90.0, 80.0)), false)]);
        let rows = detail_rows(
            &l,
            &Filters::default(),
            Some(DeviceFamily::Bos),
            "BMM 101",
            |d| d.identity.name.clone(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(raw(&rows[0].hashrate), Some(0.0));
        assert_eq!(raw(&rows[0].power), Some(0.0));
        assert_eq!(rows[0].ok_count, 0);
        assert_eq!(rows[0].max_temperature, Availability::Unavailable);
    }
}
