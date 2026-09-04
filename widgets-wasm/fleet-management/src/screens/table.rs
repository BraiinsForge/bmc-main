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
use crate::history::{ChartWindow, HistoryDatum};
use crate::layout::truncate_label;
use crate::screens::parts::{
    BORDER, CARD_BG, DATA_H, DETAIL_BUTTON_WIDTH, FRAME_H, FRAME_W, GAP, HEAD_H, HEADER_BG, LABEL,
    LABEL_FONT, PAD, ROW_FONT, ROW_PAD, TABLE_MAX_W, UNAVAILABLE, area_chart, header, pager,
    si_parts_or_dash, status_counts,
};
use crate::view::{PagerScope, model_detail_click_id};

// Column widths within the table card. Balanced against the Figma "Braiins DECK"
// list frame: Model was over-wide and starved the rest, the sparkline worst of
// all — width shifted off Model onto the spark (to the design's ~83px) and the
// value columns.
const COL_MODEL: f32 = 310.0;
// The three count slots fill 150; the extra 12 keeps a two-digit off count from
// crowding the next column, which a short "—" placeholder makes obvious.
const COL_STATUS: f32 = 162.0;
const COL_HASHRATE: f32 = 127.0;
const COL_SPARK: f32 = 100.0;
const COL_POWER: f32 = 106.0;
const COL_EFF: f32 = 118.0;
const COL_AVG: f32 = 91.0;
const STATUS_SLOT: f32 = 50.0;
const SPARK_W: f32 = COL_SPARK - 16.0;
const MODEL_CHARS: usize = 28;

const MODEL_FONT: u32 = 22;
const STATUS_ICON: f32 = 16.0;

#[derive(Debug)]
pub struct ModelRow {
    pub name: String,
    /// The model group's family, for the Detail drill-in click id.
    pub family: Option<DeviceFamily>,
    pub ok: usize,
    pub degraded: usize,
    pub off: usize,
    pub hashrate: Option<Hashrate>,
    pub series: Vec<HistoryDatum>,
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
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [
            header(&data.title, data.device_count, true),
            row(
                props!(gap: GAP, flex: 1.0),
                [
                    table_card(data),
                    pager(PagerScope::Fleet, data.page, data.page_count),
                ],
            ),
        ],
    )
}

fn table_card(data: &TableViewData) -> Node {
    let mut rows: Vec<Node> = vec![header_row()];
    for r in &data.rows {
        rows.push(separator());
        rows.push(model_row(r, data.window));
    }
    // border-sim card wrapping the header + data rows.
    col(
        props!(background: BORDER, flex: 1.0, padding: 1.0, max_width: TABLE_MAX_W),
        [col(
            props!(background: CARD_BG, flex: 1.0, cross_align: CrossAlign::Stretch),
            rows,
        )],
    )
}

fn header_row() -> Node {
    row(
        props!(background: HEADER_BG, height: HEAD_H, padding: ROW_PAD, cross_align: CrossAlign::Center),
        [
            head_cell(COL_MODEL, "Model"),
            head_cell(COL_STATUS, "Status"),
            head_cell(COL_HASHRATE, "Hashrate"),
            head_cell(COL_SPARK, ""),
            head_cell(COL_POWER, "Power"),
            head_cell(COL_EFF, "Efficiency"),
            head_cell(COL_AVG, "Avg"),
            col(props!(flex: 1.0), Vec::<Node>::new()),
        ],
    )
}

fn model_row(r: &ModelRow, window: ChartWindow) -> Node {
    row(
        props!(height: DATA_H, padding: ROW_PAD, cross_align: CrossAlign::Center),
        [
            cell(
                COL_MODEL,
                text(
                    truncate_label(&r.name, MODEL_CHARS),
                    style!(size: MODEL_FONT, weight: FontWeight::BOLD, color: WHITE),
                ),
            ),
            cell(
                COL_STATUS,
                status_counts(
                    r.ok,
                    r.degraded,
                    r.off,
                    STATUS_ICON,
                    ROW_FONT,
                    STATUS_SLOT,
                    FontWeight::REGULAR,
                ),
            ),
            cell(COL_HASHRATE, {
                let (v, u) = si_parts_or_dash(r.hashrate.map(|h| h.format_si_parts(3)));
                value(&v, &u)
            }),
            cell(
                COL_SPARK,
                canvas(
                    props!(width: SPARK_W, height: 32.0),
                    area_chart(&r.series, window, SPARK_W, 32.0, 0.15),
                ),
            ),
            cell(COL_POWER, {
                let (v, u) = si_parts_or_dash(r.power.map(|p| p.format_si_parts(3)));
                value(&v, &u)
            }),
            cell(
                COL_EFF,
                value(
                    &r.efficiency
                        .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(1)),
                    r.efficiency.map_or("", |_| MiningEfficiency::UNIT),
                ),
            ),
            cell(
                COL_AVG,
                value(
                    &r.avg_temp
                        .map_or_else(|| UNAVAILABLE.to_owned(), |t| t.format_value(0)),
                    r.avg_temp.map_or("", |_| "°C"),
                ),
            ),
            col(props!(flex: 1.0), Vec::<Node>::new()),
            detail_button(r.family, &r.name),
        ],
    )
}

fn head_cell(width: f32, label: &str) -> Node {
    col(
        props!(width: width),
        [text(label, style!(size: LABEL_FONT, color: LABEL))],
    )
}

fn cell(width: f32, node: Node) -> Node {
    col(props!(width: width, cross_align: CrossAlign::Start), [node])
}

fn value(number: &str, unit: &str) -> Node {
    text(
        fmt!("{number} {unit}"),
        style!(size: ROW_FONT, color: WHITE),
    )
}

fn detail_button(family: Option<DeviceFamily>, name: &str) -> Node {
    let id = model_detail_click_id(family, name);
    touchable(
        &id,
        props!(width: DETAIL_BUTTON_WIDTH, height: 48.0, background: GRAY_90),
        vec![Draw::text(
            DETAIL_BUTTON_WIDTH / 2.0,
            24.0,
            "Detail",
            style!(size: LABEL_FONT, color: WHITE, align: TextAlign::Center, valign: VerticalAlign::Center),
        )],
    )
}

fn separator() -> Node {
    col(props!(height: 1.0, background: BORDER), Vec::<Node>::new())
}
