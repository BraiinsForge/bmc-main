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

use super::{
    BACKGROUND, TITLE, block, change_color, fixed_height, icons, info_overload_bottom_row,
    info_overload_difficulty_row, info_overload_primary_row, price_chart, space_between_rows,
    with_horizontal_padding,
};
use crate::format;
use crate::layout::{self, Panel};
use crate::model::{MinerData, PublicData};

const EDGE: f32 = 16.0;
const TITLE_ICON_SIZE: f32 = 16.0;
const TITLE_GAP: f32 = 8.0;
const TITLE_SIZE: u32 = 14;
const TITLE_TO_BAND: f32 = 12.0;
const BAND_TO_RULE: f32 = 8.0;
const RULE: f32 = 1.0;
const RULE_COLOR: Color = GRAY_90;

const CHART_WIDTH: f32 = 120.0;
const CHART_HEIGHT: f32 = 44.0;
const PRICE_SIZE: u32 = 32;
const PRICE_SYMBOL_SIZE: u32 = 20;

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

#[cfg(test)]
mod tests {
    use super::*;
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

    /// The frame: the title, the price band, then the nine figures.
    #[test]
    fn the_info_overload_face_reads_top_down() {
        assets::init_test_registrars();
        let texts = texts(&info_overload(
            &miner(Reported::All, Some(1.02)),
            &public(Reported::All, PriceMove::Down),
        ));
        assert_eq!(
            texts[..4],
            [
                "Miner Info - Info Overload",
                "Bitcoin (24h)",
                "-4,80%",
                "$ 101\u{a0}754"
            ]
        );
        for label in [
            "Hashrate",
            "Power Consump.",
            "Block Height",
            "Est. Diff. Adjust.",
            "Prev. Diff. Adjust.",
            "Epoch Progress",
            "Miner Uptime",
            "Fees (144 Blocks)",
            "Hashvalue",
        ] {
            assert!(texts.contains(&label.to_owned()), "{label}: {texts:?}");
        }
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
