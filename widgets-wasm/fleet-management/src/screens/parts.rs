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

use crate::screens::icons;
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
pub const BACK_CHIP: f32 = 40.0;

// Table-card row geometry, shared by the list and model-detail screens.
pub const ROW_PAD: f32 = 20.0;
// A full page of rows must fit the body height.
// Overflow clamps the flex row up to min-content and shifts the pinned pager.
pub const HEAD_H: f32 = 56.0;
pub const DATA_H: f32 = 78.0;

// A tinted icon of `size`, on its own square canvas.
#[must_use]
pub fn icon(svg: &Svg, size: f32, color: Color) -> Node {
    canvas(
        props!(width: size, height: size),
        vec![Draw::svg(0.0, 0.0, size, size, svg, color).with_anti_alias()],
    )
}

// The back chip — a tappable `back` button with a left chevron.
#[must_use]
pub fn back_button() -> Node {
    touchable(
        "back",
        props!(width: BACK_CHIP, height: BACK_CHIP, background: GRAY_90),
        vec![Draw::svg(10.0, 10.0, 20.0, 20.0, &icons::CHEVRON_LEFT, WHITE).with_anti_alias()],
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
                icon: &icons::TOGGLE_GRID,
                click_id: view_click_id(ViewMode::Grid),
            },
            Tab {
                icon: &icons::TOGGLE_LIST,
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
            status_item(&icons::PERF_OK, ok, OK, icon_px, font, slot_w, weight),
            status_item(
                &icons::PERF_LOW,
                degraded,
                DEGRADED,
                icon_px,
                font,
                slot_w,
                weight,
            ),
            status_item(&icons::PERF_OFF, off, OFF, icon_px, font, slot_w, weight),
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
    // Fill and stroke share one pre-smoothed top edge drawn as straight segments.
    // A smoothed fill instead bends toward its baseline corners and peels off the stroke.
    let curve = smooth_curve(&trend);
    let mut area = curve.clone();
    area.push((w, h));
    area.push((0.0, h));
    vec![
        fill!(area, linear: (CHART.with_alpha(0.55), CHART.with_alpha(0.05))),
        path!(curve, stroke: 3.0, color: CHART),
    ]
}

// Sample the Catmull-Rom spline through `pts` into a dense polyline (the renderer's
// control points, `bmc-render::build_femtovg_path`), so a linear draw looks smooth.
fn smooth_curve(pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    const STEPS: usize = 10;
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let n = pts.len();
    let mut out = vec![pts[0]];
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(n - 1)];
        let cp1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let cp2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        for s in 1..=STEPS {
            out.push(cubic_bezier(p1, cp1, cp2, p2, idx_f32(s) / idx_f32(STEPS)));
        }
    }
    out
}

fn cubic_bezier(
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p1: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let mt = 1.0 - t;
    let w0 = mt * mt * mt;
    let w1 = 3.0 * mt * mt * t;
    let w2 = 3.0 * mt * t * t;
    let w3 = t * t * t;
    (
        w0 * p0.0 + w1 * c1.0 + w2 * c2.0 + w3 * p1.0,
        w0 * p0.1 + w1 * c1.1 + w2 * c2.1 + w3 * p1.1,
    )
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
