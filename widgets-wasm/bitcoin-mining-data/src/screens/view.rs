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

use crate::model::{
    BitcoinData, Series, SizeBucket, Status, TERAHASHES_PER_EXAHASH, TERAHASHES_PER_PETAHASH,
};
use crate::screens::icons;
use crate::screens::parts::{self, color};

const FULL_COLUMN_WIDTH: f32 = 400.0;
const FULL_CHART_WIDTH: f32 = 368.0;

#[derive(Clone, Debug)]
pub struct ViewData {
    pub bucket: SizeBucket,
    pub data: BitcoinData,
    pub status: Status,
    pub now_secs: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum DisplayValue {
    Value(String),
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

fn availability_value<T>(
    source: &Availability<T>,
    render: impl FnOnce(&T) -> Option<String>,
) -> DisplayValue {
    match source {
        Availability::Available(value) => {
            render(value).map_or(DisplayValue::Absent, DisplayValue::Value)
        }
        Availability::Unavailable => DisplayValue::Loading,
        Availability::Failed => DisplayValue::Absent,
    }
}

fn primary_value<T>(
    source: &Availability<T>,
    render: impl FnOnce(&T) -> Option<String>,
    size: u32,
) -> Node {
    let content = match availability_value(source, render) {
        DisplayValue::Absent => parts::unavailable(size),
        value => {
            let (value, value_color) = value.into_text_and_color();
            parts::primary(value, size, value_color)
        }
    };
    col(
        props!(height: parts::font_height(size), justify_content: Justify::Center),
        [content],
    )
}

fn stat_row<T>(
    label: &str,
    source: &Availability<T>,
    render: impl FnOnce(&T) -> Option<String>,
) -> Node {
    let (value, value_color) = availability_value(source, render).into_text_and_color();
    parts::stat_row(label, value, value_color)
}

fn chart_or_status<'a, T>(
    source: &'a Availability<T>,
    select_series: impl FnOnce(&'a T) -> &'a Series,
    width: f32,
    height: f32,
    force_color: Option<Color>,
) -> Node {
    match source {
        Availability::Available(value) => {
            parts::sparkline(select_series(value), width, height, force_color)
        }
        Availability::Unavailable => col(
            props!(width: width, height: height),
            [parts::muted("Loading history…", 16)],
        ),
        Availability::Failed => parts::unavailable_chart(width, height),
    }
}

fn adjustment_sizes(bucket: SizeBucket) -> (u32, u32, u32) {
    match bucket {
        SizeBucket::Full => (24, 16, 24),
        SizeBucket::Large | SizeBucket::Medium => (20, 16, 20),
        SizeBucket::Small => (16, 12, 16),
    }
}

fn difficulty_panel(view: &ViewData, chart: Option<(f32, f32)>, show_previous: bool) -> Node {
    let stats = view.data.difficulty_stats.as_option().copied();
    let value_size = if view.bucket == SizeBucket::Large {
        48
    } else {
        32
    };
    let (label_size, time_size, badge_size) = adjustment_sizes(view.bucket);
    let (previous_adjustment, previous_adjustment_color) =
        availability_value(&view.data.difficulty_stats, |stats| {
            match (stats.epoch_block, stats.epoch_block_time_secs) {
                (Some(block), Some(block_time_secs)) => {
                    Some(parts::previous_adjustment_days(block, block_time_secs))
                }
                _ => None,
            }
        })
        .into_text_and_color();
    let (next_adjustment, next_adjustment_color) =
        availability_value(&view.data.difficulty_stats, |stats| {
            stats
                .estimated_adjustment_at
                .map(|at| parts::relative_days(at, view.now_secs))
        })
        .into_text_and_color();
    let mut upper = vec![
        parts::title(&icons::PICKAXE, WHITE, "Bitcoin Difficulty", None),
        row(
            props!(cross_align: CrossAlign::Center),
            [
                primary_value(
                    &view.data.difficulty_stats,
                    |stats| {
                        stats
                            .difficulty
                            .map(|difficulty| fmt!("{} T", format_number!(difficulty / 1e12, 1)))
                    },
                    value_size,
                ),
                spacer(1.0),
                parts::muted(if chart.is_some() { "1 year" } else { "" }, 24),
            ],
        ),
    ];
    if let Some((width, height)) = chart {
        upper.push(chart_or_status(
            &view.data.year_history,
            |series| series,
            width,
            height,
            Some(color::DOWN),
        ));
    }
    let mut adjustments = Vec::new();
    if show_previous {
        adjustments.push(parts::adjustment_row(
            "Prev Adjust",
            previous_adjustment,
            stats.and_then(|stats| stats.previous_adjustment_percent),
            label_size,
            time_size,
            badge_size,
            previous_adjustment_color,
        ));
        adjustments.push(parts::divider());
    }
    adjustments.push(parts::adjustment_row(
        "Next Adjust",
        next_adjustment,
        stats.and_then(|stats| stats.estimated_adjustment_percent),
        label_size,
        time_size,
        badge_size,
        next_adjustment_color,
    ));
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
    let value_size = if view.bucket == SizeBucket::Large {
        48
    } else {
        32
    };
    let value = primary_value(
        &view.data.hashrate_stats,
        |stats| {
            stats.hashprice_per_th_day.map(|value| {
                fmt!(
                    "{} {}/PH/Day",
                    parts::compact_number(value * TERAHASHES_PER_PETAHASH, 2),
                    "USD"
                )
            })
        },
        value_size,
    );
    col(
        props!(
            background: TRANSPARENT,
            padding: 16.0,
            gap: if view.bucket == SizeBucket::Small { 6.0 } else { 24.0 },
            flex: 1.0
        ),
        [
            parts::title(&icons::CHART, WHITE, "Hash Price", None),
            value,
        ],
    )
}

fn price_panel(view: &ViewData, chart: Option<(f32, f32)>) -> Node {
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
            ),
            primary_value(
                &view.data.price_stats,
                |stats| stats.price.map(|price| parts::money(price, 0)),
                32,
            ),
        ],
    );
    let mut children = vec![summary];
    if let Some((width, height)) = chart {
        children.push(chart_or_status(
            &view.data.day_history,
            |history| &history.price,
            width,
            height,
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
        FULL_CHART_WIDTH,
        98.0,
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
            ),
            primary_value(
                &view.data.hashrate_stats,
                |stats| {
                    stats.current_ehs.map(|ehs| {
                        Hashrate::from_terahashes_per_second(ehs * TERAHASHES_PER_EXAHASH)
                            .format_si(4)
                    })
                },
                32,
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
                [difficulty_panel(view, Some((287.0, 42.0)), true)],
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
                [difficulty_panel(view, Some((606.0, 76.0)), true)],
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
                            Some((FULL_CHART_WIDTH, 76.0)),
                            true,
                        )])],
                    ),
                    parts::bordered([hashprice_panel(view)]),
                ],
            ),
            col(props!(flex: 1.0), [network_stats(view)]),
            col(
                props!(width: FULL_COLUMN_WIDTH, gap: parts::GAP),
                [
                    parts::bordered([price_panel(view, Some((FULL_CHART_WIDTH, 98.0)))]),
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

    #[test]
    fn loading_value_keeps_muted_presentation() {
        let loading: Availability<()> = Availability::Unavailable;

        assert_eq!(
            availability_value(&loading, |_| None),
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
}
