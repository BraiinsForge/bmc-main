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
/// and device detail, catalogued by variant in the storybook.
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

/// Every status tag stacked for the storybook catalog of inline-status variants.
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
// `top_frac` leaves headroom above the chart (for the dashboard's hero text).
#[must_use]
pub fn area_chart(series: &[f32], w: f32, h: f32, top_frac: f32) -> Vec<Draw> {
    let (min, max) = series
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (max - min).max(1e-3);
    let top = h * top_frac;
    let usable = h - top - BASELINE_INSET;
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

// Large charts: 0-anchored to `nice(max(nominal, window max))` with dashed
// gridlines and optional right-edge tick labels. Sparklines stay bare.
#[must_use]
pub fn scaled_area_chart(
    series: &[f32],
    w: f32,
    h: f32,
    top_frac: f32,
    nominal: Option<f32>,
    ticks: bool,
) -> Vec<Draw> {
    let local_max = series.iter().copied().fold(0.0_f32, f32::max);
    let ceiling_hint = nominal.unwrap_or(0.0).max(local_max);
    let Some((ceiling, step)) = nice_scale(ceiling_hint) else {
        return area_chart(series, w, h, top_frac);
    };
    let top = h * top_frac;
    let plot_h = (h - top - BASELINE_INSET).max(1.0);
    let last = idx_f32(series.len().max(2) - 1);
    let y_of = |v: f32| top + (1.0 - (v / ceiling).clamp(0.0, 1.0)) * plot_h;
    let trend: Vec<(f32, f32)> = series
        .iter()
        .enumerate()
        .map(|(i, &v)| (idx_f32(i) / last * w, y_of(v)))
        .collect();
    let curve = smooth_curve(&trend);
    let mut area = curve.clone();
    area.push((w, h));
    area.push((0.0, h));

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
    draws.push(fill!(area, linear: (CHART.with_alpha(0.55), CHART.with_alpha(0.05))));
    draws.push(path!(curve, stroke: 3.0, color: CHART));
    draws.extend(labels);
    draws
}

// Round axis ceiling ≥ `raw` and gridline step (1/2/5 × 10ⁿ); `None` if raw <= 0.
fn nice_scale(raw: f32) -> Option<(f32, f32)> {
    if raw <= 0.0 {
        return None;
    }
    let step = nice_step(raw / 3.0);
    let ceiling = (raw / step).ceil() * step;
    Some((ceiling, step))
}

fn nice_step(rough: f32) -> f32 {
    let mag = pow10_floor(rough);
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
    snapped * mag
}

// Largest power of ten ≤ `v`, sans `log10`/int casts.
fn pow10_floor(v: f32) -> f32 {
    let mut p = 1.0_f32;
    while p * 10.0 <= v {
        p *= 10.0;
    }
    while p > v {
        p /= 10.0;
    }
    p
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
