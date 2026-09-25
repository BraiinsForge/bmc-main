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

//! The Big Chart screen: one thin layout per design size.
//! The Deck's chart canvas bleeds to the frame edges — its gutters
//! carry the margins — while the header lines sit in a padded block above it;
//! BMM101's sits inside the padding.

use bmc_wasm_sdk::types::SiPrefix;
#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::model::{PayoutKind, PoolData, SizeBucket};
use crate::screens::parts::{self, TitleSize, color, font, space};
use crate::screens::plot::{self, ChartSpec, DECK_TICKS, TickSpec};

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

/// BMM101's header and hero-line boxes: Figma's line box for their text.
const BMM101_HEADER_H: f32 = 18.0;
const BMM101_HERO_H: f32 = 26.0;

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
                parts::unbound_body(view.bucket, view.width, &view.bind_hint),
            ],
        );
    }
    if view.data.access_denied {
        // The account joins the header where the normal layouts show it.
        let account = match view.bucket {
            SizeBucket::Small | SizeBucket::Medium | SizeBucket::Bmm101 => None,
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
        SizeBucket::Bmm101 => bmm101(view),
    }
}

fn header(account: Option<&str>) -> Node {
    row(
        props!(height: HEADER_H, cross_align: CrossAlign::Center),
        [parts::header_left(account, TitleSize::DECK)],
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
        ticks: DECK_TICKS,
        marker_size: None,
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            col(
                props!(padding: space::PADDING, gap: space::GAP),
                [header(None), hashrate_hero(&view.data, font::BODY)],
            ),
            chart(view, view.width, chart_h, &spec, font::BODY),
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
        ticks: DECK_TICKS,
        marker_size: None,
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            row(
                props!(padding: space::PADDING, height: HEADER_H + 2.0 * space::PADDING, cross_align: CrossAlign::Center, gap: space::GAP),
                [
                    parts::header_left(None, TitleSize::DECK),
                    spacer(1.0),
                    compact_hero(view),
                ],
            ),
            chart(view, view.width, chart_h, &spec, font::BODY),
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
        ticks: DECK_TICKS,
        marker_size: None,
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            col(
                props!(padding: space::PADDING, gap: space::GAP),
                [header(view.account.as_deref()), legend(view)],
            ),
            chart(view, view.width, chart_h, &spec, font::BODY),
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
        ticks: DECK_TICKS,
        marker_size: Some(FULL_MARKER),
    };
    col(
        props!(background: color::BG, flex: 1.0),
        [
            row(
                props!(padding: space::PADDING, height: HEADER_H + 2.0 * space::PADDING, cross_align: CrossAlign::Center, gap: 24.0),
                [header(view.account.as_deref()), legend(view)],
            ),
            chart(view, view.width, chart_h, &spec, font::BODY),
        ],
    )
}

/// Header with the account, the hero line with the active workers pushed right,
/// and a labelled chart inset by the frame padding — every gap the padding's width.
/// The chart takes what the fixed lines leave.
fn bmm101(view: &BigChartViewData) -> Node {
    let chart_w = view.width - 2.0 * space::PADDING;
    let chart_h = view.height - 4.0 * space::PADDING - BMM101_HEADER_H - BMM101_HERO_H;
    let spec = ChartSpec {
        left_gutter: 36.0,
        right_gutter: 40.0,
        hashrate_ticks: true,
        workers_ticks: workers_line_on(view),
        x_band: None,
        solid_baseline: false,
        grid_steps: 3,
        ticks: TickSpec {
            font: font::bmm101::TICK,
            inset: 8.0,
            at_edges: true,
            shared_hashrate_scale: true,
            hero_unit: hero_prefix(&view.data),
            shared_worker_scale: true,
        },
        marker_size: None,
    };
    let mut hero_line = vec![hashrate_hero(&view.data, font::bmm101::BODY)];
    if view.worker_states {
        hero_line.push(spacer(1.0));
        hero_line.push(workers_pair(view, font::bmm101::BODY));
    }
    col(
        props!(padding: space::PADDING, gap: space::PADDING, background: color::BMM101_BG, flex: 1.0),
        [
            row(
                props!(height: BMM101_HEADER_H, cross_align: CrossAlign::Center),
                [parts::header_left(
                    view.account.as_deref(),
                    TitleSize::BMM101,
                )],
            ),
            row(
                props!(height: BMM101_HERO_H, cross_align: CrossAlign::Center),
                hero_line,
            ),
            chart(view, chart_w, chart_h, &spec, font::bmm101::BODY),
        ],
    )
}

/// The chart canvas, or a loading block over the plot's own footprint
/// (gutters and x-band left clear) while no history arrived yet.
/// The spec itself gates the extras: labels need its band, markers its size.
fn chart(
    view: &BigChartViewData,
    width: f32,
    height: f32,
    spec: &ChartSpec,
    callout_size: u32,
) -> Node {
    let Some(history) = view.data.hashrate_history.as_option() else {
        let plot_w = width - spec.left_gutter - spec.right_gutter;
        let plot_h = height - spec.x_band.unwrap_or(0.0);
        let plot = parts::placeholder(
            &view.data.hashrate_history,
            parts::absent_block(plot_w, plot_h, parts::callout::HISTORY, callout_size),
            parts::skeleton_block(plot_w, plot_h),
        );
        return center(props!(flex: 1.0), [plot]);
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
fn hashrate_hero(data: &PoolData, size: u32) -> Node {
    match hero_hashrate(data) {
        Some((label, value)) => parts::stat_pair(&label, &value, color::HASHRATE_VALUE, size),
        None => parts::placeholder(
            &data.hashrate_5m,
            parts::absent(parts::callout::HASHRATE, size),
            parts::skeleton(chars::HERO_PAIR, size),
        ),
    }
}

/// The active-workers count: "Active Workers: 2 395".
fn workers_pair(view: &BigChartViewData, size: u32) -> Node {
    match view.data.workers.as_option() {
        Some(workers) => parts::stat_pair(
            "Active Workers: ",
            &format_number!(workers.active, 0),
            color::WORKERS,
            size,
        ),
        None => parts::placeholder(
            &view.data.workers,
            parts::absent(parts::callout::WORKERS, size),
            parts::skeleton(chars::WORKERS_PAIR, size),
        ),
    }
}

/// The legend line: the hashrate hero plus the active-workers count.
/// Each cell keeps its slot while loading, so the line never reflows.
fn legend(view: &BigChartViewData) -> Node {
    let mut cells = vec![hashrate_hero(&view.data, font::BODY)];
    if view.worker_states {
        cells.push(workers_pair(view, font::BODY));
    }
    row(props!(gap: 16.0, cross_align: CrossAlign::Center), cells)
}

/// The Medium frame's right-aligned hero run: `349.8 Ph/s · 2395 Workers`.
fn compact_hero(view: &BigChartViewData) -> Node {
    // The run's spans share one paragraph, and a paragraph cannot hold
    // a skeleton node mid-line — while either source is pending,
    // the whole line loads as one bar. A failed workers source instead drops
    // its own spans below, leaving the hashrate standing.
    let workers_pending = view.worker_states
        && view.data.workers.as_option().is_none()
        && !view.data.workers.failed();
    let Some(hashrate) = view.data.hashrate_5m.as_option() else {
        return parts::placeholder(
            &view.data.hashrate_5m,
            parts::absent(parts::callout::HASHRATE, font::BODY),
            parts::skeleton(
                if view.worker_states {
                    chars::COMPACT_FULL
                } else {
                    chars::COMPACT_HERO
                },
                font::BODY,
            ),
        );
    };
    if workers_pending {
        return parts::skeleton(chars::COMPACT_FULL, font::BODY);
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
    parts::text_run(spans, font::BODY)
}

/// The hashrate hero as (label, value): the label names the SI unit the
/// value is scaled to, so the pair never drifts apart.
fn hero_hashrate(data: &PoolData) -> Option<(String, String)> {
    data.hashrate_5m.as_option().map(|hashrate| {
        let (value, unit) = hashrate.format_si_parts(4);
        (fmt!("5m HR ({unit}): "), value)
    })
}

/// The prefix of the unit the hero names, so the chart's ticks can count in it.
fn hero_prefix(data: &PoolData) -> Option<SiPrefix> {
    data.hashrate_5m
        .as_option()
        .copied()
        .and_then(plot::si_prefix)
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::typography::NBSP;

    use super::*;
    use crate::screens::fixtures;

    /// Every paragraph the tree would draw, in tree order;
    /// the chart's tick labels are canvas draws, not paragraphs.
    fn texts(node: &Node) -> Vec<String> {
        let mut out = Vec::new();
        collect_texts(node, &mut out);
        out
    }

    fn collect_texts(node: &Node, out: &mut Vec<String>) {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                for child in children {
                    collect_texts(child, out);
                }
            }
            Node::Paragraph { spans, .. } => {
                out.push(spans.iter().map(|span| span.text.as_str()).collect());
            }
            _ => {}
        }
    }

    #[test]
    fn the_bmm101_frame_reads_top_down_as_designed() {
        bmc_wasm_sdk::assets::init_test_registrars();
        let view = fixtures::sample_big_chart(SizeBucket::Bmm101, true, 5.0);
        assert_eq!(
            texts(&big_chart_view(&view)),
            [
                "Braiins Pool".to_owned(),
                "(".to_owned(),
                "user.braiins".to_owned(),
                ")".to_owned(),
                "5m HR (PH/s): 500,0".to_owned(),
                format!("Active Workers: 1{NBSP}628"),
            ]
        );
    }

    /// The chart is the frame's last canvas; the header logo draws before it.
    fn chart_height(node: &Node) -> Option<f32> {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                children.iter().rev().find_map(chart_height)
            }
            Node::Canvas { props, .. } => Some(props.height),
            _ => None,
        }
    }

    #[test]
    fn the_bmm101_chart_keeps_its_height_whatever_the_hero_line_holds() {
        bmc_wasm_sdk::assets::init_test_registrars();
        for worker_states in [true, false] {
            for spread in [0.0, 9.0] {
                let view = fixtures::sample_big_chart(SizeBucket::Bmm101, worker_states, spread);
                assert_eq!(
                    chart_height(&big_chart_view(&view)),
                    Some(212.0),
                    "worker states {worker_states}, spread {spread}"
                );
            }
        }
    }

    #[test]
    fn the_bmm101_frame_drops_the_workers_with_worker_states_off() {
        bmc_wasm_sdk::assets::init_test_registrars();
        let view = fixtures::sample_big_chart(SizeBucket::Bmm101, false, 5.0);
        let texts = texts(&big_chart_view(&view));
        assert_eq!(
            texts.last().map(String::as_str),
            Some("5m HR (PH/s): 500,0")
        );
    }

    #[test]
    fn the_ticks_count_in_the_unit_the_hero_names() {
        assert_eq!(
            hero_prefix(&fixtures::sample_data(5.0)),
            Some(SiPrefix::Peta)
        );
        assert_eq!(hero_prefix(&PoolData::default()), None);
    }
}
