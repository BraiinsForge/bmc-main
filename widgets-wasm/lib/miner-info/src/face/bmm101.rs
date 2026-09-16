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

//! The BMM101 (480×320) faces, laid out from the Figma frames at their
//! own size rather than scaled from another panel's.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use bmc_wasm_sdk::types::Ratio;

use super::round::{self, GaugeType};
use super::{
    BACKGROUND, RenderSize, TITLE, VALUE, block, change_color, fixed_height, fixed_width, icons,
    info_overload_bottom_row, info_overload_difficulty_row, info_overload_primary_row, price_chart,
    space_between_rows, text_line, unit_visible, value_with_unit, with_horizontal_padding,
};
use crate::format;
use crate::layout::{self, Panel};
use crate::model::{Availability, MinerData, PublicData};

const EDGE: f32 = 16.0;
const TITLE_ICON_SIZE: f32 = 16.0;
const TITLE_GAP: f32 = 8.0;
const TITLE_SIZE: u32 = 14;
const TITLE_TO_BAND: f32 = 12.0;
const TITLE_TO_ROWS: f32 = 16.0;
const BAND_TO_RULE: f32 = 8.0;
const RULE: f32 = 1.0;
const RULE_COLOR: Color = GRAY_90;

const CHART_WIDTH: f32 = 120.0;
const CHART_HEIGHT: f32 = 44.0;
const PRICE_SIZE: u32 = 32;
const PRICE_SYMBOL_SIZE: u32 = 20;

const ETA_GAP: f32 = 16.0;
const ADJUSTMENT_DECIMALS: u32 = 1;
const TAG_PADDING: f32 = 2.0;
// The frame pads the tag 4 px across and 2 px down; `padding` is one number.
const TAG_INSET: f32 = 2.0;
const TAG_RADIUS: f32 = 4.0;
// Tint over black, as the ticker badges do, rather than the frame's own fills.
const TAG_BACKGROUND_ALPHA: f32 = 0.15;

/// The gauge disc: the round face at 0.6, which is what fits the height.
const GAUGE_SIDE: u32 = 288;
/// The round type at 0.6, rounded to whole pixels.
const GAUGE_TYPE: GaugeType = GaugeType {
    hashrate: 38,
    hashrate_unit: 14,
    status: 10,
    caption: Some(8),
    cluster_value: 19,
    cluster_label: 10,
    cluster_unit: 10,
    unit_slot_w: 36.0,
};

fn title_row(icon: &Svg, label: &str) -> Node {
    row(
        props!(cross_align: CrossAlign::Center, gap: TITLE_GAP),
        [
            canvas(
                props!(width: TITLE_ICON_SIZE, height: TITLE_ICON_SIZE),
                [Draw::svg_contain(icon, TITLE_ICON_SIZE, TRANSPARENT).with_anti_alias()],
            ),
            text(
                label,
                style!(size: TITLE_SIZE, weight: FontWeight::SEMIBOLD, color: TITLE),
            ),
        ],
    )
}

fn rule() -> Node {
    col(props!(height: RULE, background: RULE_COLOR), [])
}

// The currency symbol rides in the amount's paragraph at its own size,
// so the two share a baseline.
fn price(public: &PublicData) -> Node {
    let mut spans = Vec::with_capacity(2);
    if let Some(symbol) = format::money_symbol(public.btc_price) {
        spans.push(span(
            bmc_wasm_sdk::fmt!("{symbol} "),
            style!(size: PRICE_SYMBOL_SIZE),
        ));
    }
    spans.push(span(format::money_amount(public.btc_price, 0), ()));
    paragraph(
        style!(size: PRICE_SIZE, weight: FontWeight::BOLD, color: WHITE),
        spans,
    )
}

fn bitcoin_band(public: &PublicData, metrics: layout::BlockLayout) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            block(
                "Bitcoin (24h)",
                format::signed_percent_unit(public.btc_change_24h, 2).into(),
                metrics,
                change_color(public.btc_change_24h),
                FontWeight::BOLD,
            ),
            spacer(1.0),
            price_chart(&public.btc_price_history, CHART_WIDTH, CHART_HEIGHT),
            spacer(1.0),
            price(public),
        ],
    )
}

fn info_overload_grid(miner: &MinerData, public: &PublicData) -> Node {
    let fields = layout::info_overload_fields(Panel::Bmm101);
    let metrics = layout::info_overload_layout(Panel::Bmm101);
    let rows = vec![
        info_overload_primary_row(miner, public, fields, metrics),
        info_overload_difficulty_row(public, metrics),
        info_overload_bottom_row(miner, public, fields, metrics),
    ];
    space_between_rows(rows, metrics)
}

#[must_use]
pub fn info_overload(miner: &MinerData, public: &PublicData) -> Node {
    let metrics = layout::info_overload_layout(Panel::Bmm101);
    col(
        props!(background: BACKGROUND),
        [
            fixed_height(EDGE),
            with_horizontal_padding(
                title_row(&icons::INFO_OVERLOAD, "Miner Info - Info Overload"),
                EDGE,
            ),
            fixed_height(TITLE_TO_BAND),
            with_horizontal_padding(bitcoin_band(public, metrics), EDGE),
            fixed_height(BAND_TO_RULE),
            with_horizontal_padding(rule(), EDGE),
            info_overload_grid(miner, public),
        ],
    )
}

fn label(name: &'static str, sizes: layout::TextSizes) -> Node {
    text(
        name,
        style!(size: sizes.title, weight: FontWeight::SEMIBOLD, color: TITLE, flex: 1.0),
    )
}

// The retarget ETA in the label's voice, the progress in the value's.
// An unknown ETA is left out, so the line reads one `N/A`, not two.
fn epoch_line(public: &PublicData, sizes: layout::TextSizes) -> Node {
    let mut parts = vec![label("Epoch Progress", sizes)];
    if let Availability::Available(remaining) = public.epoch_remaining {
        parts.push(text(
            format::epoch_eta(remaining),
            style!(size: sizes.value, weight: FontWeight::REGULAR, color: TITLE),
        ));
        parts.push(fixed_width(ETA_GAP));
    }
    parts.push(value_with_unit(
        format::fixed(public.epoch_progress, 0),
        sizes,
        TextAlign::Right,
        VALUE,
        FontWeight::REGULAR,
    ));
    row(props!(cross_align: CrossAlign::Center), parts)
}

// A signed change in a tinted pill; a placeholder stays plain, like an affix.
fn adjustment_line(
    name: &'static str,
    change: Availability<Ratio>,
    sizes: layout::TextSizes,
) -> Node {
    let rendered = format::signed_percent_unit(change, ADJUSTMENT_DECIMALS);
    let known = unit_visible(&rendered);
    let color = change_color(change);
    let value = text(
        rendered,
        style!(size: sizes.value, weight: FontWeight::BOLD, color: color),
    );
    let trailing = if known {
        row(
            props!(
                background: color.with_alpha(TAG_BACKGROUND_ALPHA),
                border_radius: TAG_RADIUS,
                padding: TAG_PADDING
            ),
            [fixed_width(TAG_INSET), value, fixed_width(TAG_INSET)],
        )
    } else {
        value
    };
    row(
        props!(cross_align: CrossAlign::Center),
        [label(name, sizes), trailing],
    )
}

/// The round gauge as a disc centred on the panel, without the chip header
/// and with the Geek quadrants, BTC price included.
#[must_use]
pub fn mining(miner: &MinerData, public: &PublicData, seed_gauge: bool) -> Node {
    let g = round::seeded_gauge(miner, seed_gauge);
    let disc = round::gauge_screen(
        RenderSize {
            width: GAUGE_SIDE,
            height: GAUGE_SIDE,
        },
        &g,
        miner.hashrate,
        None,
        &round::geek_clusters(miner, public),
        GAUGE_TYPE,
    );
    col(
        props!(background: BACKGROUND),
        [
            spacer(1.0),
            row(props!(), [spacer(1.0), disc, spacer(1.0)]),
            spacer(1.0),
        ],
    )
}

#[must_use]
pub fn geek(public: &PublicData) -> Node {
    let sizes = layout::mining_layout(Panel::Bmm101).text;
    let lines = [
        text_line(
            "Network HR",
            format::network_hashrate(public.network_hashrate),
            sizes,
        ),
        text_line(
            "Block Height",
            format::public_integer(public.block_height),
            sizes,
        ),
        epoch_line(public, sizes),
        adjustment_line("Diff. Adjustment", public.prev_diff_adjust, sizes),
        adjustment_line("Est. Diff. Adjustment", public.est_diff_adjust, sizes),
        text_line(
            "Fees (144 Blocks)",
            format::fees(public.avg_fees_per_block, public.avg_fee_share),
            sizes,
        ),
        text_line(
            "Hashvalue",
            format::fixed_strip_zero_fraction(public.hashvalue, 2),
            sizes,
        ),
    ];

    // Lines and rules spread over the height, the frame's `justify-between`.
    let mut rows = Vec::with_capacity(lines.len() * 4);
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            rows.push(spacer(1.0));
            rows.push(with_horizontal_padding(rule(), EDGE));
            rows.push(spacer(1.0));
        }
        rows.push(with_horizontal_padding(line, EDGE));
    }

    col(
        props!(background: BACKGROUND),
        [
            fixed_height(EDGE),
            with_horizontal_padding(title_row(&icons::GEEK, "Miner Info - Geek"), EDGE),
            fixed_height(TITLE_TO_ROWS),
            col(props!(flex: 1.0), rows),
            fixed_height(EDGE),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::typography::NBSP;

    use crate::fixtures::{PriceMove, Reported, miner, public};

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

    /// The frame: the title, then the seven network figures in order.
    #[test]
    fn the_geek_face_lists_the_network_top_down() {
        assets::init_test_registrars();
        let texts = texts(&geek(&public(Reported::All, PriceMove::Up)));
        assert_eq!(
            texts,
            [
                "Miner Info - Geek",
                "Network HR",
                "650  EH/s",
                "Block Height",
                format!("880{NBSP}123").as_str(),
                "Epoch Progress",
                "in ~ 2 days",
                "87  %",
                "Diff. Adjustment",
                "-2,1%",
                "Est. Diff. Adjustment",
                "-4,5%",
                "Fees (144 Blocks)",
                "~ 0,055 BTC | 12,1%",
                "Hashvalue",
                "70  SAT/TH/Day",
            ]
        );
    }

    fn mining_face() -> Node {
        mining(
            &miner(Reported::All, Some(1.02)),
            &public(Reported::All, PriceMove::Up),
            false,
        )
    }

    /// The disc sits between spacers on both axes, at the round face's 0.6.
    #[test]
    fn the_mining_face_centres_the_gauge_disc() {
        let Node::Column(_, rows) = mining_face() else {
            panic!("BUG: the face is a column");
        };
        let Node::Row(_, cells) = &rows[1] else {
            panic!("BUG: the disc row sits between the spacers");
        };
        let Node::Column(_, disc) = &cells[1] else {
            panic!("BUG: the disc sits between the spacers");
        };
        let Node::Canvas { props, .. } = &disc[0] else {
            panic!("BUG: the disc draws on a canvas");
        };
        assert_eq!((props.width, props.height), (288.0, 288.0));
    }

    /// The fixture miner reports its chip, which the round face would name.
    #[test]
    fn the_mining_face_draws_no_chip_header() {
        let texts = texts(&mining_face());
        assert!(
            !texts.contains(&"BM1370".to_owned()),
            "no chip header: {texts:?}"
        );
    }

    #[test]
    fn an_unknown_network_reads_one_placeholder_per_line() {
        assets::init_test_registrars();
        let texts = texts(&geek(&public(Reported::Nothing, PriceMove::Up)));
        let placeholders = texts.iter().filter(|text| *text == "N/A").count();
        assert_eq!(
            placeholders, 7,
            "one N/A per line, the epoch ETA left out: {texts:?}"
        );
    }

    #[test]
    fn a_known_adjustment_wears_a_pill_and_a_placeholder_does_not() {
        let sizes = layout::mining_layout(Panel::Bmm101).text;
        let known = adjustment_line(
            "Diff. Adjustment",
            Availability::Available(Ratio::from_percent(-4.5)),
            sizes,
        );
        let Node::Row(_, children) = known else {
            panic!("BUG: a line is a row");
        };
        assert!(
            matches!(&children[1], Node::Row(props, _) if props.border_radius > 0.0),
            "the value sits in a rounded pill"
        );

        let placeholder = adjustment_line("Diff. Adjustment", Availability::Unavailable, sizes);
        let Node::Row(_, children) = placeholder else {
            panic!("BUG: a line is a row");
        };
        assert!(
            matches!(&children[1], Node::Paragraph { .. }),
            "N/A stays plain text"
        );
    }

    #[test]
    fn an_unknown_price_wears_no_symbol() {
        let unknown = texts(&price(&public(Reported::Nothing, PriceMove::Up)));
        assert_eq!(unknown, [format::NOT_AVAILABLE]);
    }

    /// The rows spread by flex spacers, so the grid takes whatever height
    /// the title row's line box leaves it.
    #[test]
    fn the_grid_spreads_its_three_rows() {
        let Node::Column(props, grid) = info_overload_grid(
            &miner(Reported::All, Some(1.02)),
            &public(Reported::All, PriceMove::Up),
        ) else {
            panic!("BUG: the grid is a column");
        };
        assert!((props.flex - 1.0).abs() < f32::EPSILON, "flexed to fill");
        let spacers = grid
            .iter()
            .filter(|node| matches!(node, Node::Spacer { .. }))
            .count();
        assert_eq!(spacers, 2, "two spacers between three rows");
    }
}
