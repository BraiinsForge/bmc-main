// Copyright (C) 2026  Braiins Systems s.r.o.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "gauge model is consumed by the render path wired in a later step"
    )
)]

use bmc_wasm_sdk::Color;

pub(crate) trait JsonLookup {
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "kept for parity with the shared lookup trait; the auth path consumes it later"
        )
    )]
    fn str(&self, path: &str) -> Option<String>;
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "kept for parity with the shared lookup trait; integer fields are consumed later"
        )
    )]
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
}

pub(crate) fn ths_from_ghs(value: f64) -> f64 {
    value / 1_000.0
}

pub(crate) const MAX_POWER_W: f64 = 200.0;
pub(crate) const STALE_AFTER_MS: u32 = 15_000;

const POWER_GREEN_ANCHOR: Color = Color::from_rgb(0x13, 0xA4, 0x54);
const POWER_GOLD_ANCHOR: Color = Color::from_rgb(0xF4, 0xC0, 0x1A);
const POWER_RED_ANCHOR: Color = Color::from_rgb(0xE5, 0x34, 0x2A);

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MinerData {
    pub(crate) hashrate_ths: Option<f64>,
    pub(crate) power_w: Option<f64>,
    pub(crate) nominal_hashrate_ths: Option<f64>,
}

pub(crate) fn parse_stats(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ghs) = json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second") {
        data.hashrate_ths = Some(ths_from_ghs(ghs));
    }
    if let Some(power) = json.f64("/power_stats/approximated_consumption/watt") {
        data.power_w = Some(power);
    }
}

pub(crate) fn parse_hashboards(json: &impl JsonLookup, data: &mut MinerData) {
    let mut sum_ghs = 0.0;
    let mut idx = 0;
    while hashboard_present(json, idx) {
        if let Some(nominal) = json
            .f64(&bmc_wasm_sdk::fmt!(
                "/hashboards/{idx}/stats/nominal_hashrate/gigahash_per_second"
            ))
            .filter(|nominal| *nominal > 0.0)
        {
            sum_ghs += nominal;
        }
        idx += 1;
    }
    data.nominal_hashrate_ths = if sum_ghs > 0.0 {
        Some(ths_from_ghs(sum_ghs))
    } else {
        None
    };
}

fn hashboard_present(json: &impl JsonLookup, idx: usize) -> bool {
    [
        "stats/nominal_hashrate/gigahash_per_second",
        "stats/real_hashrate/last_1m/gigahash_per_second",
        "board_temp/degree_c",
        "highest_chip_temp/temperature/degree_c",
    ]
    .iter()
    .any(|path| {
        json.f64(&bmc_wasm_sdk::fmt!("/hashboards/{idx}/{path}"))
            .is_some()
    })
}

const POWER_GOLD_FRACTION: f32 = 0.4;

#[expect(
    clippy::cast_possible_truncation,
    reason = "gauge fraction is a clamped 0..1 ratio that loses no meaningful precision in f32"
)]
pub(crate) fn hashrate_fraction(hashrate: Option<f64>, nominal: Option<f64>) -> f32 {
    match (hashrate, nominal) {
        (Some(hashrate), Some(nominal)) if nominal > 0.0 => {
            (hashrate / nominal).clamp(0.0, 1.0) as f32
        }
        _ => 0.0,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "gauge fraction is a clamped 0..1 ratio that loses no meaningful precision in f32"
)]
pub(crate) fn power_fraction(power: Option<f64>) -> f32 {
    match power {
        Some(power) => (power / MAX_POWER_W).clamp(0.0, 1.0) as f32,
        None => 0.0,
    }
}

#[expect(
    clippy::float_cmp,
    reason = "anchor fractions must return their design colors bit-exactly before any interpolation"
)]
pub(crate) fn power_ramp_color(fraction: f32) -> Color {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction == 0.0 {
        return POWER_GREEN_ANCHOR;
    }
    if fraction == POWER_GOLD_FRACTION {
        return POWER_GOLD_ANCHOR;
    }
    if fraction == 1.0 {
        return POWER_RED_ANCHOR;
    }
    if fraction < POWER_GOLD_FRACTION {
        lerp_color(
            POWER_GREEN_ANCHOR,
            POWER_GOLD_ANCHOR,
            fraction / POWER_GOLD_FRACTION,
        )
    } else {
        lerp_color(
            POWER_GOLD_ANCHOR,
            POWER_RED_ANCHOR,
            (fraction - POWER_GOLD_FRACTION) / (1.0 - POWER_GOLD_FRACTION),
        )
    }
}

fn lerp_color(from: Color, to: Color, t: f32) -> Color {
    Color::from_rgb(
        lerp_channel(from.red(), to.red(), t),
        lerp_channel(from.green(), to.green(), t),
        lerp_channel(from.blue(), to.blue(), t),
    )
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "interpolated channel is rounded and clamped into the 0..=255 byte range"
)]
fn lerp_channel(from: u8, to: u8, t: f32) -> u8 {
    let value = f32::from(from) + (f32::from(to) - f32::from(from)) * t;
    value.round().clamp(0.0, 255.0) as u8
}

pub(crate) fn is_stale(age_ms: u32) -> bool {
    age_ms >= STALE_AFTER_MS
}

#[cfg(target_arch = "wasm32")]
impl JsonLookup for bmc_wasm_sdk::json::JsonDoc {
    fn str(&self, path: &str) -> Option<String> {
        self.str(path)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        self.i64(path)
    }

    fn f64(&self, path: &str) -> Option<f64> {
        self.f64(path)
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct MapJson {
        pub(crate) strings: BTreeMap<&'static str, &'static str>,
        pub(crate) ints: BTreeMap<&'static str, i64>,
        pub(crate) floats: BTreeMap<&'static str, f64>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).map(|s| (*s).to_owned())
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::MapJson;
    use super::*;

    #[test]
    fn parses_stats_hashrate_and_power() {
        let mut json = MapJson::default();
        json.floats.insert(
            "/miner_stats/real_hashrate/last_1m/gigahash_per_second",
            122_480.0,
        );
        json.floats
            .insert("/power_stats/approximated_consumption/watt", 41.0);
        let mut data = MinerData::default();
        parse_stats(&json, &mut data);
        assert_eq!(data.hashrate_ths, Some(122.48));
        assert_eq!(data.power_w, Some(41.0));
    }

    #[test]
    fn parses_hashboards_by_summing_nominal_hashrate() {
        let mut json = MapJson::default();
        json.floats.insert(
            "/hashboards/0/stats/nominal_hashrate/gigahash_per_second",
            100_000.0,
        );
        json.floats.insert(
            "/hashboards/1/stats/nominal_hashrate/gigahash_per_second",
            25_000.0,
        );
        let mut data = MinerData::default();
        parse_hashboards(&json, &mut data);
        assert_eq!(data.nominal_hashrate_ths, Some(125.0));
    }

    #[test]
    fn hashrate_fraction_requires_positive_nominal_and_clamps() {
        assert_fraction_eq(hashrate_fraction(Some(50.0), None), 0.0);
        assert_fraction_eq(hashrate_fraction(Some(50.0), Some(0.0)), 0.0);
        assert_fraction_eq(hashrate_fraction(Some(50.0), Some(100.0)), 0.5);
        assert_fraction_eq(hashrate_fraction(Some(125.0), Some(100.0)), 1.0);
    }

    #[test]
    fn power_fraction_is_proportional_to_full_scale_and_clamps() {
        assert_fraction_eq(power_fraction(Some(MAX_POWER_W / 2.0)), 0.5);
        assert_fraction_eq(power_fraction(Some(MAX_POWER_W * 1.25)), 1.0);
        assert_fraction_eq(power_fraction(None), 0.0);
    }

    #[test]
    fn power_ramp_hits_design_anchors() {
        assert_eq!(power_ramp_color(0.0), POWER_GREEN_ANCHOR);
        assert_eq!(power_ramp_color(0.4), POWER_GOLD_ANCHOR);
        assert_eq!(power_ramp_color(1.0), POWER_RED_ANCHOR);
    }

    #[test]
    fn staleness_threshold_is_exclusive_below_and_stale_at_threshold() {
        assert!(!is_stale(STALE_AFTER_MS - 1));
        assert!(is_stale(STALE_AFTER_MS));
    }
}
