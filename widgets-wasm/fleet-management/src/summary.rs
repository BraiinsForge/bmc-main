// Copyright (C) 2026  Braiins Systems s.r.o.

use units::availability::Availability;
use units::units::{DegreeCelsius, JoulePerTeraHash, TeraHashPerSecond, Watt};

use crate::device::KnownDevice;
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
    let online_count = devices.len();
    let mut ok_count = 0;

    let mut hashrate_sum = 0.0_f64;
    let mut hashrate_any = false;
    let mut power_sum = 0.0_f64;
    let mut power_any = false;

    // Efficiency ranges only over devices reporting BOTH a non-zero hashrate
    // and a power, so a device missing either input biases neither the
    // numerator nor the denominator.
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
            let h = f64::from(h);
            if h > 0.0 {
                eff_hashrate += h;
                eff_power += f64::from(p);
                eff_any = true;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::device::{DeviceFamily, DeviceId, DeviceIdentity, KnownDevice};
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
        let eff = raw(&group.efficiency).expect("efficiency available");
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
        let eff = raw(&group.efficiency).expect("efficiency available");
        assert!((eff - 30.0).abs() < 1e-6, "got {eff}");
        assert_eq!(raw(&group.power), Some(30.0));
        assert_eq!(raw(&group.hashrate), Some(6.0));
    }

    #[test]
    fn fold_excludes_zero_hashrate_from_efficiency_but_keeps_its_power() {
        let a = device(Some("M"), Some(full(1.0, 30.0, 60.0)));
        let b = device(Some("M"), Some(full(0.0, 17.0, 40.0)));
        let group = fold_group("M".to_owned(), &[&a, &b]);
        let eff = raw(&group.efficiency).expect("efficiency available");
        assert!((eff - 30.0).abs() < 1e-6, "got {eff}");
        assert_eq!(raw(&group.power), Some(47.0));
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
}
