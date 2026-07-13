// Copyright (C) 2026  Braiins Systems s.r.o.

//! Grid view (fleet overview): status counts, a hashrate area chart,
//! and Power / Efficiency / Temp tiles.

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
    BORDER, CARD_BG, FRAME_H, FRAME_W, GAP, LABEL, LABEL_FONT, METRIC_ICON, PAD, STAT_EFFICIENCY,
    STAT_POWER, STAT_TEMP, VALUE_FONT, area_chart, header, icon, status_counts,
};

const STATUS_ICON: f32 = 32.0;
const ROW_H: f32 = 184.0;
const STATUS_W: f32 = 405.0;
const CHART_W: f32 = 819.0;
const TILE_W: f32 = 405.0;

#[derive(Debug)]
pub struct DashboardVm {
    pub title: String,
    pub device_count: usize,
    pub ok: usize,
    pub degraded: usize,
    pub off: usize,
    pub hashrate_ths: f32,
    pub hashrate_series: Vec<f32>,
    pub power_w: f32,
    pub efficiency_jth: f32,
    pub temp_min_c: f32,
    pub temp_avg_c: f32,
    pub temp_max_c: f32,
}

#[must_use]
pub fn dashboard(vm: &DashboardVm) -> Node {
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [header(&vm.title, vm.device_count, false), grid(vm)],
    )
}

fn grid(vm: &DashboardVm) -> Node {
    col(
        props!(gap: GAP, flex: 1.0),
        [
            row(props!(gap: GAP), [fleet_status(vm), hashrate(vm)]),
            row(
                props!(gap: GAP),
                [
                    metric_tile("Power", &STAT_POWER, &group(f64::from(vm.power_w), 0), "W"),
                    metric_tile(
                        "Efficiency",
                        &STAT_EFFICIENCY,
                        &group(f64::from(vm.efficiency_jth), 2),
                        "J/TH",
                    ),
                    temp_tile(vm),
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

fn fleet_status(vm: &DashboardVm) -> Node {
    card(
        STATUS_W,
        vec![
            text("Fleet Status", style!(size: LABEL_FONT, color: LABEL)),
            status_counts(
                vm.ok,
                vm.degraded,
                vm.off,
                STATUS_ICON,
                VALUE_FONT,
                96.0,
                FontWeight::SEMIBOLD,
            ),
        ],
    )
}

// Hashrate card: the area chart plus the hero value drawn over it on one canvas.
fn hashrate(vm: &DashboardVm) -> Node {
    let mut draws = area_chart(&vm.hashrate_series, CHART_W, ROW_H, 0.42);
    draws.push(Draw::text(
        CHART_W / 2.0,
        ROW_H / 2.0 - 24.0,
        "Hashrate (24h)",
        style!(size: LABEL_FONT, color: LABEL, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    draws.push(Draw::text(
        CHART_W / 2.0,
        ROW_H / 2.0 + 12.0,
        fmt!("{} TH/s", group(f64::from(vm.hashrate_ths), 2)),
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

fn temp_tile(vm: &DashboardVm) -> Node {
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
                    value_row(&group(f64::from(vm.temp_avg_c), 0), "°C"),
                    value_row(&group(f64::from(vm.temp_min_c), 0), "°C"),
                    value_row(&group(f64::from(vm.temp_max_c), 0), "°C"),
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
            icon(&STAT_TEMP, METRIC_ICON, LABEL),
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
