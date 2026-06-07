// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;

use units::availability::Availability;
use units::units::{DegreeCelsius, JoulePerTeraHash, TeraHashPerSecond, Watt};

use crate::device::{DeviceFamily, DeviceList, KnownDevice};
use crate::telemetry::TelemetryReading;

const OK_HASHRATE_FLOOR_THS: f32 = 0.1;

#[must_use]
pub fn is_ok(reading: &TelemetryReading) -> bool {
    reading
        .current_hashrate_ths
        .is_some_and(|h| h > OK_HASHRATE_FLOOR_THS)
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
    pub online_count: usize,
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
    let online_count = devices.len();
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
        let Some(reading) = dev.telemetry.as_ref().map(|s| &s.reading) else {
            continue;
        };
        if is_ok(reading) {
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
        online_count,
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

#[must_use]
pub fn summarize(devices: &DeviceList, filters: &crate::filter::Filters) -> FleetSummary {
    let reachable: Vec<&KnownDevice> = devices
        .iter()
        .filter(|d| d.reachable && filters.is_visible(d.identity.family, d.model.as_ref()))
        .collect();

    // Key on family as well as model name so two families that happen to share
    // a display name (e.g. a "BMM"-class device running both BOS and uBOS) stay
    // separate groups instead of silently merging. Model-less devices share the
    // single family-agnostic "Unknown" catch-all.
    let mut partitions: BTreeMap<(Option<usize>, String), Vec<&KnownDevice>> = BTreeMap::new();
    for dev in &reachable {
        let key = dev.model.as_ref().map_or_else(
            || (None, UNKNOWN_GROUP.to_owned()),
            |m| (Some(dev.identity.family.index()), m.name.clone()),
        );
        partitions.entry(key).or_default().push(dev);
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

    let total = fold_group(TOTAL_LABEL.to_owned(), &reachable);
    FleetSummary { total, groups }
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
    fn ok_predicate_uses_the_hundred_gigahash_floor() {
        assert!(!is_ok(&reading(Some(0.1))), "exactly the floor is not ok");
        assert!(is_ok(&reading(Some(0.11))), "just above the floor is ok");
        assert!(!is_ok(&reading(None)), "no hashrate reading is not ok");
        assert!(!is_ok(&reading(Some(0.0))), "zero hashrate is not ok");
    }

    #[test]
    fn fold_sums_hashrate_and_power_and_counts() {
        let a = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let b = device(Some("M"), Some(full(2.0, 20.0, 50.0)));
        let group = fold_group("M".to_owned(), &[&a, &b]);
        assert_eq!(raw(&group.hashrate), Some(3.0));
        assert_eq!(raw(&group.power), Some(50.0));
        assert_eq!(group.online_count, 2);
        assert_eq!(group.ok_count, 2);
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
        assert_eq!(group.online_count, 1);
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
        assert_eq!(summary.groups[0].online_count, 2);
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
    fn unreachable_devices_contribute_to_nothing() {
        let l = list(&[
            ("a", Some("BMM 101"), Some(full(1.0, 30.0, 60.0)), true),
            ("b", Some("BMM 101"), Some(full(9.0, 90.0, 80.0)), false),
        ]);
        let summary = summarize(&l, &Filters::default());
        assert_eq!(summary.groups.len(), 1);
        assert_eq!(summary.groups[0].online_count, 1);
        assert_eq!(raw(&summary.total.hashrate), Some(1.0));
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
        assert_eq!(summary.total.online_count, 2);
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
}
