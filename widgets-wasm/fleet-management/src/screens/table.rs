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

//! List view (per-model breakdown): the fleet's models as table rows,
//! the list-view twin of the dashboard grid.

use bmc_wasm_sdk::types::{ElectricPower, Hashrate, MiningEfficiency, Temperature};
#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::device::DeviceFamily;
use crate::history::{ChartPoint, ChartWindow};
use crate::screens::icons;
use crate::screens::parts::{
    Cell, Column, CountSlot, CountStyle, DECK_PAGER, DECK_TABLE, DEGRADED, FRAME_H, FRAME_W, Fit,
    GAP, LABEL_FONT, OFF, OK, PAD, ROW_FONT, TABLE_MAX_W, UNAVAILABLE, area_chart, column_table,
    head_label, header, pager, si_parts_or_dash, status_item,
};
use crate::view::{PagerScope, model_detail_click_id};

// The figure columns, balanced against the Figma "Braiins DECK" list frame:
// the sparkline at the design's ~83px, each value column wide enough for its figure.
// Model takes what they and the status counts leave.
const COL_HASHRATE: f32 = 127.0;
const COL_SPARK: f32 = 100.0;
const COL_POWER: f32 = 106.0;
const COL_EFF: f32 = 118.0;
const COL_AVG: f32 = 91.0;
const SPARK_W: f32 = COL_SPARK - 16.0;

const MODEL_FONT: u32 = 22;
const STATUS_ICON: f32 = 16.0;
// Between the counts, and on either side of them,
// so a cut model name never touches the first icon.
const STATUS_SPACING: f32 = 16.0;

#[derive(Debug)]
pub struct ModelRow {
    pub name: String,
    /// The model group's family, for the row's drill-in click id.
    pub family: Option<DeviceFamily>,
    pub ok: usize,
    pub degraded: usize,
    pub off: usize,
    pub hashrate: Option<Hashrate>,
    pub series: Vec<Vec<ChartPoint>>,
    pub power: Option<ElectricPower>,
    pub efficiency: Option<MiningEfficiency>,
    pub avg_temp: Option<Temperature>,
}

#[derive(Debug)]
pub struct TableViewData {
    pub title: String,
    pub device_count: usize,
    pub rows: Vec<ModelRow>,
    pub window: ChartWindow,
    pub page: usize,
    pub page_count: usize,
}

#[must_use]
pub fn table_view(data: &TableViewData) -> Node {
    let model = |r: &ModelRow| {
        Cell::Node(text(
            &r.name,
            style!(size: MODEL_FONT, weight: FontWeight::BOLD, color: WHITE, text_overflow: TextOverflow::Ellipsis),
        ))
    };
    let count = CountStyle {
        icon: STATUS_ICON,
        font: ROW_FONT,
        weight: FontWeight::REGULAR,
        slot: CountSlot::Fit,
        gap: 6.0,
    };
    let ok = |r: &ModelRow| Cell::Node(status_item(&icons::PERF_OK, r.ok, OK, count));
    let degraded =
        |r: &ModelRow| Cell::Node(status_item(&icons::PERF_LOW, r.degraded, DEGRADED, count));
    let off = |r: &ModelRow| Cell::Node(status_item(&icons::PERF_OFF, r.off, OFF, count));
    let states: [&dyn Fn(&ModelRow) -> Cell; 3] = [&ok, &degraded, &off];
    let hashrate = |r: &ModelRow| {
        let (v, u) = si_parts_or_dash(r.hashrate.map(|h| h.format_si_parts(3)));
        value(&v, &u)
    };
    let spark = |r: &ModelRow| {
        Cell::Node(canvas(
            props!(width: SPARK_W, height: 32.0),
            area_chart(&r.series, data.window, SPARK_W, 32.0, 0.15),
        ))
    };
    let power = |r: &ModelRow| {
        let (v, u) = si_parts_or_dash(r.power.map(|p| p.format_si_parts(3)));
        value(&v, &u)
    };
    let efficiency = |r: &ModelRow| {
        value(
            &r.efficiency
                .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(1)),
            r.efficiency.map_or("", |_| MiningEfficiency::UNIT),
        )
    };
    let avg = |r: &ModelRow| {
        value(
            &r.avg_temp
                .map_or_else(|| UNAVAILABLE.to_owned(), |t| t.format_value(0)),
            r.avg_temp.map_or("", |_| "°C"),
        )
    };
    let head = |label| head_label(label, LABEL_FONT);
    let columns = vec![
        Column::single(head("Model"), Fit::Grow, &model),
        Column::Gap(STATUS_SPACING),
        Column::Group {
            head: head("Status"),
            spacing: STATUS_SPACING,
            cells: &states,
        },
        Column::Gap(STATUS_SPACING),
        Column::single(head("Hashrate"), Fit::Fixed(COL_HASHRATE), &hashrate),
        Column::single(head(""), Fit::Fixed(COL_SPARK), &spark),
        Column::single(head("Power"), Fit::Fixed(COL_POWER), &power),
        Column::single(head("Efficiency"), Fit::Fixed(COL_EFF), &efficiency),
        Column::single(head("Avg"), Fit::Fixed(COL_AVG), &avg),
    ];
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [
            header(&data.title, data.device_count, true),
            row(
                props!(gap: GAP, flex: 1.0),
                [
                    column_table(
                        &data.rows,
                        columns,
                        |r| model_detail_click_id(r.family, &r.name),
                        TABLE_MAX_W,
                        DECK_TABLE,
                    ),
                    pager(PagerScope::Fleet, data.page, data.page_count, DECK_PAGER),
                ],
            ),
        ],
    )
}

fn value(number: &str, unit: &str) -> Cell {
    Cell::Node(text(
        fmt!("{number} {unit}"),
        style!(size: ROW_FONT, color: WHITE),
    ))
}
