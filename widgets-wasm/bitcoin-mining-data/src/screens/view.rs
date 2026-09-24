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

#[expect(
    clippy::wildcard_imports,
    reason = "screen code uses the SDK's tree builders, macros, and tokens throughout"
)]
use bmc_wasm_sdk::*;
use units::availability::Availability;

use crate::model::{BitcoinData, Series, SizeBucket, Status, per_petahash};
use crate::screens::icons;
use crate::screens::parts::{self, color};

const FULL_COLUMN_WIDTH: f32 = 400.0;
const FULL_CHART_WIDTH: f32 = 368.0;
/// Inset from the cards' top and bottom edges, as the 26.02 screen padded its stats column.
const FULL_STATS_INSET: f32 = 16.0;

#[derive(Clone, Debug)]
pub struct ViewData {
    pub bucket: SizeBucket,
    pub data: BitcoinData,
    pub status: Status,
    pub now_secs: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum DisplayValue<V = String> {
    Value(V),
    Loading,
    Absent,
}

impl DisplayValue {
    fn into_text_and_color(self) -> (String, Color) {
        match self {
            Self::Value(value) => (value, color::VALUE),
            Self::Loading => (parts::LOADING.to_owned(), color::LABEL),
            Self::Absent => (parts::NOT_AVAILABLE.to_owned(), color::ABSENT),
        }
    }
}

fn availability_value<T, V>(
    source: &Availability<T>,
    render: impl FnOnce(&T) -> Option<V>,
) -> DisplayValue<V> {
    match source {
        Availability::Available(value) => {
            render(value).map_or(DisplayValue::Absent, DisplayValue::Value)
        }
        Availability::Unavailable => DisplayValue::Loading,
        Availability::Failed => DisplayValue::Absent,
    }
}

struct Quantity {
    number: String,
    unit: String,
}

fn primary_value<T>(
    source: &Availability<T>,
    render: impl FnOnce(&T) -> Option<Quantity>,
    sizes: parts::ValueSizes,
) -> Node {
    let content = match availability_value(source, render) {
        DisplayValue::Value(Quantity { number, unit }) => {
            parts::quantity(&number, &unit, sizes, color::VALUE)
        }
        DisplayValue::Loading => {
            let (text, color) = DisplayValue::<String>::Loading.into_text_and_color();
            parts::primary(&text, sizes.number, color)
        }
        DisplayValue::Absent => parts::unavailable(sizes.number),
    };
    col(
        props!(height: parts::font_height(sizes.number), justify_content: Justify::Center),
        [content],
    )
}

fn stat_row<T>(
    label: &str,
    source: &Availability<T>,
    render: impl FnOnce(&T) -> Option<String>,
) -> Node {
    let (value, value_color) = availability_value(source, render).into_text_and_color();
    parts::stat_row(label, &value, value_color)
}

fn chart_or_status<'a, T>(
    source: &'a Availability<T>,
    select_series: impl FnOnce(&'a T) -> &'a Series,
    chart: parts::ChartBox,
    force_color: Option<Color>,
) -> Node {
    match source {
        Availability::Available(value) => {
            parts::sparkline(select_series(value), chart, force_color)
        }
        Availability::Unavailable => col(
            props!(width: chart.width, height: chart.height),
            [parts::muted("Loading history…", 16)],
        ),
        Availability::Failed => parts::unavailable_chart(chart.width, chart.height),
    }
}

const DECK_GRIDLINES: usize = 4;

const fn deck_chart(width: f32, height: f32) -> parts::ChartBox {
    parts::ChartBox {
        width,
        height,
        gridlines: DECK_GRIDLINES,
        corner_label: None,
    }
}

fn title_size(bucket: SizeBucket) -> parts::TitleSize {
    match bucket {
        SizeBucket::Bmm101 => parts::TitleSize {
            icon: 16.0,
            label: 14,
        },
        SizeBucket::Small | SizeBucket::Medium | SizeBucket::Large | SizeBucket::Full => {
            parts::TitleSize {
                icon: 24.0,
                label: 24,
            }
        }
    }
}

/// Label, time and badge font sizes of an adjustment row.
type AdjustmentSizes = (u32, u32, u32);

fn adjustment_sizes(bucket: SizeBucket) -> AdjustmentSizes {
    match bucket {
        SizeBucket::Full => (24, 16, 24),
        SizeBucket::Large => (20, 16, 20),
        // The card is half Large's width, and Large's sizes overrun it.
        SizeBucket::Medium => (18, 14, 18),
        SizeBucket::Bmm101 => (20, 20, 20),
        SizeBucket::Small => (16, 12, 16),
    }
}

fn previous_adjustment_row(view: &ViewData, sizes: AdjustmentSizes) -> Node {
    let (label_size, time_size, badge_size) = sizes;
    let stats = view.data.difficulty_stats.as_option().copied();
    let (when, when_color) = availability_value(&view.data.difficulty_stats, |stats| {
        match (stats.epoch_block, stats.epoch_block_time_secs) {
            (Some(block), Some(block_time_secs)) => {
                Some(parts::previous_adjustment_days(block, block_time_secs))
            }
            _ => None,
        }
    })
    .into_text_and_color();
    parts::adjustment_row(
        "Prev Adjust",
        &when,
        stats.and_then(|stats| stats.previous_adjustment_percent),
        label_size,
        time_size,
        badge_size,
        when_color,
    )
}

fn next_adjustment_row(view: &ViewData, sizes: AdjustmentSizes) -> Node {
    let (label_size, time_size, badge_size) = sizes;
    let stats = view.data.difficulty_stats.as_option().copied();
    let (when, when_color) = availability_value(&view.data.difficulty_stats, |stats| {
        stats
            .estimated_adjustment_at
            .map(|at| parts::relative_days(at, view.now_secs))
    })
    .into_text_and_color();
    parts::adjustment_row(
        "Next Adjust",
        &when,
        stats.and_then(|stats| stats.estimated_adjustment_percent),
        label_size,
        time_size,
        badge_size,
        when_color,
    )
}

/// A primary value's sizes: the unit takes the bucket's label size.
fn value_sizes(bucket: SizeBucket, number: u32) -> parts::ValueSizes {
    let (label_size, _, _) = adjustment_sizes(bucket);
    parts::ValueSizes {
        number,
        unit: label_size,
    }
}

fn deck_value_sizes(bucket: SizeBucket) -> parts::ValueSizes {
    let number = if bucket == SizeBucket::Large { 48 } else { 32 };
    value_sizes(bucket, number)
}

fn difficulty_value(view: &ViewData, sizes: parts::ValueSizes) -> Node {
    primary_value(
        &view.data.difficulty_stats,
        |stats| {
            stats.difficulty.map(|difficulty| {
                let (number, unit) = parts::difficulty_parts(difficulty);
                Quantity {
                    number,
                    unit: unit.to_owned(),
                }
            })
        },
        sizes,
    )
}

fn hashprice_value(view: &ViewData, sizes: parts::ValueSizes) -> Node {
    primary_value(
        &view.data.hashrate_stats,
        |stats| {
            stats.hashprice_per_th_day.map(|value| Quantity {
                number: parts::compact_number(per_petahash(value), 2),
                unit: "USD/PH/Day".to_owned(),
            })
        },
        sizes,
    )
}

fn difficulty_panel(view: &ViewData, chart: Option<parts::ChartBox>, show_previous: bool) -> Node {
    let sizes = adjustment_sizes(view.bucket);
    let mut upper = vec![
        parts::title(
            &icons::PICKAXE,
            WHITE,
            "Bitcoin Difficulty",
            None,
            title_size(view.bucket),
        ),
        row(
            props!(cross_align: CrossAlign::Center),
            [
                difficulty_value(view, deck_value_sizes(view.bucket)),
                spacer(1.0),
                parts::muted(if chart.is_some() { "1 year" } else { "" }, 24),
            ],
        ),
    ];
    if let Some(chart) = chart {
        upper.push(chart_or_status(
            &view.data.year_history,
            |series| series,
            chart,
            Some(color::DOWN),
        ));
    }
    let mut adjustments = Vec::new();
    if show_previous {
        adjustments.push(previous_adjustment_row(view, sizes));
        adjustments.push(parts::divider());
    }
    adjustments.push(next_adjustment_row(view, sizes));
    col(
        props!(
            background: TRANSPARENT,
            padding: 16.0,
            justify_content: Justify::SpaceBetween,
            flex: 1.0
        ),
        [
            col(props!(gap: parts::GAP), upper),
            col(props!(gap: parts::GAP), adjustments),
        ],
    )
}

fn hashprice_panel(view: &ViewData) -> Node {
    col(
        props!(
            background: TRANSPARENT,
            padding: 16.0,
            gap: if view.bucket == SizeBucket::Small { 6.0 } else { 24.0 },
            flex: 1.0
        ),
        [
            parts::title(
                &icons::CHART,
                WHITE,
                "Hash Price",
                None,
                title_size(view.bucket),
            ),
            hashprice_value(view, deck_value_sizes(view.bucket)),
        ],
    )
}

fn price_panel(view: &ViewData, chart: Option<parts::ChartBox>) -> Node {
    let stats = view.data.price_stats.as_option().copied();
    let summary = col(
        props!(gap: parts::GAP),
        [
            parts::title(
                &icons::BTC,
                TRANSPARENT,
                "BTC-USD",
                Some(parts::trend(
                    stats.and_then(|stats| stats.change_24h_percent),
                    true,
                    if view.bucket == SizeBucket::Full {
                        24
                    } else {
                        20
                    },
                )),
                title_size(view.bucket),
            ),
            primary_value(
                &view.data.price_stats,
                |stats| {
                    stats.price.map(|price| Quantity {
                        number: format_number!(price, 0),
                        unit: "USD".to_owned(),
                    })
                },
                value_sizes(view.bucket, 32),
            ),
        ],
    );
    let mut children = vec![summary];
    if let Some(chart) = chart {
        children.push(chart_or_status(
            &view.data.day_history,
            |history| &history.price,
            chart,
            None,
        ));
    }
    col(
        props!(
            background: TRANSPARENT,
            padding: 16.0,
            justify_content: Justify::SpaceBetween,
            flex: 1.0
        ),
        children,
    )
}

fn hashrate_panel(view: &ViewData) -> Node {
    let change = view
        .data
        .day_history
        .as_option()
        .and_then(|history| parts::series_change_percent(&history.hashrate));
    let chart = chart_or_status(
        &view.data.day_history,
        |history| &history.hashrate,
        deck_chart(FULL_CHART_WIDTH, 98.0),
        None,
    );
    let summary = col(
        props!(gap: parts::GAP),
        [
            parts::title(
                &icons::METER,
                WHITE,
                "Hashrate",
                Some(parts::trend(change, true, 24)),
                title_size(view.bucket),
            ),
            primary_value(
                &view.data.hashrate_stats,
                |stats| {
                    stats.current.map(|rate| {
                        let (number, unit) = rate.format_si_parts(4);
                        Quantity { number, unit }
                    })
                },
                value_sizes(view.bucket, 32),
            ),
        ],
    );
    col(
        props!(
            background: TRANSPARENT,
            padding: 16.0,
            justify_content: Justify::SpaceBetween,
            flex: 1.0
        ),
        [summary, chart],
    )
}

fn network_stats(view: &ViewData) -> Node {
    col(
        props!(justify_content: Justify::SpaceBetween, flex: 1.0),
        [
            stat_row("Avg. Fees per Block", &view.data.hashrate_stats, |stats| {
                stats
                    .avg_fees_btc
                    .map(|value| fmt!("{} BTC", format_number!(value, 3)))
            }),
            stat_row("Fees % of Block Rew.", &view.data.hashrate_stats, |stats| {
                stats
                    .fees_percent
                    .map(|value| fmt!("{} %", format_number!(value, 2)))
            }),
            stat_row("Total Mining Rev.", &view.data.hashrate_stats, |stats| {
                stats.revenue.map(parts::compact_revenue)
            }),
            stat_row(
                "Curr. Epoch Blk. Time",
                &view.data.difficulty_stats,
                |stats| stats.epoch_block_time_secs.map(parts::duration_minutes),
            ),
            stat_row("Block Height", &view.data.latest_block, |height| {
                Some(fmt!("{}", height))
            }),
            stat_row("Blocks in last 24h", &view.data.blocks_24h, |count| {
                Some(fmt!("{}/144", count))
            }),
            stat_row("Blocks this Epoch", &view.data.difficulty_stats, |stats| {
                stats.epoch_block.map(|block| fmt!("{}/2016", block))
            }),
        ],
    )
}

fn small(view: &ViewData) -> Node {
    col(
        props!(background: color::BACKGROUND, flex: 1.0),
        [
            col(props!(height: 132.0), [difficulty_panel(view, None, false)]),
            parts::divider(),
            hashprice_panel(view),
        ],
    )
}

const BMM101_SPACING: f32 = 16.0;
const BMM101_VALUE_SIZE: u32 = 40;
/// Three gridlines, as the frame draws: a box this short would crowd the Deck's four.
/// The span label sits in the chart's corner: the value row that holds it on the Deck
/// is the row the chart itself occupies here.
const BMM101_CHART: parts::ChartBox = parts::ChartBox {
    width: 253.0,
    height: 78.0,
    gridlines: 3,
    corner_label: Some("1 year"),
};

/// The year chart runs beside the difficulty value here, not under it as on the Deck.
fn bmm101(view: &ViewData) -> Node {
    let title_size = title_size(view.bucket);
    let sizes = adjustment_sizes(view.bucket);
    let value_sizes = value_sizes(view.bucket, BMM101_VALUE_SIZE);
    let chart = chart_or_status(
        &view.data.year_history,
        |series| series,
        BMM101_CHART,
        Some(color::DOWN),
    );
    col(
        props!(
            background: color::BACKGROUND,
            padding: BMM101_SPACING,
            gap: BMM101_SPACING,
            flex: 1.0
        ),
        [
            row(
                props!(cross_align: CrossAlign::Start, gap: parts::GAP),
                [
                    col(
                        props!(gap: parts::GAP, flex: 1.0),
                        [
                            parts::title(
                                &icons::PICKAXE,
                                WHITE,
                                "Bitcoin Difficulty",
                                None,
                                title_size,
                            ),
                            difficulty_value(view, value_sizes),
                        ],
                    ),
                    chart,
                ],
            ),
            previous_adjustment_row(view, sizes),
            next_adjustment_row(view, sizes),
            parts::divider(),
            col(
                props!(gap: parts::GAP),
                [
                    parts::title(&icons::CHART, WHITE, "Hash Price", None, title_size),
                    hashprice_value(view, value_sizes),
                ],
            ),
        ],
    )
}

fn medium(view: &ViewData) -> Node {
    row(
        props!(
            background: color::BACKGROUND,
            cross_align: CrossAlign::Center,
            flex: 1.0
        ),
        [
            col(
                props!(width: 319.0, height: 238.0),
                [difficulty_panel(view, Some(deck_chart(287.0, 42.0)), true)],
            ),
            col(
                props!(width: 1.0, height: 224.0, background: color::BORDER),
                [],
            ),
            col(
                props!(height: 238.0, flex: 1.0),
                [
                    col(props!(height: 128.0), [hashprice_panel(view)]),
                    parts::divider(),
                    price_panel(view, None),
                ],
            ),
        ],
    )
}

fn large(view: &ViewData) -> Node {
    col(
        props!(background: color::BACKGROUND, flex: 1.0),
        [
            col(
                props!(height: 326.0),
                [difficulty_panel(view, Some(deck_chart(606.0, 76.0)), true)],
            ),
            parts::divider(),
            hashprice_panel(view),
        ],
    )
}

fn full(view: &ViewData) -> Node {
    row(
        props!(
            background: color::BACKGROUND,
            padding: 16.0,
            gap: 24.0,
            flex: 1.0
        ),
        [
            col(
                props!(width: FULL_COLUMN_WIDTH, gap: parts::GAP),
                [
                    col(
                        props!(height: 312.0),
                        [parts::bordered([difficulty_panel(
                            view,
                            Some(deck_chart(FULL_CHART_WIDTH, 76.0)),
                            true,
                        )])],
                    ),
                    parts::bordered([hashprice_panel(view)]),
                ],
            ),
            col(
                props!(flex: 1.0),
                [
                    col(props!(height: FULL_STATS_INSET), []),
                    network_stats(view),
                    col(props!(height: FULL_STATS_INSET), []),
                ],
            ),
            col(
                props!(width: FULL_COLUMN_WIDTH, gap: parts::GAP),
                [
                    parts::bordered([price_panel(view, Some(deck_chart(FULL_CHART_WIDTH, 98.0)))]),
                    parts::bordered([hashrate_panel(view)]),
                ],
            ),
        ],
    )
}

#[must_use]
pub fn bitcoin_mining_view(view: &ViewData) -> Node {
    let root = match view.bucket {
        SizeBucket::Small => small(view),
        SizeBucket::Bmm101 => bmm101(view),
        SizeBucket::Medium => medium(view),
        SizeBucket::Large => large(view),
        SizeBucket::Full => full(view),
    };
    match view.status {
        Status::Ready => root,
        Status::Stale(last_success) => status_overlay::with_stale_overlay(
            root,
            SystemTime {
                unix_secs: last_success,
            },
            ViewportShape::Rectangular,
        ),
        Status::Failed => status_overlay::with_error_overlay(
            root,
            "Bitcoin data unavailable",
            ViewportShape::Rectangular,
        ),
        Status::RateLimited => status_overlay::with_overlay(
            root,
            tag(
                TagKind::Warning,
                TagIcon::Default,
                text(
                    "Rate limited — retrying in 10 min",
                    style!(size: 12, color: ORANGE_40),
                ),
            ),
            ViewportShape::Rectangular,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::fixtures;

    #[test]
    fn loading_value_keeps_muted_presentation() {
        let loading: Availability<()> = Availability::Unavailable;

        assert_eq!(
            availability_value(&loading, |_| None::<String>),
            DisplayValue::Loading
        );
        assert_eq!(
            DisplayValue::Loading.into_text_and_color(),
            (parts::LOADING.to_owned(), color::LABEL)
        );
    }

    #[test]
    fn absent_value_uses_not_available_placeholder() {
        let failed: Availability<()> = Availability::Failed;

        assert_eq!(
            availability_value(&failed, |_| None).into_text_and_color(),
            (parts::NOT_AVAILABLE.to_owned(), color::ABSENT)
        );
    }

    #[test]
    fn a_primary_value_sets_its_unit_apart_on_the_number_line() {
        let sizes = parts::ValueSizes {
            number: 40,
            unit: 20,
        };
        let value = primary_value(
            &Availability::Available(()),
            |()| {
                Some(Quantity {
                    number: "56.20".to_owned(),
                    unit: "USD/PH/Day".to_owned(),
                })
            },
            sizes,
        );

        let Node::Column(_, content) = value else {
            panic!("BUG: a primary value is boxed in a column");
        };
        let Some(Node::Paragraph { spans, .. }) = content.first() else {
            panic!("BUG: a present value renders as one paragraph");
        };
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].text.as_str(), spans[0].size), ("56.20", None));
        assert_eq!(
            (spans[1].text.as_str(), spans[1].size, spans[1].color),
            (
                format!("{}USD/PH/Day", typography::NBSP).as_str(),
                Some(20),
                Some(color::LABEL)
            )
        );
    }

    /// The top row of the BMM101 layout: the title-and-value column and the chart beside it.
    fn bmm101_top_row(view: &ViewData) -> (Vec<Node>, Node) {
        bmc_wasm_sdk::assets::init_test_registrars();
        let Node::Column(_, blocks) = bitcoin_mining_view(view) else {
            panic!("BUG: the BMM101 root must be a column");
        };
        let Some(Node::Row(_, top)) = blocks.first() else {
            panic!("BUG: the BMM101 layout must open with the difficulty row");
        };
        let (Some(Node::Column(_, left)), Some(chart)) = (top.first(), top.get(1)) else {
            panic!("BUG: the difficulty row must hold the value column and the chart");
        };
        (left.clone(), chart.clone())
    }

    #[test]
    fn bmm101_runs_the_chart_beside_the_value_with_its_span_drawn_in_its_corner() {
        let (left, chart) = bmm101_top_row(&fixtures::healthy(SizeBucket::Bmm101));

        let Some(Node::Row(_, title)) = left.first() else {
            panic!("BUG: the value column must open with the title");
        };
        let Some(Node::Canvas { props, .. }) = title.first() else {
            panic!("BUG: the title must lead with its icon");
        };
        assert_eq!(props.width, 16.0);
        let Node::Canvas { props, draws, .. } = chart else {
            panic!("BUG: a drawn history is a canvas");
        };
        assert_eq!((props.width, props.height), (253.0, 78.0));
        // The label is the canvas's last draw: over the line, and on the one text path
        // the host outlines, so the outline it asks for is the outline it gets.
        let Some(Draw::Text { x, y, text, style }) = draws.last() else {
            panic!("BUG: the span label must be drawn last, over the line");
        };
        assert_eq!((text.as_str(), *x, *y), ("1 year", 253.0 - 4.0, 5.0));
        assert_eq!(style.align, TextAlign::Right);
        assert!(
            style.outline_width > 0.0 && style.outline_color != TRANSPARENT,
            "the label is outlined to read where the line peaks under it"
        );
    }

    #[test]
    fn bmm101_stacks_both_adjustments_then_the_hash_price() {
        bmc_wasm_sdk::assets::init_test_registrars();
        let Node::Column(_, blocks) = bitcoin_mining_view(&fixtures::healthy(SizeBucket::Bmm101))
        else {
            panic!("BUG: the BMM101 root must be a column");
        };

        let labels: Vec<String> = blocks.iter().filter_map(first_text).collect();
        assert_eq!(
            labels,
            [
                "Bitcoin Difficulty",
                "Prev Adjust",
                "Next Adjust",
                "Hash Price"
            ]
        );
    }

    /// The first run of text under `node`, in tree order — a block's label,
    /// with its non-breaking spaces read as plain ones.
    fn first_text(node: &Node) -> Option<String> {
        match node {
            Node::Paragraph { spans, .. } => spans
                .first()
                .map(|span| span.text.replace(typography::NBSP, " ")),
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                children.iter().find_map(first_text)
            }
            _ => None,
        }
    }

    #[test]
    fn bmm101_keeps_the_chart_box_while_history_loads() {
        let (_, chart) = bmm101_top_row(&fixtures::loading(SizeBucket::Bmm101));

        let Node::Column(props, _) = chart else {
            panic!("BUG: a loading history must keep its box");
        };
        assert_eq!((props.width, props.height), (253.0, 78.0));
    }

    #[test]
    fn full_insets_the_stats_column_from_the_cards_edges() {
        bmc_wasm_sdk::assets::init_test_registrars();
        let Node::Row(_, columns) = bitcoin_mining_view(&fixtures::healthy(SizeBucket::Full))
        else {
            panic!("BUG: the Full root must be a row of columns");
        };
        let Some(Node::Column(_, middle)) = columns.get(1) else {
            panic!("BUG: the stats column sits between the card columns");
        };

        let (Some(Node::Column(top, _)), Some(Node::Column(bottom, _))) =
            (middle.first(), middle.last())
        else {
            panic!("BUG: the stats column is bracketed by two spacers");
        };
        assert_eq!((top.height, bottom.height), (16.0, 16.0));
        assert_eq!(
            first_text(&middle[1]).as_deref(),
            Some("Avg. Fees per Block")
        );
    }
}
