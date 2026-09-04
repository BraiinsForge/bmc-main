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

use crate::history::{ChartWindow, HistoryDatum};
use crate::layout::truncate_label;
use crate::screens::icons;
use crate::screens::parts::{
    BACK_CHIP, BORDER, CARD_BG, Crumb, DATA_H, DETAIL_BUTTON_WIDTH, FRAME_H, FRAME_W, GAP, HEAD_H,
    HEADER_BG, LABEL, LABEL_FONT, METRIC_ICON, PAD, ROW_PAD, TABLE_MAX_W, UNAVAILABLE, area_chart,
    back_button, breadcrumb, icon, pager, si_parts_or_dash, status_glyph,
};
use crate::summary::DeviceStatus;
use crate::view::{CrumbTarget, PagerScope, crumb_click_id};

// Column widths within the table card.
const COL_HOST: f32 = 260.0;
const COL_HASHRATE: f32 = 132.0;
const COL_SPARK: f32 = 96.0;
const SPARK_W: f32 = 80.0;
const COL_POWER: f32 = 112.0;
const COL_EFF: f32 = 132.0;
const COL_TEMP: f32 = 90.0;

const HOST_CHARS: usize = 20;
const HOST_FONT: u32 = 24;
const VALUE_FONT: u32 = 24;

#[derive(Debug)]
pub struct DeviceRow {
    pub hostname: String,
    /// The device drill-in click id.
    pub click_id: String,
    pub status: DeviceStatus,
    pub hashrate: Option<Hashrate>,
    pub series: Vec<HistoryDatum>,
    pub power: Option<ElectricPower>,
    pub efficiency: Option<MiningEfficiency>,
    pub avg_temp: Option<Temperature>,
    pub min_temp: Option<Temperature>,
    pub max_temp: Option<Temperature>,
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
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [
            detail_header(&data.fleet_name, &data.title, data.device_count),
            row(
                props!(gap: GAP, flex: 1.0),
                [
                    table_card(data),
                    pager(PagerScope::ModelDetail, data.page, data.page_count),
                ],
            ),
        ],
    )
}

fn detail_header(fleet_name: &str, title: &str, count: usize) -> Node {
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

fn table_card(data: &ModelDetailViewData) -> Node {
    let mut rows: Vec<Node> = vec![header_row()];
    for r in &data.rows {
        rows.push(separator());
        rows.push(device_row(r, data.window));
    }
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
            head_cell(COL_HOST, "Hostname"),
            head_cell(COL_HASHRATE, "Hashrate"),
            head_cell(COL_SPARK, ""),
            head_cell(COL_POWER, "Power"),
            head_cell(COL_EFF, "Efficiency"),
            temp_head("Avg"),
            temp_head("Min"),
            temp_head("Max"),
            col(props!(flex: 1.0), Vec::<Node>::new()),
        ],
    )
}

fn device_row(r: &DeviceRow, window: ChartWindow) -> Node {
    let host = cell(
        COL_HOST,
        text(
            truncate_label(&r.hostname, HOST_CHARS),
            style!(size: HOST_FONT, weight: FontWeight::SEMIBOLD, color: WHITE),
        ),
    );
    // A device with no live telemetry would be a row of meaningless zeros; show
    // its status where the metrics would be, instead of cramming it beside them.
    let body = match r.status {
        DeviceStatus::Ok | DeviceStatus::Degraded => metric_cells(r, window),
        DeviceStatus::Unreachable | DeviceStatus::ApiError | DeviceStatus::AuthError => {
            vec![status_banner(r.status)]
        }
    };
    let mut children = vec![host];
    children.extend(body);
    children.push(detail_button(&r.click_id));
    row(
        props!(height: DATA_H, padding: ROW_PAD, cross_align: CrossAlign::Center),
        children,
    )
}

// The metric cells for a delivering device, ending in the flex slack that pushes
// the Detail button to the right edge.
fn metric_cells(r: &DeviceRow, window: ChartWindow) -> Vec<Node> {
    vec![
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
        cell(COL_TEMP, temp_cell(r.avg_temp)),
        cell(COL_TEMP, temp_cell(r.min_temp)),
        cell(COL_TEMP, temp_cell(r.max_temp)),
        col(props!(flex: 1.0), Vec::<Node>::new()),
    ]
}

// A non-delivering device: the status glyph + label fill the metric area,
// left-aligned just after the hostname, in place of a row of zeros.
fn status_banner(status: DeviceStatus) -> Node {
    let (svg, color, label) = status_glyph(status);
    row(
        props!(flex: 1.0, gap: 12.0, cross_align: CrossAlign::Center),
        [
            icon(svg, 24.0, color),
            text(
                label,
                style!(size: VALUE_FONT, weight: FontWeight::SEMIBOLD, color: color),
            ),
        ],
    )
}

fn head_cell(width: f32, label: &str) -> Node {
    col(
        props!(width: width),
        [text(label, style!(size: LABEL_FONT, color: LABEL))],
    )
}

fn temp_head(label: &str) -> Node {
    col(
        props!(width: COL_TEMP),
        [row(
            props!(gap: 8.0, cross_align: CrossAlign::Center),
            [
                text(label, style!(size: LABEL_FONT, color: LABEL)),
                icon(&icons::STAT_TEMP, METRIC_ICON, LABEL),
            ],
        )],
    )
}

fn cell(width: f32, node: Node) -> Node {
    col(props!(width: width, cross_align: CrossAlign::Start), [node])
}

fn value(number: &str, unit: &str) -> Node {
    text(
        fmt!("{number} {unit}"),
        style!(size: VALUE_FONT, color: WHITE),
    )
}

fn temp_cell(temp: Option<Temperature>) -> Node {
    temp.map_or_else(
        || value(UNAVAILABLE, ""),
        |t| value(&t.format_value(0), "°C"),
    )
}

fn detail_button(click_id: &str) -> Node {
    touchable(
        click_id,
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
