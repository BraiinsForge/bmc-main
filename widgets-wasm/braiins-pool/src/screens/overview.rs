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

//! The Overview (Data) screen: one thin layout per design size, composing
//! the shared fragments.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::model::{PoolData, SizeBucket};
use crate::screens::parts::{self, color, font, space};
use crate::screens::plot::{self, ChartSpec};

/// Everything the Overview screen shows; the wasm side fills it from live
/// state, the storybook from fixtures.
#[derive(Clone, Debug)]
pub struct OverviewViewData {
    pub bucket: SizeBucket,
    /// The viewport in pixels: canvas draw lists bake absolute coordinates,
    /// so chart dimensions are computed from it at build time.
    pub width: f32,
    pub height: f32,
    /// The bound account's operator-given name; `None` renders the
    /// missing-account placeholder state.
    pub account: Option<String>,
    /// Where the placeholder state sends the operator to bind one.
    pub bind_hint: parts::BindHint,
    pub worker_states: bool,
    pub data: PoolData,
}

/// Glyph counts the loading bars stand in for, one per slot's own strings.
mod chars {
    /// "500,0"
    pub const HASHRATE: f32 = 5.0;
    /// "0,170468 BTC"
    pub const REWARD: f32 = 12.0;
    /// "Last payout: 0,000380 BTC"
    pub const LAST_PAYOUT: f32 = 18.0;
}

/// The fixed blocks in the frames' budgets; whatever is left over goes
/// to the chart, which needs a definite size to bake its draw list.
mod budget {
    /// The header line: with the frame padding and gap it puts content at
    /// y = 64, as designed.
    pub const HEADER: f32 = 40.0;
    /// The Fullscreen chart section and the workers card beside it.
    pub const FULL_CHART: f32 = 214.0;
    pub const FULL_WORKERS_W: f32 = 193.0;
    /// The Large frame's payout card (two body lines and the meter, with
    /// pads and gaps) and its tiles row (label/value/sub with theirs).
    pub const L_PAYOUT: f32 = 104.0;
    pub const L_TILES: f32 = 124.0;
}

const SPARKLINE: ChartSpec = ChartSpec {
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

/// The Overview screen for one widget viewport.
#[must_use]
pub fn overview_view(view: &OverviewViewData) -> Node {
    if view.account.is_none() {
        return frame(vec![
            header(None),
            parts::unbound_body(view.bucket, &view.bind_hint),
        ]);
    }
    if view.data.access_denied {
        // The account joins the header where the normal layouts show it.
        let account = match view.bucket {
            SizeBucket::Small => None,
            SizeBucket::Medium | SizeBucket::Large | SizeBucket::Full => view.account.as_deref(),
        };
        return frame(vec![header(account), parts::denied_body(view.bucket)]);
    }
    match view.bucket {
        SizeBucket::Small => small(view),
        SizeBucket::Medium => medium(view),
        SizeBucket::Large => large(view),
        SizeBucket::Full => full(view),
    }
}

fn frame(children: Vec<Node>) -> Node {
    col(
        props!(padding: space::PADDING, gap: space::GAP, background: color::BG, flex: 1.0),
        children,
    )
}

fn header(account: Option<&str>) -> Node {
    row(
        props!(height: budget::HEADER, cross_align: CrossAlign::Center),
        [parts::header_left(account)],
    )
}

/// A single centered hashrate hero, no card.
fn small(view: &OverviewViewData) -> Node {
    let (value, label) = hashrate_strings(&view.data);
    let hero = match value.as_deref() {
        Some(value) => parts::hero_value(value),
        None => parts::skeleton_value(chars::HASHRATE, font::HERO),
    };
    frame(vec![
        header(None),
        center(
            props!(flex: 1.0),
            [col(
                props!(gap: 12.0, cross_align: CrossAlign::Center),
                [parts::label(&label), hero, parts::label("5m Average")],
            )],
        ),
    ])
}

/// Borderless stat blocks beside a compact workers card; the card centers
/// on the whole frame height, so the columns split before the header.
fn medium(view: &OverviewViewData) -> Node {
    let (hr_value, hr_label) = hashrate_strings(&view.data);
    let left = col(
        props!(gap: space::GAP, flex: 1.0),
        vec![
            header(view.account.as_deref()),
            // Center the stats in the space under the header instead of
            // packing them against it.
            col(
                props!(flex: 1.0, justify_content: Justify::Center),
                [row(
                    props!(gap: 32.0),
                    [
                        parts::stat_block(
                            None,
                            &hr_label,
                            hr_value.as_deref(),
                            Some("5m Average"),
                            parts::STAT_OPEN,
                            chars::HASHRATE,
                        ),
                        reward_block(&view.data, parts::STAT_OPEN),
                    ],
                )],
            ),
        ],
    );
    let mut cells = vec![left];
    if view.worker_states {
        cells.push(col(
            props!(justify_content: Justify::Center),
            [parts::card(
                parts::CARD_M,
                parts::workers_panel(&view.data.workers, &parts::WORKERS_COMPACT),
            )],
        ));
    }
    col(
        props!(padding: space::PADDING, background: color::BG, flex: 1.0),
        [row(props!(gap: space::PADDING, flex: 1.0), cells)],
    )
}

/// Full-width payout card, two stat cards, and a bare sparkline.
fn large(view: &OverviewViewData) -> Node {
    let (hr_value, hr_label) = hashrate_strings(&view.data);
    let content_w = view.width - 2.0 * space::PADDING;
    let content_h = view.height - 2.0 * space::PADDING - budget::HEADER - space::GAP;
    let spark_h = content_h - budget::L_PAYOUT - budget::L_TILES - 2.0 * space::GAP;
    let mut rows = vec![
        header(view.account.as_deref()),
        // Natural height, unlike the fill-to-row tiles below it.
        parts::card(
            parts::CardSpec {
                fill: false,
                ..parts::CARD_L
            },
            payout_body(&view.data, parts::STAT_TIGHT),
        ),
        row(
            props!(gap: space::GAP, height: budget::L_TILES, cross_align: CrossAlign::Stretch),
            [
                col(
                    props!(flex: 1.0),
                    [parts::card(
                        parts::CARD_L,
                        parts::stat_block(
                            Some(color::HASHRATE),
                            &hr_label,
                            hr_value.as_deref(),
                            Some("5m Average"),
                            parts::STAT_TIGHT,
                            chars::HASHRATE,
                        ),
                    )],
                ),
                col(
                    props!(flex: 1.0),
                    [parts::card(
                        parts::CARD_L,
                        reward_block(&view.data, parts::STAT_TIGHT),
                    )],
                ),
            ],
        ),
    ];
    rows.push(match view.data.hashrate_history.as_option() {
        Some(history) => plot::line_chart(history, None, content_w, spark_h, &SPARKLINE, &[], &[]),
        None => parts::skeleton_block(content_w, spark_h),
    });
    frame(rows)
}

/// Carded tiles row, the chart section, and the roomy workers card.
fn full(view: &OverviewViewData) -> Node {
    let (hr_value, hr_label) = hashrate_strings(&view.data);
    let content_w = view.width - 2.0 * space::PADDING;
    let content_h = view.height - 2.0 * space::PADDING - budget::HEADER - space::GAP;
    let main_w = if view.worker_states {
        content_w - budget::FULL_WORKERS_W - space::GAP
    } else {
        content_w
    };
    let tiles_h = content_h - budget::FULL_CHART - space::GAP;

    let tiles = row(
        props!(gap: space::GAP, height: tiles_h, cross_align: CrossAlign::Stretch),
        [
            col(
                props!(flex: 1.0),
                [parts::card(
                    parts::CARD_FULL_TILE,
                    parts::stat_block(
                        Some(color::HASHRATE),
                        &hr_label,
                        hr_value.as_deref(),
                        Some("5m Average"),
                        parts::STAT_ROOMY,
                        chars::HASHRATE,
                    ),
                )],
            ),
            col(
                props!(width: 440.0),
                [parts::card(
                    parts::CARD_FULL_TILE,
                    payout_body(&view.data, parts::STAT_ROOMY),
                )],
            ),
            col(
                props!(flex: 1.0),
                [parts::card(
                    parts::CARD_FULL_TILE,
                    reward_block(&view.data, parts::STAT_ROOMY),
                )],
            ),
        ],
    );

    let mut main = vec![tiles];
    if let Some(history) = view.data.hashrate_history.as_option() {
        let workers_history = view
            .worker_states
            .then(|| view.data.workers_history.as_option())
            .flatten();
        let spec = ChartSpec {
            left_gutter: 60.0,
            right_gutter: 72.0,
            hashrate_ticks: true,
            workers_ticks: workers_history.is_some(),
            x_band: None,
            solid_baseline: true,
            grid_steps: 3,
            tick_font: font::TICK,
            marker_size: None,
        };
        main.push(plot::line_chart(
            history,
            workers_history,
            main_w,
            budget::FULL_CHART,
            &spec,
            &[],
            &[],
        ));
    } else {
        main.push(parts::skeleton_block(main_w, budget::FULL_CHART));
    }

    let main_col = col(
        props!(gap: space::GAP, width: main_w, height: content_h),
        main,
    );
    let mut cells = vec![main_col];
    if view.worker_states {
        cells.push(col(
            props!(width: budget::FULL_WORKERS_W),
            [parts::card(
                parts::CARD_FULL_WORKERS,
                parts::workers_panel(&view.data.workers, &parts::WORKERS_ROOMY),
            )],
        ));
    }
    frame(vec![
        header(view.account.as_deref()),
        row(props!(gap: space::GAP, height: content_h), cells),
    ])
}

/// The hashrate hero as (value, label): the label names the SI unit the
/// value is scaled to, so the pair never drifts apart. A `None` value
/// renders as a skeleton, with the unit-less label above it.
fn hashrate_strings(data: &PoolData) -> (Option<String>, String) {
    match data.hashrate_5m.as_option() {
        Some(hashrate) => {
            let (value, unit) = hashrate.format_si_parts(4);
            (Some(value), fmt!("Hashrate ({unit})"))
        }
        None => (None, "Hashrate".to_owned()),
    }
}

fn reward_block(data: &PoolData, gaps: parts::StatGaps) -> Node {
    match data.rewards.as_option() {
        Some(rewards) => {
            let btc = format_number!(rewards.today_btc, 6);
            let usd = format_number!(rewards.today_usd, 2);
            parts::stat_block(
                None,
                "Todays Reward",
                Some(&fmt!("{btc} BTC")),
                Some(&fmt!("≈ {usd} USD")),
                gaps,
                chars::REWARD,
            )
        }
        None => parts::stat_block(None, "Todays Reward", None, None, gaps, chars::REWARD),
    }
}

/// "Next Payout in ~" with a self-updating remaining time, the progress
/// meter, and the last completed payout's amount.
fn payout_body(data: &PoolData, gaps: parts::StatGaps) -> Node {
    let payout = data.next_payout.as_option();
    let title = match payout.and_then(|p| p.estimate_at) {
        // The reltime label carries its own "in" prefix (RemainingOnly).
        Some(estimate_at) => row(
            props!(cross_align: CrossAlign::End),
            [
                parts::label("Next Payout "),
                relative_time_live(
                    SystemTime {
                        unix_secs: estimate_at,
                    },
                    RelTimeFormat {
                        length: RelTimeLength::Long,
                        segments: RelTimeSegments::Single,
                    },
                    RelTimeClamp::RemainingOnly,
                    style!(size: font::BODY, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0)
                        .into(),
                ),
            ],
        ),
        None => parts::label("Next Payout"),
    };
    // Every line keeps its slot while loading, so the card never reflows
    // as the sources land one by one. A skeleton stands for a source that
    // has not answered yet; one that answered empty says so outright —
    // with the title gap folded into its slot [`parts::absent_meter`],
    // so the stack gets that gap zeroed to keep the footer in place.
    let (meter, gaps) = match payout {
        Some(p) => match p.progress_pct {
            Some(pct) => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a 0–100 percentage is exact in f32"
                )]
                let fraction = (pct / 100.0).clamp(0.0, 1.0) as f32;
                (parts::meter_row(fraction), gaps)
            }
            None => (
                parts::absent_meter("No payout scheduled", gaps),
                parts::StatGaps {
                    label_value: 0.0,
                    ..gaps
                },
            ),
        },
        None => (parts::skeleton_meter(), gaps),
    };
    let last_line = match data.payouts.as_option() {
        Some(payouts) => match payouts.last() {
            Some(payout) => {
                let amount = format_number!(payout.amount_btc, 6);
                parts::text_run(vec![
                    span("Last payout: ", ()),
                    span(
                        fmt!("{amount} BTC"),
                        style!(weight: FontWeight::SEMIBOLD, color: color::TEXT),
                    ),
                ])
            }
            None => parts::absent("No payouts yet"),
        },
        None => parts::skeleton(chars::LAST_PAYOUT),
    };
    parts::stat_stack(title, meter, last_line, gaps)
}
