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

//! Shared parts across the fleet screens: design tokens, icons,
//! the header + view toggle, the status counts, and the area/spark chart.
//! Kept here so the grid (dashboard) and list (per-model table) views render identically.

use bmc_wasm_sdk::types::Hashrate;
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
use crate::summary::DeviceStatus;
use crate::view::{PageTurn, PagerScope, ViewMode, pager_click_id, view_click_id};

// Design tokens map 1:1 to the Figma "Braiins DECK" frames.
pub const CARD_BG: Color = Color::from_hex(0x09_09_09);
pub const HEADER_BG: Color = GRAY_100;
pub const BORDER: Color = GRAY_100;
pub const LABEL: Color = GRAY_40;
pub const OK: Color = GREEN_50;
pub const DEGRADED: Color = ORANGE_40;
pub const OFF: Color = BLUE_60;
pub const ERROR: Color = RED_50;
pub const CHART: Color = VIOLET_60;
pub const LINK: Color = VIOLET_50;

pub const TITLE_FONT: u32 = 24;
pub const VALUE_FONT: u32 = 40;
pub const ROW_FONT: u32 = 20;
pub const LABEL_FONT: u32 = 20;
pub const METRIC_ICON: f32 = 20.0;
/// Shown in a metric slot the device has no reading for
/// — never a zero, which reads as a real measurement.
pub const UNAVAILABLE: &str = typography::EMDASH;

/// A quantity's `format_si_parts` value and unit, or the marker and no unit when
/// the reading is absent — the one place the value/unit split renders "no data".
#[must_use]
pub fn si_parts_or_dash(parts: Option<(String, String)>) -> (String, String) {
    parts.unwrap_or_else(|| (UNAVAILABLE.to_owned(), String::new()))
}
const AXIS_FONT: u32 = 16;
// Page count is a secondary control, sized below the row text.
const PAGER_FONT: u32 = 16;
// keeps a flat/zero run's stroke off the clipped canvas edge
const BASELINE_INSET: f32 = 3.0;

// Fixed geometry for the Deck's Full band (1280 x 480).
pub const FRAME_W: f32 = 1280.0;
pub const FRAME_H: f32 = 480.0;
pub const PAD: f32 = 24.0;
pub const GAP: f32 = 8.0;
pub const BACK_CHIP: f32 = 40.0;
// Fixed width of the pager strip on the table's right. Roomy for the font-16
// count with the 40px chevrons, and small enough to leave the table its columns.
const PAGER_W: f32 = 72.0;

// Cap the table so the pager keeps a fixed strip; otherwise the pager absorbs
// all flex-shrink and collapses onto its own label (no min-width prop to pin it).
pub const TABLE_MAX_W: f32 = FRAME_W - 2.0 * PAD - PAGER_W - GAP;

// Table-card geometry, shared by the list and model-detail screens.
// A full page of rows must fit the body height.
// Overflow clamps the flex row up to min-content and shifts the pinned pager.
pub const DECK_TABLE: TableMetrics = TableMetrics {
    head_h: 56.0,
    row_h: 78.0,
    edge: 20.0,
    gutter: 0.0,
};

// A tinted icon of `size`, on its own square canvas.
#[must_use]
pub fn icon(svg: &Svg, size: f32, color: Color) -> Node {
    canvas(
        props!(width: size, height: size),
        vec![Draw::svg(0.0, 0.0, size, size, svg, color).with_anti_alias()],
    )
}

/// The Back chip — steps out one nav level (device → model → fleet), while a
/// breadcrumb jumps straight to any ancestor.
#[must_use]
pub fn back_button(size: f32) -> Node {
    touchable(
        "back",
        chip_props(size),
        [chip_glyph(&icons::CHEVRON_LEFT, size, WHITE)],
    )
}

fn chip_props(size: f32) -> PropsData {
    props!(width: size, height: size, background: GRAY_90)
}

fn chip_glyph(svg: &Svg, size: f32, color: Color) -> Draw {
    let inset = size / 4.0;
    Draw::svg(inset, inset, size / 2.0, size / 2.0, svg, color).with_anti_alias()
}

const CRUMB_FONT: u32 = TITLE_FONT;

/// The widest a crumb gets before it ends in "…": at this width,
/// the back chip, three crumbs and their separators still fit the frame.
const CRUMB_MAX_W: u32 = 360;

/// One breadcrumb segment: an ancestor jump when `click_id` is set,
/// else the current level (plain, non-clickable).
#[derive(Debug)]
pub struct Crumb<'a> {
    pub label: &'a str,
    pub click_id: Option<&'a str>,
}

/// A `/`-separated path — ancestors link-coloured and tappable (each jumps
/// straight to its level), ending in the plain, non-clickable current level.
#[must_use]
pub fn breadcrumb(crumbs: &[Crumb<'_>]) -> Node {
    let mut children: Vec<Node> = Vec::new();
    for (i, crumb) in crumbs.iter().enumerate() {
        if i > 0 {
            children.push(text("/", style!(size: CRUMB_FONT, color: LABEL)));
        }
        children.push(crumb_segment(crumb));
    }
    row(props!(cross_align: CrossAlign::Center, gap: 12.0), children)
}

fn crumb_segment(crumb: &Crumb<'_>) -> Node {
    match crumb.click_id {
        Some(id) => link(
            id,
            crumb.label,
            style!(size: CRUMB_FONT, color: LINK, max_width: CRUMB_MAX_W, text_overflow: TextOverflow::Ellipsis),
        ),
        None => text(
            crumb.label,
            style!(size: CRUMB_FONT, color: WHITE, max_width: CRUMB_MAX_W, text_overflow: TextOverflow::Ellipsis),
        ),
    }
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

#[must_use]
pub fn toggle(list_active: bool) -> Node {
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

// Icon, tint, and label for a device's fine status — shared by the device-detail
// State tile and the model-detail rows so both read identically.
// Unreachable and API-error share the broken-link glyph, split by colour and label.
#[must_use]
pub fn status_glyph(status: DeviceStatus) -> (&'static Svg, Color, &'static str) {
    match status {
        DeviceStatus::Ok => (&icons::PERF_OK, OK, "OK"),
        DeviceStatus::Degraded => (&icons::PERF_LOW, DEGRADED, "Degraded"),
        DeviceStatus::Unreachable => (&icons::UNLINK, OFF, "Unreachable"),
        DeviceStatus::ApiError => (&icons::UNLINK, ERROR, "API error"),
        DeviceStatus::AuthError => (&icons::UNLINK, ERROR, "Login rejected"),
        DeviceStatus::AccountUnusable => (&icons::UNLINK, ERROR, "Check account"),
    }
}

/// How one status count is drawn: its icon, the count's type,
/// the slot it sits in and the gap between icon and count.
#[derive(Clone, Copy, Debug)]
pub struct CountStyle {
    pub icon: f32,
    pub font: u32,
    pub weight: FontWeight,
    pub slot: CountSlot,
    pub gap: f32,
}

/// A fixed slot lines the icons up across table rows; a fitted one hugs its count.
#[derive(Clone, Copy, Debug)]
pub enum CountSlot {
    Fixed(f32),
    Fit,
}

/// `font`, or `stepped` once a count reaches four digits,
/// so a lone row of counts still fits the tile it is centred in.
#[must_use]
pub fn status_count_font(ok: usize, degraded: usize, off: usize, font: u32, stepped: u32) -> u32 {
    if ok.max(degraded).max(off) >= 1_000 {
        stepped
    } else {
        font
    }
}

// The ok / degraded / off counts, always all three (even zeros), `spacing` apart.
// Auth failures show on their own line (see `auth_failures`), not a fourth count column.
#[must_use]
pub fn status_counts(
    ok: usize,
    degraded: usize,
    off: usize,
    style: CountStyle,
    spacing: f32,
) -> Node {
    row(
        props!(gap: spacing, cross_align: CrossAlign::Center),
        [
            status_item(&icons::PERF_OK, ok, OK, style),
            status_item(&icons::PERF_LOW, degraded, DEGRADED, style),
            status_item(&icons::PERF_OFF, off, OFF, style),
        ],
    )
}

/// The "N Auth failures" line under the status counts, shown when a miner
/// rejects its credentials — a distinct prompt, not a fourth count column.
#[must_use]
pub fn auth_failures(count: usize, icon_px: f32, font: u32) -> Node {
    row(
        props!(gap: 6.0, cross_align: CrossAlign::Center),
        [
            icon(&icons::UNLINK, icon_px, ERROR),
            text(
                fmt!("{count} Auth failures"),
                style!(size: font, color: ERROR),
            ),
        ],
    )
}

/// A device's status as a labelled, tinted chip — the inline tag on the rows
/// and device detail, catalogued by variant in the gallery.
#[must_use]
pub fn status_tag(status: DeviceStatus) -> Node {
    let (glyph, color, label) = status_glyph(status);
    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            icon(glyph, 24.0, color),
            text(
                label,
                style!(size: 20, weight: FontWeight::SEMIBOLD, color: color),
            ),
        ],
    )
}

/// Every status tag stacked for the gallery catalog of inline-status variants.
#[must_use]
pub fn status_tag_catalog() -> Node {
    col(
        props!(gap: 20.0, padding: 32.0),
        [
            status_tag(DeviceStatus::Ok),
            status_tag(DeviceStatus::Degraded),
            status_tag(DeviceStatus::Unreachable),
            status_tag(DeviceStatus::ApiError),
            status_tag(DeviceStatus::AuthError),
            status_tag(DeviceStatus::AccountUnusable),
        ],
    )
}

#[must_use]
pub fn status_item(svg: &Svg, count: usize, color: Color, style: CountStyle) -> Node {
    let props = match style.slot {
        CountSlot::Fixed(width) => {
            props!(width: width, gap: style.gap, cross_align: CrossAlign::Center)
        }
        CountSlot::Fit => props!(gap: style.gap, cross_align: CrossAlign::Center),
    };
    row(
        props,
        [
            icon(svg, style.icon, color),
            text(
                count_str(count),
                style!(size: style.font, weight: style.weight, color: WHITE),
            ),
        ],
    )
}

// The filled area chart + trend line for a hashrate series, mapped into `w`x`h`.
// Points sit by their time within `window`; each run gets its own fill and stroke,
// so the chart never draws a false line across missing data.
// `top_frac` leaves headroom above the chart (hero text).
#[must_use]
pub fn area_chart(
    series: &[Vec<ChartPoint>],
    window: ChartWindow,
    w: f32,
    h: f32,
    top_frac: f32,
) -> Vec<Draw> {
    let (min, max) = value_range(series);
    let span = (max - min).max(1e-3);
    let top = h * top_frac;
    let usable = h - top - BASELINE_INSET;
    let y_of = move |v: f32| top + (1.0 - (v - min) / span) * usable;
    draw_runs(series, window, w, h, &y_of)
}

// Large charts: 0-anchored with dashed gridlines and optional right-edge tick
// labels. A `nominal` anchors the axis to the nameplate (device detail reads as
// "vs capacity"); without one (the fleet total) the axis fits the drawn data, so
// it tracks the peak instead of snapping the ceiling to round numbers the data
// never reaches. Sparklines stay bare.
#[must_use]
pub fn scaled_area_chart(
    series: &[Vec<ChartPoint>],
    window: ChartWindow,
    w: f32,
    h: f32,
    top_frac: f32,
    nominal: Option<f32>,
    ticks: bool,
) -> Vec<Draw> {
    let anchored = matches!(nominal, Some(n) if n > 0.0);
    let local_max = series
        .iter()
        .flatten()
        .map(|p| p.value)
        .fold(0.0_f32, f32::max);
    let scale = if anchored {
        nice_scale(local_max.max(nominal.unwrap_or(0.0)))
    } else {
        data_fit_scale(local_max)
    };
    let Some((ceiling, step)) = scale else {
        return area_chart(series, window, w, h, top_frac);
    };
    let top = h * top_frac;
    let plot_h = (h - top - BASELINE_INSET).max(1.0);
    let y_of = move |v: f32| top + (1.0 - (v / ceiling).clamp(0.0, 1.0)) * plot_h;

    // gridlines behind the fill, so they never darken the data line
    let grid = WHITE.with_alpha(0.14);
    let mut draws: Vec<Draw> = Vec::new();
    let mut labels: Vec<Draw> = Vec::new();
    let mut level = step;
    while level <= ceiling + step * 0.5 {
        let y = y_of(level);
        draws.push(path!(vec![(0.0, y), (w, y)], stroke: 1.0, color: grid, dashed: (4.0, 4.0)));
        // Ticks only where the chart is wide enough to keep them off the overlay.
        if ticks {
            labels.push(Draw::text(
                w - GAP,
                y,
                Hashrate::from_terahashes_per_second(f64::from(level)).format_si(3),
                style!(size: AXIS_FONT, color: LABEL, align: TextAlign::Right, valign: VerticalAlign::Center),
            ));
        }
        level += step;
    }
    draws.extend(draw_runs(series, window, w, h, &y_of));
    draws.extend(labels);
    draws
}

// ── Series → drawable runs (time-placed, gap-broken) ─────────────────

// Min and max of the plotted values; `(0, 0)` when none.
fn value_range(series: &[Vec<ChartPoint>]) -> (f32, f32) {
    series
        .iter()
        .flatten()
        .map(|p| p.value)
        .fold(None, |acc: Option<(f32, f32)>, v| {
            Some(acc.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))))
        })
        .unwrap_or((0.0, 0.0))
}

// Fill, trend line, and sample markers for each run;
// `y_of` maps a value to a y pixel and x is placed by time within `window`.
fn draw_runs(
    series: &[Vec<ChartPoint>],
    window: ChartWindow,
    w: f32,
    h: f32,
    y_of: &impl Fn(f32) -> f32,
) -> Vec<Draw> {
    let x_of = time_x(window, w);
    let r = marker_radius(h);
    let mut draws = Vec::new();
    for run in series {
        let pts: Vec<(f32, f32)> = run.iter().map(|p| (x_of(p.at), y_of(p.value))).collect();
        // Two-plus points form the interpolated curve and its fill; a lone point
        // is just its marker below.
        if pts.len() >= 2 {
            let curve = smooth_curve(&pts);
            let x_start = curve.first().map_or(0.0, |p| p.0);
            let x_end = curve.last().map_or(w, |p| p.0);
            let mut area = curve.clone();
            area.push((x_end, h));
            area.push((x_start, h));
            draws.push(fill!(area, linear: (CHART.with_alpha(0.55), CHART.with_alpha(0.05))));
            draws.push(path!(curve, stroke: 3.0, color: CHART));
        }
        // Mark the real samples so a reader tells them from the interpolated
        // curve — dropped only where points pack tighter than a marker, where
        // the curve already coincides with them.
        if markers_fit(&pts, r) {
            for &(x, y) in &pts {
                draws.extend(sample_marker(x, y, r));
            }
        }
    }
    draws
}

// A data-point marker: a filled dot with a punched-out centre, so a real sample
// reads as a marker on the curve it sits on rather than a bulge in the line.
fn sample_marker(x: f32, y: f32, r: f32) -> [Draw; 2] {
    [
        Draw::circle(x, y, r, CHART),
        Draw::circle(x, y, r * 0.42, CARD_BG),
    ]
}

// Marker radius scaled to the chart: bold on the hero/detail charts, tiny on a
// row sparkline.
fn marker_radius(h: f32) -> f32 {
    (h * 0.06).clamp(1.5, 3.5)
}

// Markers fit when the samples are typically at least a marker apart — judged by the median gap,
// so a few tight pairs, such as a restored bucket and the first live one after a quick restart,
// can't strip markers off an otherwise well-spread run.
// A lone point (no pair) always fits.
fn markers_fit(pts: &[(f32, f32)], r: f32) -> bool {
    let mut gaps: Vec<f32> = pts.windows(2).map(|p| (p[1].0 - p[0].0).abs()).collect();
    if gaps.is_empty() {
        return true;
    }
    gaps.sort_unstable_by(f32::total_cmp);
    #[expect(
        clippy::integer_division,
        reason = "median index; which of the two middle gaps is irrelevant to the fit test"
    )]
    let mid = gaps.len() / 2;
    gaps[mid] >= r * 2.5
}

// A closure placing a time on the x axis: the window's right edge at `w`,
// back `span_secs` to `0`, so partial history hugs the right and a stale tail
// leaves a gap there.
fn time_x(window: ChartWindow, w: f32) -> impl Fn(i64) -> f32 {
    let start = window.end - window.span_secs;
    let span = window.span_secs.max(1);
    #[expect(
        clippy::cast_precision_loss,
        reason = "chart pixel coordinates tolerate f32"
    )]
    move |at: i64| (at - start) as f32 / span as f32 * w
}

// Round axis ceiling ≥ `raw` and gridline step (1/2/5 × 10ⁿ).
// `None` unless `raw` is a finite, positive figure the axis can span.
fn nice_scale(raw: f32) -> Option<(f32, f32)> {
    if !raw.is_finite() || raw <= 0.0 {
        return None;
    }
    let step = nice_step(raw / 3.0)?;
    let ceiling = (raw / step).ceil() * step;
    Some((ceiling, step))
}

// Axis fitted to the plotted peak: a hair of headroom above it (so the peak
// marker isn't clipped at the top edge) with nice, readable gridline steps
// within. The ceiling is continuous in the peak, so it tracks the data instead
// of snapping between round numbers as the value ramps.
// `None` unless `max` is a finite, positive peak the axis can fit.
fn data_fit_scale(max: f32) -> Option<(f32, f32)> {
    const HEADROOM: f32 = 1.08;
    const GRIDLINES: f32 = 4.0;
    if !max.is_finite() || max <= 0.0 {
        return None;
    }
    let ceiling = max * HEADROOM;
    Some((ceiling, nice_step(ceiling / GRIDLINES)?))
}

// `None` when no usable step exists, which the gridline loop needs: it advances
// by `step`, so a zero leaves it walking in place. Snapping a `rough` near the
// f32 ceiling up to the next power of ten overflows, hence the finite check.
fn nice_step(rough: f32) -> Option<f32> {
    let mag = pow10_floor(rough)?;
    let norm = rough / mag; // [1, 10)
    let snapped = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = snapped * mag;
    (step.is_finite() && step > 0.0).then_some(step)
}

// Largest power of ten ≤ `v`, sans `log10`/int casts.
// `None` unless `v` is finite and positive: infinity never falls below the
// climbing power, a negative never rises above the shrinking one,
// and either way the loop spins.
fn pow10_floor(v: f32) -> Option<f32> {
    if !v.is_finite() || v <= 0.0 {
        return None;
    }
    let mut p = 1.0_f32;
    while p * 10.0 <= v {
        p *= 10.0;
    }
    while p > v {
        p /= 10.0;
    }
    Some(p)
}

// Sample a Catmull-Rom spline through `pts` into a dense polyline.
// Control points are clamped to each segment's endpoint box:
// in y the curve can't overshoot a peak or dip off-canvas,
// and in x it can't run back on a point much closer than its neighbour,
// as the first live bucket can be to a restored one after a quick restart.
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
        let (left, right) = (p1.0.min(p2.0), p1.0.max(p2.0));
        let (lo, hi) = (p1.1.min(p2.1), p1.1.max(p2.1));
        let cp1 = (
            (p1.0 + (p2.0 - p0.0) / 6.0).clamp(left, right),
            (p1.1 + (p2.1 - p0.1) / 6.0).clamp(lo, hi),
        );
        let cp2 = (
            (p2.0 - (p3.0 - p1.0) / 6.0).clamp(left, right),
            (p2.1 - (p3.1 - p1.1) / 6.0).clamp(lo, hi),
        );
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

#[derive(Clone, Copy, Debug)]
pub struct PagerSize {
    pub width: f32,
    pub chip: f32,
    pub gap: f32,
    pub font: u32,
}

pub const DECK_PAGER: PagerSize = PagerSize {
    width: PAGER_W,
    chip: 40.0,
    gap: 8.0,
    font: PAGER_FONT,
};

/// The bottom page control: 1-based page/total framed by up/down chips.
/// `scope` scopes the turn click ids to the fleet or model-detail table.
#[must_use]
pub fn pager(scope: PagerScope, page: usize, page_count: usize, size: PagerSize) -> Node {
    col(
        props!(gap: size.gap, cross_align: CrossAlign::Center, width: size.width),
        [
            // Leading spacer pushes the cluster to the bottom of its column.
            col(props!(flex: 1.0), Vec::<Node>::new()),
            pager_button(
                &icons::PAGER_UP,
                page > 0,
                pager_click_id(scope, PageTurn::Prev),
                size.chip,
            ),
            text(
                // Unbroken, so a tight column can't split the count over two lines.
                typography::unbroken(fmt!("{} / {}", page + 1, page_count.max(1))),
                style!(size: size.font, color: LABEL),
            ),
            pager_button(
                &icons::PAGER_DOWN,
                page + 1 < page_count,
                pager_click_id(scope, PageTurn::Next),
                size.chip,
            ),
        ],
    )
}

fn pager_button(svg: &Svg, enabled: bool, click_id: &str, chip: f32) -> Node {
    // Disabled keeps the chip and only dims the glyph; a dead direction isn't tappable.
    let glyph = if enabled {
        WHITE
    } else {
        WHITE.with_alpha(0.3)
    };
    let draws = [chip_glyph(svg, chip, glyph)];
    if enabled {
        touchable(click_id, chip_props(chip), draws)
    } else {
        canvas(chip_props(chip), draws)
    }
}

// ── Column tables ────────────────────────────────────────────────────

/// How a column takes its width and places its cells.
#[derive(Clone, Copy, Debug)]
pub enum Fit {
    /// Takes what the other columns leave.
    Grow,
    /// As wide as its widest cell, cells to the left.
    Start,
    /// As wide as its widest cell, cells to the right, as figures read.
    End,
    /// Exactly this wide, cells to the left.
    Fixed(f32),
}

/// What one table cell holds.
#[derive(Debug)]
pub enum Cell {
    Node(Node),
    Empty,
    /// Laid over the columns after it without widening its own:
    /// a message that stands in for a row's figures.
    Overflow(Node),
}

/// A table column: one head over a cell per row,
/// or one head over sub-columns that each fit their own cells.
pub enum Column<'a, R> {
    Single {
        head: Node,
        fit: Fit,
        cell: &'a dyn Fn(&R) -> Cell,
    },
    Group {
        head: Node,
        spacing: f32,
        cells: &'a [&'a dyn Fn(&R) -> Cell],
    },
    /// Empty space this wide between two columns, for a table with no gutter.
    Gap(f32),
}

impl<'a, R> Column<'a, R> {
    #[must_use]
    pub fn single(head: Node, fit: Fit, cell: &'a dyn Fn(&R) -> Cell) -> Self {
        Self::Single { head, fit, cell }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TableMetrics {
    pub head_h: f32,
    pub row_h: f32,
    pub edge: f32,
    pub gutter: f32,
}

#[must_use]
pub fn head_label(label: &str, font: u32) -> Node {
    text(label, style!(size: font, color: LABEL))
}

/// A table laid out by column, each column sized by its `Fit`;
/// an overflow cell never widens its own.
/// Rows exist only as fixed offsets, so the header band, the separators
/// and the tap catchers are layered over the columns at them.
#[must_use]
pub fn column_table<R>(
    rows: &[R],
    columns: Vec<Column<'_, R>>,
    row_id: impl Fn(&R) -> String,
    width: f32,
    m: TableMetrics,
) -> Node {
    // A row's slot also spans the separator line above it.
    let slot_h = m.row_h + 1.0;
    let top = |i: usize| m.head_h + idx_f32(i) * slot_h;
    let band = |top: f32, height: f32, background: Color| {
        col(
            props!(inset_top: top, inset_left: 0.0, inset_right: 0.0, height: height, background: background),
            Vec::<Node>::new(),
        )
    };
    let edge = || col(props!(width: m.edge), Vec::<Node>::new());

    let mut layers = vec![band(0.0, m.head_h, HEADER_BG)];
    layers.extend((0..rows.len()).map(|i| band(top(i), 1.0, BORDER)));
    let columns: Vec<Node> = columns
        .into_iter()
        .map(|c| table_column(rows, c, m))
        .collect();
    layers.push(row(
        props!(),
        [
            edge(),
            row(props!(flex: 1.0, gap: m.gutter), columns),
            edge(),
        ],
    ));
    layers.extend(rows.iter().enumerate().map(|(i, r)| {
        touchable(
            &row_id(r),
            props!(inset_top: top(i), inset_left: 0.0, inset_right: 0.0, height: slot_h),
            Vec::<Draw>::new(),
        )
    }));
    col(
        props!(background: BORDER, width: width, padding: 1.0),
        [col(props!(background: CARD_BG, flex: 1.0), layers)],
    )
}

fn table_column<R>(rows: &[R], column: Column<'_, R>, m: TableMetrics) -> Node {
    let head = |node, align| table_slot(m.head_h, align, Cell::Node(node));
    // A row's slot holds the separator line, then the row.
    let cells = |cell: &dyn Fn(&R) -> Cell, align| {
        rows.iter()
            .map(move |r| {
                col(
                    props!(height: m.row_h + 1.0, cross_align: CrossAlign::Stretch),
                    [
                        col(props!(height: 1.0), Vec::<Node>::new()),
                        table_slot(m.row_h, align, cell(r)),
                    ],
                )
            })
            .collect::<Vec<_>>()
    };
    match column {
        Column::Single {
            head: label,
            fit,
            cell,
        } => {
            let align = match fit {
                Fit::Grow | Fit::Start | Fit::Fixed(_) => CrossAlign::Start,
                Fit::End => CrossAlign::End,
            };
            let mut children = vec![head(label, align)];
            children.extend(cells(cell, align));
            let props = match fit {
                Fit::Grow => props!(flex: 1.0, cross_align: CrossAlign::Stretch),
                Fit::Start | Fit::End => props!(cross_align: CrossAlign::Stretch),
                Fit::Fixed(width) => props!(width: width, cross_align: CrossAlign::Stretch),
            };
            col(props, children)
        }
        Column::Group {
            head: label,
            spacing,
            cells: subs,
        } => col(
            props!(),
            [
                head(label, CrossAlign::Start),
                row(
                    props!(gap: spacing),
                    subs.iter()
                        .map(|cell| col(props!(), cells(*cell, CrossAlign::Start)))
                        .collect::<Vec<_>>(),
                ),
            ],
        ),
        Column::Gap(width) => col(props!(width: width), Vec::<Node>::new()),
    }
}

fn table_slot(height: f32, align: CrossAlign, cell: Cell) -> Node {
    let props = props!(height: height, justify_content: Justify::Center, cross_align: align);
    match cell {
        Cell::Node(node) => col(props, [node]),
        Cell::Empty => col(props, Vec::<Node>::new()),
        Cell::Overflow(node) => col(
            props,
            [col(
                props!(inset_top: 0.0, inset_bottom: 0.0, inset_left: 0.0, justify_content: Justify::Center),
                [node],
            )],
        ),
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "series indices stay tiny, well within f32's exact integer range"
)]
fn idx_f32(i: usize) -> f32 {
    i as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crumb_reaches_the_renderer_whole_capped_to_its_width() {
        const LABEL: &str = "Braiins Mini Miner BMM 101 in the office, by the window";
        let crumb = Crumb {
            label: LABEL,
            click_id: None,
        };

        let Node::Paragraph {
            base_style, spans, ..
        } = crumb_segment(&crumb)
        else {
            panic!("BUG: the current level is plain text");
        };
        assert_eq!(spans[0].text, LABEL);
        assert_eq!(
            (base_style.max_width, base_style.text_overflow),
            (CRUMB_MAX_W, TextOverflow::Ellipsis)
        );
    }

    #[test]
    fn no_scale_spans_a_figure_the_axis_cannot_hold() {
        // Infinity reached these through a `<= 0.0` guard and hung the search
        // for a power of ten; a device reporting an absurd hashrate is enough.
        for bad in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 0.0, -1.0] {
            assert_eq!(nice_scale(bad), None, "nice_scale({bad})");
            assert_eq!(data_fit_scale(bad), None, "data_fit_scale({bad})");
        }
    }

    #[test]
    fn a_real_peak_scales_to_a_step_the_gridlines_can_climb() {
        for (ceiling, step) in [
            nice_scale(37.0).expect("BUG: 37 is spannable"),
            data_fit_scale(37.0).expect("BUG: 37 is fittable"),
        ] {
            assert!(step > 0.0, "step {step} would stall the gridline loop");
            assert!(ceiling >= 37.0, "ceiling {ceiling} clips the peak");
        }
    }

    #[test]
    fn a_peak_too_small_to_halve_leaves_no_step_to_climb_by() {
        // `raw / 3.0` underflows to zero here, and zero has no power of ten.
        let smallest = f32::from_bits(1);
        assert_eq!(pow10_floor(0.0), None);
        assert_eq!(nice_scale(smallest), None);
    }

    #[test]
    fn a_peak_near_the_f32_ceiling_leaves_no_finite_step() {
        // Snapping the magnitude up to the next power of ten runs past f32::MAX.
        assert_eq!(nice_step(f32::MAX), None);
    }

    #[test]
    fn the_curve_never_runs_back_on_a_point_close_to_the_one_before() {
        let curve = smooth_curve(&[(0.0, 50.0), (15.0, 40.0), (30.0, 60.0), (31.0, 10.0)]);
        assert!(
            curve.windows(2).all(|w| w[0].0 <= w[1].0),
            "a gap a fifteenth of the one before it pushes the unclamped curve past 31 and back"
        );
    }

    const WINDOW: ChartWindow = ChartWindow {
        end: 900,
        span_secs: 900,
    };

    const POINT: ChartPoint = ChartPoint {
        at: 600,
        value: 40.0,
    };

    fn marker_ys(draws: &[Draw]) -> Vec<f32> {
        draws
            .iter()
            .filter_map(|d| match d {
                Draw::Circle { cy, .. } => Some(*cy),
                _ => None,
            })
            .collect()
    }

    /// The y a marker for `value` draws at on a data-fit axis whose highest point is `peak`.
    fn fitted_y(value: f32, peak: f32, h: f32) -> f32 {
        let (ceiling, _) = data_fit_scale(peak).expect("BUG: a real peak fits");
        (1.0 - value / ceiling) * (h - BASELINE_INSET)
    }

    #[test]
    fn a_lone_point_draws_on_a_data_fit_axis() {
        let draws = scaled_area_chart(&[vec![POINT]], WINDOW, 200.0, 100.0, 0.0, None, false);
        assert!(
            marker_ys(&draws).first().is_some_and(|&y| (y - fitted_y(
                POINT.value,
                POINT.value,
                100.0
            ))
            .abs()
                < 1e-3),
            "the axis fits the one point there is"
        );
    }

    #[test]
    fn the_axis_fits_the_newest_point_too() {
        let newest = ChartPoint {
            at: 750,
            value: 90.0,
        };
        let draws = scaled_area_chart(
            &[vec![POINT, newest]],
            WINDOW,
            200.0,
            100.0,
            0.0,
            None,
            false,
        );
        assert!(
            marker_ys(&draws).first().is_some_and(|&y| (y - fitted_y(
                POINT.value,
                newest.value,
                100.0
            ))
            .abs()
                < 1e-3),
            "the plot holds no open bucket, so every point it hands over is final"
        );
    }
}
