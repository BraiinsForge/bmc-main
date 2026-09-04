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

use crate::history::{ChartWindow, HistoryDatum};
use crate::layout::truncate_label;
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
pub const UNAVAILABLE: &str = "\u{2014}";

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
pub const DETAIL_BUTTON_WIDTH: f32 = 96.0;
pub const BACK_CHIP: f32 = 40.0;
// Fixed width of the pager strip on the table's right. Roomy for the font-16
// count with the 40px chevrons, and small enough to leave the table its columns.
const PAGER_W: f32 = 72.0;

// Cap the table so the pager keeps a fixed strip; otherwise the pager absorbs
// all flex-shrink and collapses onto its own label (no min-width prop to pin it).
pub const TABLE_MAX_W: f32 = FRAME_W - 2.0 * PAD - PAGER_W - GAP;

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

/// The Back chip — steps out one nav level (device → model → fleet), while a
/// breadcrumb jumps straight to any ancestor.
#[must_use]
pub fn back_button() -> Node {
    touchable(
        "back",
        props!(width: BACK_CHIP, height: BACK_CHIP, background: GRAY_90),
        vec![Draw::svg(10.0, 10.0, 20.0, 20.0, &icons::CHEVRON_LEFT, WHITE).with_anti_alias()],
    )
}

const CRUMB_FONT: u32 = TITLE_FONT;
const CRUMB_MAX_CHARS: usize = 22;

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
    let label = truncate_label(crumb.label, CRUMB_MAX_CHARS);
    match crumb.click_id {
        Some(id) => link(id, &label, style!(size: CRUMB_FONT, color: LINK)),
        None => text(label, style!(size: CRUMB_FONT, color: WHITE)),
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
        DeviceStatus::AuthError => (&icons::UNLINK, ERROR, "Not authenticating"),
    }
}

// The ok / degraded / off counts, always all three (even zeros). Auth failures
// show on their own line (see `auth_failures`), not a fourth count column.
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

/// The "N Auth failures" line under the status counts, shown when a miner
/// rejects its credentials — a distinct prompt, not a fourth count column.
#[must_use]
pub fn auth_failures(count: usize) -> Node {
    row(
        props!(gap: 6.0, cross_align: CrossAlign::Center),
        [
            icon(&icons::UNLINK, 20.0, ERROR),
            text(
                fmt!("{count} Auth failures"),
                style!(size: LABEL_FONT, color: ERROR),
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
// Points sit by timestamp within `window`; a `None` value or a downtime gap
// breaks the fill and stroke into separate runs, so the chart never draws a
// false line across missing data. `top_frac` leaves headroom above the chart
// (hero text).
#[must_use]
pub fn area_chart(
    series: &[HistoryDatum],
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
    series: &[HistoryDatum],
    window: ChartWindow,
    w: f32,
    h: f32,
    top_frac: f32,
    nominal: Option<f32>,
    ticks: bool,
) -> Vec<Draw> {
    let anchored = matches!(nominal, Some(n) if n > 0.0);
    // A lone point is the still-forming open bucket, its value refreshed with the
    // live reading every tick — a data-fit axis would chase it. There's no
    // settled value to scale against yet, so draw nothing until a second point
    // commits and the scale settles. A nameplate axis is stable regardless.
    if !anchored && series.len() < 2 {
        return Vec::new();
    }
    // Scale from the settled points — exclude the still-forming last bucket, so
    // the axis re-fits only when a bucket commits, not as the open one refreshes.
    let settled = if series.len() > 1 {
        &series[..series.len() - 1]
    } else {
        series
    };
    let local_max = settled
        .iter()
        .filter_map(|s| s.value)
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

// Min and max of the present values, ignoring gaps; `(0, 0)` when none present.
fn value_range(series: &[HistoryDatum]) -> (f32, f32) {
    series
        .iter()
        .filter_map(|s| s.value)
        .fold(None, |acc: Option<(f32, f32)>, v| {
            Some(acc.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))))
        })
        .unwrap_or((0.0, 0.0))
}

// Fill, trend line, and sample markers for each contiguous run of present,
// gap-free samples; `y_of` maps a value to a y pixel and x is placed by
// timestamp within `window`.
fn draw_runs(
    series: &[HistoryDatum],
    window: ChartWindow,
    w: f32,
    h: f32,
    y_of: &impl Fn(f32) -> f32,
) -> Vec<Draw> {
    let x_of = time_x(window, w);
    let r = marker_radius(h);
    let mut draws = Vec::new();
    for run in runs(series) {
        let pts: Vec<(f32, f32)> = run.iter().map(|&(at, v)| (x_of(at), y_of(v))).collect();
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

// Markers fit when the samples are typically at least a marker apart — judged by
// the median gap, so one unusually tight pair (the short first interval before
// the clock reaches an interval boundary) can't strip markers off an otherwise
// well-spread run. A lone point (no pair) always fits.
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

// A closure placing a timestamp on the x axis: the window's right edge at `w`,
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

// Split into runs of present, gap-free samples: a `None` value or a jump larger
// than `gap_threshold` (a downtime) ends the current run.
fn runs(series: &[HistoryDatum]) -> Vec<Vec<(i64, f32)>> {
    let gap = gap_threshold(series);
    let mut out: Vec<Vec<(i64, f32)>> = Vec::new();
    let mut cur: Vec<(i64, f32)> = Vec::new();
    let mut prev_at: Option<i64> = None;
    for s in series {
        if prev_at.is_some_and(|p| s.at - p > gap) && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        match s.value {
            Some(v) => cur.push((s.at, v)),
            None if !cur.is_empty() => out.push(std::mem::take(&mut cur)),
            None => {}
        }
        prev_at = Some(s.at);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// A time delta beyond which two samples read as a break — 1.5x the median
// spacing, so one big jump (a restart) is an outlier, not the norm.
#[expect(
    clippy::integer_division,
    reason = "the median index is exact enough for a whole-second spacing heuristic"
)]
fn gap_threshold(series: &[HistoryDatum]) -> i64 {
    let mut deltas: Vec<i64> = series
        .windows(2)
        .map(|w| w[1].at - w[0].at)
        .filter(|&d| d > 0)
        .collect();
    if deltas.is_empty() {
        return i64::MAX;
    }
    deltas.sort_unstable();
    let median = deltas[deltas.len() / 2];
    (median + (median >> 1)).max(median + 1)
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
// Control-point y is clamped to each segment's endpoint band,
// so the curve can't overshoot a peak or dip off-canvas.
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
        let (lo, hi) = (p1.1.min(p2.1), p1.1.max(p2.1));
        let cp1 = (
            p1.0 + (p2.0 - p0.0) / 6.0,
            (p1.1 + (p2.1 - p0.1) / 6.0).clamp(lo, hi),
        );
        let cp2 = (
            p2.0 - (p3.0 - p1.0) / 6.0,
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

/// The bottom page control: 1-based page/total framed by up/down chips.
/// `scope` scopes the turn click ids to the fleet or model-detail table.
#[must_use]
pub fn pager(scope: PagerScope, page: usize, page_count: usize) -> Node {
    col(
        props!(gap: 8.0, cross_align: CrossAlign::Center, width: PAGER_W),
        [
            // Leading spacer pushes the cluster to the bottom of its column.
            col(props!(flex: 1.0), Vec::<Node>::new()),
            pager_button(
                &icons::PAGER_UP,
                page > 0,
                pager_click_id(scope, PageTurn::Prev),
            ),
            text(
                fmt!("{} / {}", page + 1, page_count.max(1)),
                // Clip keeps the count single-line (vs the default Wrap) so it
                // can't be split into two lines when the column is tight.
                style!(size: PAGER_FONT, color: LABEL, text_overflow: TextOverflow::Clip),
            ),
            pager_button(
                &icons::PAGER_DOWN,
                page + 1 < page_count,
                pager_click_id(scope, PageTurn::Next),
            ),
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
}
