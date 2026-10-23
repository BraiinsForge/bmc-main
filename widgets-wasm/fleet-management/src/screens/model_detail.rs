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

use bmc_wasm_sdk::types::{ElectricPower, Hashrate, MiningEfficiency, Temperature};
#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::history::{ChartPoint, ChartWindow};
use crate::screens::icons;
use crate::screens::parts::{
    BACK_CHIP, Cell, Column, Crumb, DECK_PAGER, DECK_TABLE, FRAME_H, FRAME_W, Fit, GAP, LABEL,
    LABEL_FONT, METRIC_ICON, PAD, TABLE_MAX_W, UNAVAILABLE, area_chart, back_button, breadcrumb,
    column_table, head_label, icon, pager, si_parts_or_dash, status_glyph,
};
use crate::summary::DeviceStatus;
use crate::view::{CrumbTarget, PagerScope, crumb_click_id};

// Column widths within the table card.
const COL_HOST: f32 = 356.0;
const COL_HASHRATE: f32 = 132.0;
const COL_SPARK: f32 = 96.0;
const SPARK_W: f32 = 80.0;
const COL_POWER: f32 = 112.0;
const COL_EFF: f32 = 132.0;
const COL_TEMP: f32 = 90.0;

const HOST_FONT: u32 = 24;
const VALUE_FONT: u32 = 24;

#[derive(Debug)]
pub struct DeviceRow {
    pub hostname: String,
    /// The device drill-in click id.
    pub click_id: String,
    pub status: DeviceStatus,
    pub hashrate: Option<Hashrate>,
    pub series: Vec<Vec<ChartPoint>>,
    pub power: Option<ElectricPower>,
    pub efficiency: Option<MiningEfficiency>,
    pub avg_temp: Option<Temperature>,
    pub min_temp: Option<Temperature>,
    pub max_temp: Option<Temperature>,
}

impl DeviceRow {
    /// Whether the device answers with telemetry, rather than a status standing in for it.
    #[must_use]
    pub const fn delivering(&self) -> bool {
        match self.status {
            DeviceStatus::Ok | DeviceStatus::Degraded => true,
            DeviceStatus::Unreachable
            | DeviceStatus::ApiError
            | DeviceStatus::AuthError
            | DeviceStatus::AccountUnusable => false,
        }
    }

    /// A figure cell, empty for a device whose status stands in for its figures.
    pub fn figures(&self, node: impl FnOnce() -> Node) -> Cell {
        if self.delivering() {
            Cell::Node(node())
        } else {
            Cell::Empty
        }
    }
}

#[derive(Debug)]
pub struct ModelDetailViewData {
    pub fleet_name: String,
    /// The drilled-into model name.
    pub title: String,
    pub device_count: usize,
    pub rows: Vec<DeviceRow>,
    pub window: ChartWindow,
    pub page: usize,
    pub page_count: usize,
}

#[must_use]
pub fn model_detail_view(data: &ModelDetailViewData) -> Node {
    let host = |r: &DeviceRow| {
        Cell::Node(text(
            &r.hostname,
            style!(size: HOST_FONT, weight: FontWeight::SEMIBOLD, color: WHITE, text_overflow: TextOverflow::Ellipsis),
        ))
    };
    // A device with no live telemetry would be a row of meaningless zeros;
    // its status is laid over where the figures would be instead.
    let hashrate = |r: &DeviceRow| {
        if r.delivering() {
            let (v, u) = si_parts_or_dash(r.hashrate.map(|h| h.format_si_parts(3)));
            Cell::Node(value_text(&v, &u))
        } else {
            Cell::Overflow(status_banner(r.status))
        }
    };
    let spark = |r: &DeviceRow| {
        r.figures(|| {
            canvas(
                props!(width: SPARK_W, height: 32.0),
                area_chart(&r.series, data.window, SPARK_W, 32.0, 0.15),
            )
        })
    };
    let power = |r: &DeviceRow| {
        r.figures(|| {
            let (v, u) = si_parts_or_dash(r.power.map(|p| p.format_si_parts(3)));
            value_text(&v, &u)
        })
    };
    let efficiency = |r: &DeviceRow| {
        r.figures(|| {
            value_text(
                &r.efficiency
                    .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(1)),
                r.efficiency.map_or("", |_| MiningEfficiency::UNIT),
            )
        })
    };
    let avg = |r: &DeviceRow| r.figures(|| temp_text(r.avg_temp));
    let min = |r: &DeviceRow| r.figures(|| temp_text(r.min_temp));
    let max = |r: &DeviceRow| r.figures(|| temp_text(r.max_temp));
    let head = |label| head_label(label, LABEL_FONT);
    let columns = vec![
        Column::single(head("Hostname"), Fit::Fixed(COL_HOST), &host),
        Column::single(head("Hashrate"), Fit::Fixed(COL_HASHRATE), &hashrate),
        Column::single(head(""), Fit::Fixed(COL_SPARK), &spark),
        Column::single(head("Power"), Fit::Fixed(COL_POWER), &power),
        Column::single(head("Efficiency"), Fit::Fixed(COL_EFF), &efficiency),
        Column::single(temp_head("Avg"), Fit::Fixed(COL_TEMP), &avg),
        Column::single(temp_head("Min"), Fit::Fixed(COL_TEMP), &min),
        Column::single(temp_head("Max"), Fit::Fixed(COL_TEMP), &max),
    ];
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [
            detail_header(&data.fleet_name, &data.title, data.device_count),
            row(
                props!(gap: GAP, flex: 1.0),
                [
                    column_table(
                        &data.rows,
                        columns,
                        |r| r.click_id.clone(),
                        TABLE_MAX_W,
                        DECK_TABLE,
                    ),
                    pager(
                        PagerScope::ModelDetail,
                        data.page,
                        data.page_count,
                        DECK_PAGER,
                    ),
                ],
            ),
        ],
    )
}

fn detail_header(fleet_name: &str, title: &str, count: usize) -> Node {
    row(
        props!(height: BACK_CHIP, cross_align: CrossAlign::Center, gap: 16.0),
        [
            back_button(BACK_CHIP),
            breadcrumb(&[
                Crumb {
                    label: fleet_name,
                    click_id: Some(crumb_click_id(CrumbTarget::Fleet)),
                },
                Crumb {
                    label: title,
                    click_id: None,
                },
            ]),
            text(
                fmt!("{count} Devices"),
                style!(size: LABEL_FONT, color: LABEL),
            ),
        ],
    )
}

fn status_banner(status: DeviceStatus) -> Node {
    let (svg, color, label) = status_glyph(status);
    row(
        props!(gap: 12.0, cross_align: CrossAlign::Center),
        [
            icon(svg, 24.0, color),
            text(
                label,
                style!(size: VALUE_FONT, weight: FontWeight::SEMIBOLD, color: color),
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

fn value_text(number: &str, unit: &str) -> Node {
    text(
        fmt!("{number} {unit}"),
        style!(size: VALUE_FONT, color: WHITE),
    )
}

fn temp_text(temp: Option<Temperature>) -> Node {
    temp.map_or_else(
        || value_text(UNAVAILABLE, ""),
        |t| value_text(&t.format_value(0), "°C"),
    )
}
