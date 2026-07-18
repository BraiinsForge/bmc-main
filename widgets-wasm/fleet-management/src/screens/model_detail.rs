// Copyright (C) 2026  Braiins Systems s.r.o.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::layout::truncate_label;
use crate::screens::icons;
use crate::screens::parts::{
    BACK_CHIP, BORDER, CARD_BG, Crumb, DATA_H, DETAIL_BUTTON_WIDTH, FRAME_H, FRAME_W, GAP, HEAD_H,
    HEADER_BG, LABEL, LABEL_FONT, METRIC_ICON, PAD, ROW_PAD, area_chart, back_button, breadcrumb,
    icon, pager, status_glyph,
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
    pub hashrate: Hashrate,
    pub series: Vec<f32>,
    pub power: ElectricPower,
    pub efficiency: MiningEfficiency,
    pub avg_temp: Temperature,
    pub min_temp: Temperature,
    pub max_temp: Temperature,
}

#[derive(Debug)]
pub struct ModelDetailViewData {
    pub fleet_name: String,
    /// The drilled-into model name.
    pub title: String,
    pub device_count: usize,
    pub rows: Vec<DeviceRow>,
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
        rows.push(device_row(r));
    }
    col(
        props!(background: BORDER, flex: 1.0, padding: 1.0),
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

fn device_row(r: &DeviceRow) -> Node {
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
        DeviceStatus::Ok | DeviceStatus::Degraded => metric_cells(r),
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
fn metric_cells(r: &DeviceRow) -> Vec<Node> {
    vec![
        cell(COL_HASHRATE, {
            let (v, u) = r.hashrate.format_si_parts(3);
            value(&v, &u)
        }),
        cell(
            COL_SPARK,
            canvas(
                props!(width: SPARK_W, height: 32.0),
                area_chart(&r.series, SPARK_W, 32.0, 0.15),
            ),
        ),
        cell(COL_POWER, {
            let (v, u) = r.power.format_si_parts(3);
            value(&v, &u)
        }),
        cell(
            COL_EFF,
            value(&r.efficiency.format_value(1), MiningEfficiency::UNIT),
        ),
        cell(COL_TEMP, value(&r.avg_temp.format_value(0), "°C")),
        cell(COL_TEMP, value(&r.min_temp.format_value(0), "°C")),
        cell(COL_TEMP, value(&r.max_temp.format_value(0), "°C")),
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
