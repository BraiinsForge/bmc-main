// Copyright (C) 2026  Braiins Systems s.r.o.

//! List view (per-model breakdown): the fleet's models as table rows,
//! the list-view twin of the dashboard grid.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use units::format::group;

use crate::screens::parts::{
    BORDER, CARD_BG, FRAME_H, FRAME_W, GAP, HEADER_BG, LABEL, LABEL_FONT, PAD, ROW_FONT,
    area_chart, header, status_counts,
};

const PAGER_UP: Svg = include_svg!("assets/icons/chevron-up.svg");
const PAGER_DOWN: Svg = include_svg!("assets/icons/chevron-down.svg");

/// Click ids the pager buttons emit — the storybook logs them, the widget pages.
pub const PAGE_UP_ID: &str = "fleet::page_up";
pub const PAGE_DOWN_ID: &str = "fleet::page_down";

// Column widths within the table card.
const COL_MODEL: f32 = 260.0;
const COL_STATUS: f32 = 160.0;
const COL_HASHRATE: f32 = 130.0;
const COL_SPARK: f32 = 90.0;
const COL_POWER: f32 = 120.0;
const COL_EFF: f32 = 130.0;
const COL_AVG: f32 = 90.0;
const DETAIL_W: f32 = 150.0;
const STATUS_SLOT: f32 = 50.0;

const ROW_PAD: f32 = 20.0;
const HEAD_H: f32 = 60.0;
const DATA_H: f32 = 78.0;
const MODEL_FONT: u32 = 22;
const STATUS_ICON: f32 = 16.0;

#[derive(Debug)]
pub struct ModelRow {
    pub name: String,
    pub ok: usize,
    pub degraded: usize,
    pub off: usize,
    pub hashrate_ths: f32,
    pub series: Vec<f32>,
    pub power_w: f32,
    pub efficiency_jth: f32,
    pub avg_temp_c: f32,
}

#[derive(Debug)]
pub struct FleetTableVm {
    pub title: String,
    pub device_count: usize,
    pub rows: Vec<ModelRow>,
    pub page: usize,
    pub page_count: usize,
}

#[must_use]
pub fn table(vm: &FleetTableVm) -> Node {
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [
            header(&vm.title, vm.device_count, true),
            row(
                props!(gap: GAP, flex: 1.0),
                [
                    table_card(vm),
                    pager(vm.page > 0, vm.page + 1 < vm.page_count),
                ],
            ),
        ],
    )
}

fn table_card(vm: &FleetTableVm) -> Node {
    let mut rows: Vec<Node> = vec![header_row()];
    for r in &vm.rows {
        rows.push(separator());
        rows.push(model_row(r));
    }
    // border-sim card wrapping the header + data rows.
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

fn model_row(r: &ModelRow) -> Node {
    row(
        props!(height: DATA_H, padding: ROW_PAD, cross_align: CrossAlign::Center),
        [
            cell(
                COL_MODEL,
                text(
                    r.name.as_str(),
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
            cell(
                COL_HASHRATE,
                value(&group(f64::from(r.hashrate_ths), 2), "TH/s"),
            ),
            cell(
                COL_SPARK,
                canvas(
                    props!(width: 80.0, height: 32.0),
                    area_chart(&r.series, 80.0, 32.0, 0.15),
                ),
            ),
            cell(COL_POWER, value(&group(f64::from(r.power_w), 2), "W")),
            cell(
                COL_EFF,
                value(&group(f64::from(r.efficiency_jth), 1), "J/TH"),
            ),
            cell(COL_AVG, value(&group(f64::from(r.avg_temp_c), 0), "°C")),
            col(props!(flex: 1.0), Vec::<Node>::new()),
            detail_button(),
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

fn detail_button() -> Node {
    center(
        props!(width: DETAIL_W, height: 48.0, background: GRAY_90),
        [text("Detail", style!(size: LABEL_FONT, color: WHITE))],
    )
}

fn separator() -> Node {
    col(props!(height: 1.0, background: BORDER), Vec::<Node>::new())
}

fn pager(can_up: bool, can_down: bool) -> Node {
    // Compact pair pinned to the table's bottom — the leading spacer pushes it down.
    col(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            col(props!(flex: 1.0), Vec::<Node>::new()),
            pager_button(&PAGER_UP, can_up, PAGE_UP_ID),
            pager_button(&PAGER_DOWN, can_down, PAGE_DOWN_ID),
        ],
    )
}

fn pager_button(svg: &Svg, enabled: bool, click_id: &str) -> Node {
    // Disabled keeps the chip and only dims the glyph; a dead direction isn't tappable.
    let glyph = if enabled {
        WHITE
    } else {
        WHITE.with_alpha(0.3)
    };
    let draws = vec![Draw::svg(10.0, 10.0, 20.0, 20.0, svg, glyph).with_anti_alias()];
    let props = props!(width: 40.0, height: 40.0, background: GRAY_90);
    if enabled {
        touchable(click_id, props, draws)
    } else {
        canvas(props, draws)
    }
}
