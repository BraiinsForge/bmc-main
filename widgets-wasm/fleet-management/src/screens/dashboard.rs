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

//! Grid view (fleet overview): status counts, a hashrate area chart,
//! and Power / Efficiency / Temp tiles.

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
    BORDER, CARD_BG, FRAME_H, FRAME_W, GAP, LABEL, LABEL_FONT, METRIC_ICON, PAD, UNAVAILABLE,
    VALUE_FONT, auth_failures, header, icon, scaled_area_chart, si_parts_or_dash, status_counts,
};

const STATUS_ICON: f32 = 32.0;
const ROW_H: f32 = 184.0;
const STATUS_W: f32 = 405.0;
const CHART_W: f32 = 819.0;
const TILE_W: f32 = 405.0;

#[derive(Debug)]
pub struct DashboardViewData {
    pub title: String,
    pub device_count: usize,
    pub ok: usize,
    pub degraded: usize,
    pub off: usize,
    pub auth: usize,
    pub hashrate: Option<Hashrate>,
    pub hashrate_series: Vec<HistoryDatum>,
    pub window: ChartWindow,
    pub power: Option<ElectricPower>,
    pub efficiency: Option<MiningEfficiency>,
    pub temp_min: Option<Temperature>,
    pub temp_avg: Option<Temperature>,
    pub temp_max: Option<Temperature>,
}

#[must_use]
pub fn dashboard_view(data: &DashboardViewData) -> Node {
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [header(&data.title, data.device_count, false), grid(data)],
    )
}

fn grid(data: &DashboardViewData) -> Node {
    col(
        props!(gap: GAP, flex: 1.0),
        [
            row(props!(gap: GAP), [fleet_status(data), hashrate(data)]),
            row(
                props!(gap: GAP),
                [
                    {
                        let (v, u) = si_parts_or_dash(data.power.map(|p| p.format_si_parts(3)));
                        metric_tile("Power", &icons::STAT_POWER, &v, &u)
                    },
                    metric_tile(
                        "Efficiency",
                        &icons::STAT_EFFICIENCY,
                        &data
                            .efficiency
                            .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(2)),
                        data.efficiency.map_or("", |_| MiningEfficiency::UNIT),
                    ),
                    temp_tile(data),
                ],
            ),
        ],
    )
}

// A dark card with a hairline border (border-colour frame + inset bg).
fn card(width: f32, children: Vec<Node>) -> Node {
    col(
        props!(background: BORDER, width: width, height: ROW_H, padding: 1.0),
        [center(
            props!(background: CARD_BG, flex: 1.0),
            [col(
                props!(gap: 24.0, cross_align: CrossAlign::Center),
                children,
            )],
        )],
    )
}

fn fleet_status(data: &DashboardViewData) -> Node {
    let mut children = vec![
        text("Fleet Status", style!(size: LABEL_FONT, color: LABEL)),
        status_counts(
            data.ok,
            data.degraded,
            data.off,
            STATUS_ICON,
            VALUE_FONT,
            96.0,
            FontWeight::SEMIBOLD,
        ),
    ];
    if data.auth > 0 {
        children.push(auth_failures(data.auth));
    }
    card(STATUS_W, children)
}

// Hashrate card: the area chart plus the hero value drawn over it on one canvas.
fn hashrate(data: &DashboardViewData) -> Node {
    let mut draws = scaled_area_chart(
        &data.hashrate_series,
        data.window,
        CHART_W,
        ROW_H,
        0.42,
        None,
        true,
    );
    draws.push(Draw::text(
        CHART_W / 2.0,
        ROW_H / 2.0 - 24.0,
        "Hashrate",
        style!(size: LABEL_FONT, color: LABEL, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    draws.push(Draw::text(
        CHART_W / 2.0,
        ROW_H / 2.0 + 12.0,
        data.hashrate
            .map_or_else(|| UNAVAILABLE.to_owned(), |h| h.format_si(3)),
        style!(size: VALUE_FONT, weight: FontWeight::SEMIBOLD, color: WHITE, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    col(
        props!(background: BORDER, flex: 1.0, height: ROW_H, padding: 1.0),
        [col(
            props!(background: CARD_BG, flex: 1.0),
            [canvas(props!(width: CHART_W, height: ROW_H), draws)],
        )],
    )
}

fn metric_tile(label: &str, svg: &Svg, value: &str, unit: &str) -> Node {
    card(
        TILE_W,
        vec![
            row(
                props!(gap: 8.0, cross_align: CrossAlign::Center),
                [
                    text(label, style!(size: LABEL_FONT, color: LABEL)),
                    icon(svg, METRIC_ICON, LABEL),
                ],
            ),
            value_row(value, unit),
        ],
    )
}

fn temp_tile(data: &DashboardViewData) -> Node {
    card(
        TILE_W,
        vec![
            row(
                props!(gap: 40.0),
                [temp_head("Avg"), temp_head("Min"), temp_head("Max")],
            ),
            row(
                props!(gap: 24.0),
                [
                    temp_value(data.temp_avg),
                    temp_value(data.temp_min),
                    temp_value(data.temp_max),
                ],
            ),
        ],
    )
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

fn temp_value(temp: Option<Temperature>) -> Node {
    temp.map_or_else(
        || value_row(UNAVAILABLE, ""),
        |t| value_row(&t.format_value(0), "°C"),
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
