// Copyright (C) 2026  Braiins Systems s.r.o.

//! Hand-picked fixture summaries for the storybook screens.

use units::availability::Availability;
use units::units::{DegreeCelsius, JoulePerTeraHash, TeraHashPerSecond, Watt};

use crate::device::DeviceFamily;
use crate::summary::{FleetSummary, GroupSummary};

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
