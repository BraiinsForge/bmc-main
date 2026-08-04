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

//! The Big Chart screen: one thin layout per design size. The chart canvas
//! bleeds to the frame edges — its gutters carry the margins — while the
//! header lines sit in a padded block above it.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::model::{PayoutKind, PoolData, SizeBucket};
use crate::screens::parts::{self, color, font, space};
use crate::screens::plot::{self, ChartSpec};

/// Everything the Big Chart screen shows.
#[derive(Clone, Debug)]
pub struct BigChartViewData {
    pub bucket: SizeBucket,
    /// The viewport in pixels; the chart fills it minus header and padding.
    pub width: f32,
    pub height: f32,
    pub account: Option<String>,
    /// Where the placeholder state sends the operator to bind an account.
    pub bind_hint: parts::BindHint,
    pub worker_states: bool,
    pub data: PoolData,
    /// Time labels under the chart, as (fraction of the span, text);
    /// only the Fullscreen layout draws them.
    pub x_labels: Vec<(f32, String)>,
}

/// The header line's height in every frame's vertical budget.
const HEADER_H: f32 = 40.0;

/// The Fullscreen frame's x-label band and payout icon size.
const FULL_X_BAND: f32 = 40.0;
const FULL_MARKER: f32 = 36.0;

/// Glyph counts the loading bars stand in for, one per line's own strings.
mod chars {
    /// "5m HR (PH/s): 349,8"
    pub const HERO_PAIR: f32 = 19.0;
    /// "Active Workers: 2 495"
    pub const WORKERS_PAIR: f32 = 21.0;
    /// "500,0 Ph/s"
    pub const COMPACT_HERO: f32 = 10.0;
    /// "500,0 Ph/s · 2 495 Workers"
    pub const COMPACT_FULL: f32 = 26.0;
}

/// The Big Chart screen for one widget viewport.
#[must_use]
pub fn big_chart_view(view: &BigChartViewData) -> Node {
    if view.account.is_none() {
        return col(
            props!(padding: space::PADDING, gap: space::GAP, background: color::BG, flex: 1.0),
            [
                header(None),
                parts::unbound_body(view.bucket, &view.bind_hint),
            ],
        );
    }
    if view.data.access_denied {
        // The account joins the header where the normal layouts show it.
        let account = match view.bucket {
            SizeBucket::Small | SizeBucket::Medium => None,
            SizeBucket::Large | SizeBucket::Full => view.account.as_deref(),
        };
        return col(
            props!(padding: space::PADDING, gap: space::GAP, background: color::BG, flex: 1.0),
            [header(account), parts::denied_body(view.bucket)],
        );
    }
    match view.bucket {
        SizeBucket::Small => small(view),
        SizeBucket::Medium => medium(view),
        SizeBucket::Large => large(view),
        SizeBucket::Full => full(view),
    }
}

fn header(account: Option<&str>) -> Node {
    row(
        props!(height: HEADER_H, cross_align: CrossAlign::Center),
        [parts::header_left(account)],
    )
}

/// Title line, the hashrate hero, and a full-bleed label-less chart.
fn small(view: &BigChartViewData) -> Node {
    // The padded header block: padding above and below, header, gap, hero line.
    let header_block = 2.0 * space::PADDING + HEADER_H + space::GAP + 24.0;
    let chart_h = view.height - header_block - space::PADDING;
    let spec = ChartSpec {
        left_gutter: 0.0,
        right_gutter: 0.0,
        hashrate_ticks: false,
        workers_ticks: false,
        x_band: None,
        solid_baseline: false,
        grid_steps: 2,
        tick_font: font::TICK,
        marker_size: None,
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            col(
                props!(padding: space::PADDING, gap: space::GAP),
                [header(None), hashrate_hero(&view.data)],
            ),
            chart(view, view.width, chart_h, &spec),
        ],
    )
}

/// Compact one-line header — title left, `X Ph/s · N Workers` right.
fn medium(view: &BigChartViewData) -> Node {
    let header_block = HEADER_H + 2.0 * space::PADDING;
    let chart_h = view.height - header_block - space::GAP;
    let spec = ChartSpec {
        left_gutter: 64.0,
        right_gutter: 68.0,
        hashrate_ticks: true,
        workers_ticks: workers_line_on(view),
        x_band: None,
        solid_baseline: true,
        grid_steps: 3,
        tick_font: font::TICK,
        marker_size: None,
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            row(
                props!(padding: space::PADDING, height: HEADER_H + 2.0 * space::PADDING, cross_align: CrossAlign::Center, gap: space::GAP),
                [parts::header_left(None), spacer(1.0), compact_hero(view)],
            ),
            chart(view, view.width, chart_h, &spec),
        ],
    )
}

/// Two header lines — title + account, then the legend — over the chart.
fn large(view: &BigChartViewData) -> Node {
    let header_block = 2.0 * space::PADDING + HEADER_H + space::GAP + 24.0;
    let chart_h = view.height - header_block - space::PADDING;
    let spec = ChartSpec {
        left_gutter: 64.0,
        right_gutter: 68.0,
        hashrate_ticks: true,
        workers_ticks: workers_line_on(view),
        x_band: None,
        solid_baseline: true,
        grid_steps: 3,
        tick_font: font::TICK,
        marker_size: None,
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            col(
                props!(padding: space::PADDING, gap: space::GAP),
                [header(view.account.as_deref()), legend(view)],
            ),
            chart(view, view.width, chart_h, &spec),
        ],
    )
}

/// One header line with the legend inline, x-time labels, payout markers.
fn full(view: &BigChartViewData) -> Node {
    // The x-label band inside the canvas carries the bottom margin.
    let chart_h = view.height - HEADER_H - 2.0 * space::PADDING;
    let spec = ChartSpec {
        left_gutter: 80.0,
        right_gutter: 72.0,
        hashrate_ticks: true,
        workers_ticks: workers_line_on(view),
        x_band: Some(FULL_X_BAND),
        solid_baseline: true,
        grid_steps: 3,
        tick_font: font::TICK,
        marker_size: Some(FULL_MARKER),
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            row(
                props!(padding: space::PADDING, height: HEADER_H + 2.0 * space::PADDING, cross_align: CrossAlign::Center, gap: 24.0),
                [header(view.account.as_deref()), legend(view)],
            ),
            chart(view, view.width, chart_h, &spec),
        ],
    )
}

/// The chart canvas, or a loading block over the plot's own footprint
/// (gutters and x-band left clear) while no history arrived yet.
/// The spec itself gates the extras: labels need its band, markers its size.
fn chart(view: &BigChartViewData, width: f32, height: f32, spec: &ChartSpec) -> Node {
    let Some(history) = view.data.hashrate_history.as_option() else {
        let plot_w = width - spec.left_gutter - spec.right_gutter;
        let plot_h = height - spec.x_band.unwrap_or(0.0);
        return center(props!(flex: 1.0), [parts::skeleton_block(plot_w, plot_h)]);
    };
    let workers_history = view
        .worker_states
        .then(|| view.data.workers_history.as_option())
        .flatten();
    let markers = if spec.marker_size.is_some() {
        payout_markers(view, history)
    } else {
        Vec::new()
    };
    plot::line_chart(
        history,
        workers_history,
        width,
        height,
        spec,
        &view.x_labels,
        &markers,
    )
}

fn workers_line_on(view: &BigChartViewData) -> bool {
    view.worker_states && view.data.workers_history.as_option().is_some()
}

/// Completed payouts inside the chart's window, as time fractions.
/// A payout whose rail the reply did not name draws no marker:
/// there is no icon for it, and either of the two would say something untrue.
fn payout_markers(
    view: &BigChartViewData,
    history: &crate::model::Series,
) -> Vec<(f32, PayoutKind)> {
    let (Some(from), Some(to)) = (history.from, history.to) else {
        return Vec::new();
    };
    view.data
        .payouts
        .as_option()
        .map(|payouts| {
            payouts
                .iter()
                .filter_map(|payout| {
                    let kind = payout.kind?;
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "a 0..=1 fraction is exact in f32"
                    )]
                    crate::chart::time_fraction(payout.at, from, to)
                        .map(|fraction| (fraction as f32, kind))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The hashrate hero on its own line: "5m HR (PH/s): 349.8".
fn hashrate_hero(data: &PoolData) -> Node {
    match hero_hashrate(data) {
        Some((label, value)) => parts::stat_pair(&label, &value, color::HASHRATE_VALUE),
        None => parts::skeleton(chars::HERO_PAIR),
    }
}

/// The legend line: the hashrate hero plus the active-workers count.
/// Each cell keeps its slot while loading, so the line never reflows.
fn legend(view: &BigChartViewData) -> Node {
    let mut cells = vec![hashrate_hero(&view.data)];
    if view.worker_states {
        cells.push(match view.data.workers.as_option() {
            Some(workers) => parts::stat_pair(
                "Active Workers: ",
                &format_number!(workers.active, 0),
                color::WORKERS,
            ),
            None => parts::skeleton(chars::WORKERS_PAIR),
        });
    }
    row(props!(gap: 16.0, cross_align: CrossAlign::Center), cells)
}

/// The Medium frame's right-aligned hero run: `349.8 Ph/s · 2395 Workers`.
fn compact_hero(view: &BigChartViewData) -> Node {
    // The run's spans share one paragraph, and a paragraph cannot hold
    // a skeleton node mid-line — while either source is pending,
    // the whole line loads as one bar.
    let workers_pending = view.worker_states && view.data.workers.as_option().is_none();
    let Some(hashrate) = view.data.hashrate_5m.as_option() else {
        return parts::skeleton(if view.worker_states {
            chars::COMPACT_FULL
        } else {
            chars::COMPACT_HERO
        });
    };
    if workers_pending {
        return parts::skeleton(chars::COMPACT_FULL);
    }
    let mut spans = Vec::new();
    let (value, unit) = hashrate.format_si_parts(4);
    spans.push(parts::value_span(&value, color::HASHRATE_VALUE));
    spans.push(span(fmt!(" {unit}"), ()));
    if view.worker_states
        && let Some(workers) = view.data.workers.as_option()
    {
        spans.push(span(" · ", style!(color: color::SEPARATOR)));
        spans.push(parts::value_span(
            &format_number!(workers.active, 0),
            color::WORKERS,
        ));
        spans.push(span(
            if workers.active == 1 {
                " Worker"
            } else {
                " Workers"
            },
            (),
        ));
    }
    parts::text_run(spans)
}

/// The hashrate hero as (label, value): the label names the SI unit the
/// value is scaled to, so the pair never drifts apart.
fn hero_hashrate(data: &PoolData) -> Option<(String, String)> {
    data.hashrate_5m.as_option().map(|hashrate| {
        let (value, unit) = hashrate.format_si_parts(4);
        (fmt!("5m HR ({unit}): "), value)
    })
}
