// Copyright (C) 2026  Braiins Systems s.r.o.

#[expect(
    clippy::wildcard_imports,
    reason = "render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::device::DeviceList;
use crate::telemetry::TelemetryReading;

fn hashrate_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.current_hashrate_ths) {
        Some(v) => fmt!("{} TH/s", format_number!(f64::from(v), 2)),
        None => "N/A".to_owned(),
    }
}

fn power_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.power_w) {
        Some(v) => fmt!("{} W", format_number!(f64::from(v), 0)),
        None => "N/A".to_owned(),
    }
}

fn temp_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.temperature_c) {
        Some(v) => fmt!("{} °C", format_number!(f64::from(v), 1)),
        None => "N/A".to_owned(),
    }
}

fn uptime_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.uptime_s) {
        // Whole hours; the per-device interim view does not need finer detail.
        // try_from -> u32 keeps clippy's cast lints happy under -D warnings.
        Some(s) => {
            let hours = u32::try_from(s / 3_600).unwrap_or(u32::MAX);
            fmt!("{} h", format_number!(f64::from(hours), 0))
        }
        None => "N/A".to_owned(),
    }
}

#[must_use]
pub fn view(devices: &DeviceList, _width: u32, _height: u32) -> Node {
    if devices.is_empty() {
        return col(
            props!(background: BLACK),
            [center(
                props!(flex: 1.0),
                [text(
                    "Searching for miners\u{2026}",
                    style!(size: 28, color: WHITE),
                )],
            )],
        );
    }

    let mut children: Vec<Node> = vec![text(
        fmt!("{} miners", devices.len()),
        style!(size: 28, weight: FontWeight::BOLD, color: WHITE),
    )];

    for dev in devices.iter() {
        let reading = dev.telemetry.as_ref().map(|s| &s.reading);
        children.push(row(
            props!(gap: 12.0, cross_align: CrossAlign::Center),
            [
                text(
                    dev.identity.name.clone(),
                    style!(size: 20, color: WHITE, flex: 1.0),
                ),
                text(
                    hashrate_cell(reading),
                    style!(size: 20, color: GRAY_40, align: TextAlign::Right),
                ),
                text(
                    power_cell(reading),
                    style!(size: 20, color: GRAY_40, align: TextAlign::Right),
                ),
                text(
                    temp_cell(reading),
                    style!(size: 20, color: GRAY_40, align: TextAlign::Right),
                ),
                text(
                    uptime_cell(reading),
                    style!(size: 20, color: GRAY_40, align: TextAlign::Right),
                ),
            ],
        ));
    }

    col(
        props!(background: BLACK, inset_top: 16.0, inset_left: 16.0, inset_right: 16.0, gap: 8.0),
        children,
    )
}
