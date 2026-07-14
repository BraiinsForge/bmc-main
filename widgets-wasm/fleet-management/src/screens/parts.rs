// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared parts across the fleet screens: design tokens, icons,
//! the header + view toggle, the status counts, and the area/spark chart.
//! Kept here so the grid (dashboard) and list (per-model table) views render identically.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::view::{ViewMode, view_click_id};

// Design tokens map 1:1 to the Figma "Braiins DECK" frames.
pub const CARD_BG: Color = Color::from_hex(0x09_09_09);
pub const HEADER_BG: Color = GRAY_100;
pub const BORDER: Color = GRAY_100;
pub const LABEL: Color = GRAY_40;
pub const OK: Color = GREEN_50;
pub const DEGRADED: Color = ORANGE_40;
pub const OFF: Color = BLUE_60;
pub const CHART: Color = VIOLET_60;

pub const TITLE_FONT: u32 = 24;
pub const VALUE_FONT: u32 = 40;
pub const ROW_FONT: u32 = 20;
pub const LABEL_FONT: u32 = 20;
pub const METRIC_ICON: f32 = 20.0;

// Fixed geometry for the Deck's Full band (1280 x 480).
pub const FRAME_W: f32 = 1280.0;
pub const FRAME_H: f32 = 480.0;
pub const PAD: f32 = 24.0;
pub const GAP: f32 = 8.0;
pub const DETAIL_BUTTON_WIDTH: f32 = 96.0;

pub const PERF_OK: Svg = include_svg!("assets/icons/perf-ok.svg");
pub const PERF_LOW: Svg = include_svg!("assets/icons/perf-low.svg");
pub const PERF_OFF: Svg = include_svg!("assets/icons/perf-off.svg");
pub const STAT_POWER: Svg = include_svg!("assets/icons/stat-power.svg");
pub const STAT_EFFICIENCY: Svg = include_svg!("assets/icons/stat-efficiency.svg");
pub const STAT_TEMP: Svg = include_svg!("assets/icons/stat-temp.svg");
const TOGGLE_GRID: Svg = include_svg!("assets/icons/dashboard.svg");
const TOGGLE_LIST: Svg = include_svg!("assets/icons/list.svg");

// A tinted icon of `size`, on its own square canvas.
#[must_use]
pub fn icon(svg: &Svg, size: f32, color: Color) -> Node {
    canvas(
        props!(width: size, height: size),
        vec![Draw::svg(0.0, 0.0, size, size, svg, color).with_anti_alias()],
    )
}

#[must_use]
pub fn header(title: &str, count: usize, list_active: bool) -> Node {
    row(
        props!(height: 40.0, cross_align: CrossAlign::Center, gap: 16.0),
        [
            text(
                title,
                style!(size: TITLE_FONT, weight: FontWeight::BOLD, color: WHITE),
            ),
            text(
                fmt!("{count} Devices"),
                style!(size: LABEL_FONT, color: LABEL),
            ),
            col(props!(flex: 1.0), Vec::<Node>::new()),
            toggle(list_active),
        ],
    )
}

fn toggle(list_active: bool) -> Node {
    switcher(
        usize::from(list_active),
        false,
        &[
            Tab {
                icon: &TOGGLE_GRID,
                click_id: view_click_id(ViewMode::Grid),
            },
            Tab {
                icon: &TOGGLE_LIST,
                click_id: view_click_id(ViewMode::List),
            },
        ],
    )
}

// The three status counts (ok / degraded / off) — always all three, even zeros.
#[must_use]
pub fn status_counts(
    ok: usize,
    degraded: usize,
    off: usize,
    icon_px: f32,
    font: u32,
    slot_w: f32,
    weight: FontWeight,
) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            status_item(&PERF_OK, ok, OK, icon_px, font, slot_w, weight),
            status_item(&PERF_LOW, degraded, DEGRADED, icon_px, font, slot_w, weight),
            status_item(&PERF_OFF, off, OFF, icon_px, font, slot_w, weight),
        ],
    )
}

// Fixed-width slot so the icons line up in a column across table rows.
fn status_item(
    svg: &Svg,
    count: usize,
    color: Color,
    icon_px: f32,
    font: u32,
    slot_w: f32,
    weight: FontWeight,
) -> Node {
    row(
        props!(width: slot_w, gap: 6.0, cross_align: CrossAlign::Center),
        [
            icon(svg, icon_px, color),
            text(
                count_str(count),
                style!(size: font, weight: weight, color: WHITE),
            ),
        ],
    )
}

// The filled area chart + trend line for a hashrate series, mapped into `w`x`h`.
// `top_frac` leaves headroom above the chart (for the dashboard's hero text).
#[must_use]
pub fn area_chart(series: &[f32], w: f32, h: f32, top_frac: f32) -> Vec<Draw> {
    let (min, max) = series
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (max - min).max(1e-3);
    let top = h * top_frac;
    let usable = h - top;
    let last = idx_f32(series.len().max(2) - 1);
    let trend: Vec<(f32, f32)> = series
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            (
                idx_f32(i) / last * w,
                top + (1.0 - (v - min) / span) * usable,
            )
        })
        .collect();
    let mut area = trend.clone();
    area.push((w, h));
    area.push((0.0, h));
    vec![
        fill!(area, linear: (CHART.with_alpha(0.55), CHART.with_alpha(0.05)), smooth),
        path!(trend, stroke: 2.0, color: CHART, smooth),
    ]
}

#[must_use]
pub fn count_str(count: usize) -> String {
    let mut out = String::new();
    units::format::push_int(&mut out, count as u64);
    out
}

#[expect(
    clippy::cast_precision_loss,
    reason = "series indices stay tiny, well within f32's exact integer range"
)]
fn idx_f32(i: usize) -> f32 {
    i as f32
}
