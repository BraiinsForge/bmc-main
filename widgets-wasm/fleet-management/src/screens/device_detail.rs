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

//! One miner's read-only telemetry as a stat-tile grid,
//! opened from a device's Detail button.

use bmc_wasm_sdk::types::{ElectricPower, Hashrate, MiningEfficiency, Temperature};
#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::history::{ChartWindow, HistoryDatum};
use crate::screens::icons;
use crate::screens::parts::{
    BACK_CHIP, BORDER, CARD_BG, Crumb, FRAME_H, FRAME_W, GAP, LABEL, LABEL_FONT, METRIC_ICON, PAD,
    UNAVAILABLE, VALUE_FONT, back_button, breadcrumb, icon, scaled_area_chart, si_parts_or_dash,
    status_glyph,
};
use crate::summary::DeviceStatus;
use crate::telemetry::DeviceTemp;
use crate::view::{CrumbTarget, crumb_click_id};

// Charts need an explicit canvas size, so derive each tile's inner dimensions
// from the frame: a 4-wide, 2-tall grid filling the body below the header.
const TILE_W: f32 = (FRAME_W - 2.0 * PAD - 3.0 * GAP) / 4.0;
const TILE_H: f32 = (FRAME_H - 2.0 * PAD - BACK_CHIP - 16.0 - GAP) / 2.0;
const CHART_W: f32 = TILE_W - 2.0;
const CHART_H: f32 = TILE_H - 2.0;

const TILE_GAP: f32 = 24.0;
const STATE_ICON: f32 = 32.0;

#[derive(Debug)]
pub struct DeviceDetailData {
    pub fleet_name: String,
    pub model: String,
    pub hostname: String,
    pub ip: String,
    pub mac: Option<String>,
    pub state: DeviceStatus,
    pub hashrate: Option<Hashrate>,
    pub hashrate_series: Vec<HistoryDatum>,
    pub window: ChartWindow,
    pub nominal_hashrate: Option<Hashrate>,
    pub power: Option<ElectricPower>,
    pub efficiency: Option<MiningEfficiency>,
    pub uptime_hours: Option<u64>,
    pub temperature: Option<DeviceTemp>,
}

#[must_use]
pub fn device_detail_view(d: &DeviceDetailData) -> Node {
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [
            header(&d.fleet_name, &d.model, &d.hostname),
            col(
                props!(gap: GAP, flex: 1.0),
                [
                    row(
                        props!(gap: GAP, flex: 1.0),
                        [
                            ip_mac_tile(&d.ip, d.mac.as_deref()),
                            state_tile(d.state),
                            chart_tile(
                                "Hashrate",
                                &d.hashrate
                                    .map_or_else(|| UNAVAILABLE.to_owned(), |h| h.format_si(3)),
                                scaled_area_chart(
                                    &d.hashrate_series,
                                    d.window,
                                    CHART_W,
                                    CHART_H,
                                    0.42,
                                    nominal_ths(d.nominal_hashrate),
                                    false,
                                ),
                            ),
                            nominal_tile(d.nominal_hashrate),
                        ],
                    ),
                    row(
                        props!(gap: GAP, flex: 1.0),
                        [
                            {
                                let (v, u) =
                                    si_parts_or_dash(d.power.map(|p| p.format_si_parts(3)));
                                metric_tile("Power", &icons::STAT_POWER, &v, &u)
                            },
                            metric_tile(
                                "Efficiency",
                                &icons::STAT_EFFICIENCY,
                                &d.efficiency
                                    .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(2)),
                                d.efficiency.map_or("", |_| MiningEfficiency::UNIT),
                            ),
                            metric_tile(
                                "Uptime",
                                &icons::STAT_TIME,
                                &d.uptime_hours
                                    .map_or_else(|| UNAVAILABLE.to_owned(), |h| fmt!("{h}")),
                                d.uptime_hours.map_or("", |_| "Hrs"),
                            ),
                            temp_tile(d.temperature),
                        ],
                    ),
                ],
            ),
        ],
    )
}

fn header(fleet_name: &str, model: &str, hostname: &str) -> Node {
    row(
        props!(height: BACK_CHIP, cross_align: CrossAlign::Center, gap: 16.0),
        [
            back_button(),
            breadcrumb(&[
                Crumb {
                    label: fleet_name,
                    click_id: Some(crumb_click_id(CrumbTarget::Fleet)),
                },
                Crumb {
                    label: model,
                    click_id: Some(crumb_click_id(CrumbTarget::Model)),
                },
                Crumb {
                    label: hostname,
                    click_id: None,
                },
            ]),
        ],
    )
}

// A flex tile: border-sim frame around a centred content column.
fn tile(children: Vec<Node>) -> Node {
    col(
        props!(background: BORDER, flex: 1.0, padding: 1.0),
        [center(
            props!(background: CARD_BG, flex: 1.0),
            [col(
                props!(gap: TILE_GAP, cross_align: CrossAlign::Center),
                children,
            )],
        )],
    )
}

fn label_row(label: &str, svg: &Svg) -> Node {
    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            text(label, style!(size: LABEL_FONT, color: LABEL)),
            icon(svg, METRIC_ICON, LABEL),
        ],
    )
}

fn value_row(value: &str, unit: &str) -> Node {
    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            text(
                value,
                style!(size: VALUE_FONT, weight: FontWeight::SEMIBOLD, color: WHITE),
            ),
            text(unit, style!(size: LABEL_FONT, color: LABEL)),
        ],
    )
}

fn metric_tile(label: &str, svg: &Svg, value: &str, unit: &str) -> Node {
    tile(vec![label_row(label, svg), value_row(value, unit)])
}

// IP over MAC, both at label size;
// MAC is an em dash when the API omits it.
fn ip_mac_tile(ip: &str, mac: Option<&str>) -> Node {
    let line = |s: &str| {
        text(
            s,
            style!(size: LABEL_FONT, weight: FontWeight::SEMIBOLD, color: WHITE),
        )
    };
    tile(vec![
        label_row("IP/MAC", &icons::STAT_NETWORK),
        col(
            props!(gap: 8.0, cross_align: CrossAlign::Center),
            [line(ip), line(mac.unwrap_or("\u{2014}"))],
        ),
    ])
}

fn state_tile(state: DeviceStatus) -> Node {
    let (svg, color, label) = status_glyph(state);
    tile(vec![
        text("State", style!(size: LABEL_FONT, color: LABEL)),
        row(
            props!(gap: 8.0, cross_align: CrossAlign::Center),
            [
                icon(svg, STATE_ICON, color),
                text(
                    label,
                    style!(size: VALUE_FONT, weight: FontWeight::SEMIBOLD, color: WHITE),
                ),
            ],
        ),
    ])
}

// Nameplate TH/s for the chart ceiling; `None` when unknown.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a chart ceiling is fine at f32 precision"
)]
fn nominal_ths(nominal: Option<Hashrate>) -> Option<f32> {
    let ths = nominal?.as_terahashes_per_second() as f32;
    (ths > 0.0).then_some(ths)
}

// Nominal is a static nameplate, so it shows as a value, not a time chart.
// Shown even for an unreachable device — it is a spec, not a live reading.
fn nominal_tile(nominal: Option<Hashrate>) -> Node {
    let (v, u) = si_parts_or_dash(nominal.map(|n| n.format_si_parts(3)));
    tile(vec![
        text("Nominal Hashrate", style!(size: LABEL_FONT, color: LABEL)),
        value_row(&v, &u),
    ])
}

// label + hero value over a pre-built chart
fn chart_tile(label: &str, value: &str, mut draws: Vec<Draw>) -> Node {
    draws.push(Draw::text(
        CHART_W / 2.0,
        CHART_H / 2.0 - 20.0,
        label,
        style!(size: LABEL_FONT, color: LABEL, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    draws.push(Draw::text(
        CHART_W / 2.0,
        CHART_H / 2.0 + 16.0,
        value,
        style!(size: VALUE_FONT, weight: FontWeight::SEMIBOLD, color: WHITE, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    col(
        props!(background: BORDER, flex: 1.0, padding: 1.0),
        [col(
            props!(background: CARD_BG, flex: 1.0),
            [canvas(props!(width: CHART_W, height: CHART_H), draws)],
        )],
    )
}

// Honest temperature: one value for a single sensor, Avg/Min/Max for a spread.
fn temp_tile(temp: Option<DeviceTemp>) -> Node {
    match temp {
        None => tile(vec![
            label_row("Temp", &icons::STAT_TEMP),
            value_row(UNAVAILABLE, ""),
        ]),
        Some(DeviceTemp::Single(t)) => tile(vec![
            label_row("Temp", &icons::STAT_TEMP),
            value_row(&t.format_value(0), "°C"),
        ]),
        Some(DeviceTemp::Spread { min, avg, max }) => tile(vec![
            row(
                props!(gap: 20.0, cross_align: CrossAlign::Center),
                [temp_head("Avg"), temp_head("Min"), temp_head("Max")],
            ),
            row(
                props!(gap: 8.0, cross_align: CrossAlign::Center),
                [temp_value(avg), temp_value(min), temp_value(max)],
            ),
        ]),
    }
}

fn temp_head(label: &str) -> Node {
    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            text(label, style!(size: LABEL_FONT, color: LABEL)),
            icon(&icons::STAT_TEMP, METRIC_ICON, LABEL),
        ],
    )
}

fn temp_value(t: Temperature) -> Node {
    value_row(&t.format_value(0), "°C")
}
