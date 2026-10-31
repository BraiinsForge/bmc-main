// Copyright (C) 2026  Jan Novák
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Solar readings of a Venus OS system, and the MQTT topics they come from.
//!
//! Every Venus OS device publishes `N/<portal id>/<service>/<instance>/<path>` with a
//! `{"value": …}` payload. Solar comes from two kinds of device, any number
//! of each: MPPT solar chargers (DC-coupled) and PV inverters (AC-coupled,
//! e.g. Fronius). Venus OS already totals their live power under
//! `system/0`; their energy counters are summed here.

use std::collections::BTreeMap;

/// Days of per-day yield an MPPT keeps (`History/Daily/0` = today … `/30`).
pub const DAILY_HISTORY_DAYS: usize = 31;

/// Topic filters to subscribe to once the portal id is known.
#[must_use]
pub fn subscriptions(portal: &str) -> Vec<String> {
    [
        "full_publish_completed",
        "system/0/Dc/Pv/Power",
        "system/0/Ac/+/+/Power",
        "solarcharger/+/Yield/User",
        "solarcharger/+/History/Daily/+/Yield",
        "pvinverter/+/Ac/Energy/Forward",
    ]
    .iter()
    .map(|p| format!("N/{portal}/{p}"))
    .collect()
}

pub const DISCOVERY_FILTER: &str = "N/+/system/0/Serial";

/// The portal id from a discovery message (`N/<id>/system/0/Serial`).
#[must_use]
pub fn portal_from_serial_topic(topic: &str) -> Option<&str> {
    let rest = topic.strip_prefix("N/")?;
    let (portal, path) = rest.split_once('/')?;
    (path == "system/0/Serial" && !portal.is_empty()).then_some(portal)
}

/// The number in a `{"value": <number>}` payload; `None` for `null`,
/// an empty payload (Venus OS's way of saying a path disappeared) or a string.
#[must_use]
pub fn payload_value(payload: &[u8]) -> Option<f64> {
    let text = core::str::from_utf8(payload).ok()?;
    let at = text.find("\"value\"")? + "\"value\"".len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')))
        .unwrap_or(rest.len());
    rest[..end].parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Live state assembled from the device, keyed by device instance (MQTT)
/// or Modbus unit id.
#[derive(Default, Debug)]
pub struct Readings {
    dc_pv_w: Option<f64>,
    /// `system/0/Ac/<PvOnGrid|PvOnOutput|PvOnGenset>/<L1..L3>/Power`
    ac_pv_w: BTreeMap<String, f64>,
    mppt_lifetime_kwh: BTreeMap<u32, f64>,
    mppt_daily_kwh: BTreeMap<u32, [Option<f64>; DAILY_HISTORY_DAYS]>,
    pvinverter_lifetime_kwh: BTreeMap<u32, f64>,
    /// Set once every device has reported (MQTT: the broker says it has sent
    /// every value; Modbus: one full round of reads); counters summed before
    /// that may be missing a device.
    pub complete: bool,
}

impl Readings {
    /// Apply one message. Returns whether it touched anything this widget shows.
    pub fn apply(&mut self, portal: &str, topic: &str, payload: &[u8]) -> bool {
        let Some(path) = topic
            .strip_prefix("N/")
            .and_then(|t| t.strip_prefix(portal))
            .and_then(|t| t.strip_prefix('/'))
        else {
            return false;
        };
        if path == "full_publish_completed" {
            let was = self.complete;
            self.complete = true;
            return !was;
        }
        let value = payload_value(payload);
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            ["system", "0", "Dc", "Pv", "Power"] => self.set_dc_pv_w(value),
            ["system", "0", "Ac", kind, phase, "Power"] if kind.starts_with("PvOn") => {
                self.set_ac_pv_w(&format!("{kind}/{phase}"), value);
            }
            ["solarcharger", inst, "Yield", "User"] => {
                let Ok(inst) = inst.parse() else { return false };
                self.set_charger_lifetime_kwh(inst, value);
            }
            ["solarcharger", inst, "History", "Daily", day, "Yield"] => {
                let (Ok(inst), Ok(day)) = (inst.parse(), day.parse::<usize>()) else {
                    return false;
                };
                if day >= DAILY_HISTORY_DAYS {
                    return false;
                }
                self.set_charger_day_kwh(inst, day, value);
            }
            ["pvinverter", inst, "Ac", "Energy", "Forward"] => {
                let Ok(inst) = inst.parse() else { return false };
                self.set_pvinverter_lifetime_kwh(inst, value);
            }
            _ => return false,
        }
        true
    }

    pub fn set_dc_pv_w(&mut self, value: Option<f64>) {
        self.dc_pv_w = value;
    }

    /// `key` is `<PvOnGrid|PvOnOutput|PvOnGenset>/<L1..L3>`.
    pub fn set_ac_pv_w(&mut self, key: &str, value: Option<f64>) {
        match value {
            Some(v) => self.ac_pv_w.insert(key.to_owned(), v),
            None => self.ac_pv_w.remove(key),
        };
    }

    pub fn set_charger_lifetime_kwh(&mut self, inst: u32, value: Option<f64>) {
        set_or_remove(&mut self.mppt_lifetime_kwh, inst, value);
    }

    /// `day` 0 is today. Panics past [`DAILY_HISTORY_DAYS`].
    pub fn set_charger_day_kwh(&mut self, inst: u32, day: usize, value: Option<f64>) {
        self.mppt_daily_kwh
            .entry(inst)
            .or_insert([None; DAILY_HISTORY_DAYS])[day] = value;
    }

    /// A charger that stopped answering: drop everything it contributed.
    pub fn remove_charger(&mut self, inst: u32) {
        self.mppt_lifetime_kwh.remove(&inst);
        self.mppt_daily_kwh.remove(&inst);
    }

    pub fn set_pvinverter_lifetime_kwh(&mut self, inst: u32, value: Option<f64>) {
        set_or_remove(&mut self.pvinverter_lifetime_kwh, inst, value);
    }

    /// Live PV power: DC (all MPPTs) plus AC-coupled PV on every phase.
    #[must_use]
    pub fn pv_power_w(&self) -> Option<f64> {
        if self.dc_pv_w.is_none() && self.ac_pv_w.is_empty() {
            return None;
        }
        Some(self.dc_pv_w.unwrap_or(0.0) + self.ac_pv_w.values().sum::<f64>())
    }

    /// Lifetime solar energy of the whole system, in kWh.
    #[must_use]
    pub fn lifetime_kwh(&self) -> Option<f64> {
        if self.mppt_lifetime_kwh.is_empty() && self.pvinverter_lifetime_kwh.is_empty() {
            return None;
        }
        Some(
            self.mppt_lifetime_kwh.values().sum::<f64>()
                + self.pvinverter_lifetime_kwh.values().sum::<f64>(),
        )
    }

    /// Per-day MPPT yield summed over all chargers, index 0 = today,
    /// cut at the first day any charger doesn't report. PV inverters keep
    /// no history, so their share isn't in here.
    #[must_use]
    pub fn daily_history_kwh(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if self.mppt_daily_kwh.is_empty() {
            return out;
        }
        for day in 0..DAILY_HISTORY_DAYS {
            let mut sum = 0.0;
            for days in self.mppt_daily_kwh.values() {
                match days[day] {
                    Some(v) => sum += v,
                    None => return out,
                }
            }
            out.push(sum);
        }
        out
    }
}

fn set_or_remove(map: &mut BTreeMap<u32, f64>, key: u32, value: Option<f64>) {
    match value {
        Some(v) => map.insert(key, v),
        None => map.remove(&key),
    };
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::*;

    const P: &str = "0123456789ab";

    fn feed(r: &mut Readings, path: &str, payload: &str) -> bool {
        r.apply(P, &format!("N/{P}/{path}"), payload.as_bytes())
    }

    #[wasm_test]
    fn finds_the_portal_from_the_serial_topic_only() {
        assert_eq!(
            portal_from_serial_topic("N/0123456789ab/system/0/Serial"),
            Some(P)
        );
        assert_eq!(
            portal_from_serial_topic("N/0123456789ab/system/0/Serial/x"),
            None
        );
        assert_eq!(
            portal_from_serial_topic("R/0123456789ab/system/0/Serial"),
            None
        );
    }

    #[wasm_test]
    fn reads_numbers_and_treats_null_strings_and_empty_as_absent() {
        assert_eq!(
            payload_value(br#"{"value": 4883.990234375}"#),
            Some(4883.990_234_375)
        );
        assert_eq!(payload_value(br#"{"value":-12}"#), Some(-12.0));
        assert_eq!(payload_value(br#"{"value": 1e3, "x": 1}"#), Some(1000.0));
        assert_eq!(payload_value(br#"{"value": null}"#), None);
        assert_eq!(payload_value(br#"{"value": "Pylontech battery"}"#), None);
        assert_eq!(payload_value(b""), None);
    }

    #[wasm_test]
    fn sums_pv_power_over_dc_and_every_ac_coupled_phase() {
        let mut r = Readings::default();
        assert_eq!(r.pv_power_w(), None);
        feed(&mut r, "system/0/Dc/Pv/Power", r#"{"value": 9.766}"#);
        feed(&mut r, "system/0/Ac/PvOnGrid/L1/Power", r#"{"value": 100}"#);
        feed(
            &mut r,
            "system/0/Ac/PvOnOutput/L2/Power",
            r#"{"value": 50}"#,
        );
        feed(
            &mut r,
            "system/0/Ac/PvOnGrid/L3/Power",
            r#"{"value": null}"#,
        );
        assert!(!feed(
            &mut r,
            "system/0/Ac/Consumption/L1/Power",
            r#"{"value": 500}"#
        ));
        assert!((r.pv_power_w().unwrap() - 159.766).abs() < 1e-9);
    }

    #[wasm_test]
    fn lifetime_adds_every_charger_and_pv_inverter() {
        let mut r = Readings::default();
        feed(
            &mut r,
            "solarcharger/278/Yield/User",
            r#"{"value": 4883.99}"#,
        );
        feed(&mut r, "solarcharger/279/Yield/User", r#"{"value": 10}"#);
        feed(&mut r, "pvinverter/20/Ac/Energy/Forward", r#"{"value": 6}"#);
        assert!((r.lifetime_kwh().unwrap() - 4899.99).abs() < 1e-9);
        // A charger that disappears stops counting.
        feed(&mut r, "solarcharger/279/Yield/User", "");
        assert!((r.lifetime_kwh().unwrap() - 4889.99).abs() < 1e-9);
    }

    #[wasm_test]
    fn daily_history_is_summed_per_day_and_cut_where_a_charger_has_no_data() {
        let mut r = Readings::default();
        for (inst, days) in [("278", 3), ("279", 2)] {
            for day in 0..days {
                feed(
                    &mut r,
                    &format!("solarcharger/{inst}/History/Daily/{day}/Yield"),
                    &format!(r#"{{"value": {}}}"#, day + 1),
                );
            }
        }
        assert_eq!(r.daily_history_kwh(), vec![2.0, 4.0]);
        assert!(!feed(
            &mut r,
            "solarcharger/278/History/Daily/31/Yield",
            r#"{"value": 1}"#
        ));
    }

    #[wasm_test]
    fn ignores_other_portals_and_marks_completion_once() {
        let mut r = Readings::default();
        assert!(!r.apply(P, "N/other/system/0/Dc/Pv/Power", br#"{"value": 1}"#));
        assert!(feed(&mut r, "full_publish_completed", r#"{"value": 1}"#));
        assert!(r.complete);
        assert!(!feed(&mut r, "full_publish_completed", r#"{"value": 1}"#));
    }

    #[wasm_test]
    fn subscribes_to_the_portal_specific_topics() {
        let subs = subscriptions(P);
        assert!(subs.contains(&format!("N/{P}/solarcharger/+/History/Daily/+/Yield")));
        assert!(subs.iter().all(|s| s.starts_with(&format!("N/{P}/"))));
    }
}
