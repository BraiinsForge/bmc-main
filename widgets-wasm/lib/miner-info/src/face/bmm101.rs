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
    BACKGROUND, EDGE, block, change_color, fixed_height, icons, info_overload_bottom_row,
    info_overload_difficulty_row, info_overload_primary_row, price_chart, rule, space_between_rows,
    text_line, title_row, titled_lines, with_horizontal_padding,
};
use crate::format;
use crate::layout::{self, Panel};
use crate::model::{MinerData, PublicData};

const TITLE_TO_BAND: f32 = 12.0;
const BAND_TO_RULE: f32 = 8.0;

const CHART_WIDTH: f32 = 120.0;
const CHART_HEIGHT: f32 = 44.0;
const PRICE_SIZE: u32 = 32;
const PRICE_SYMBOL_SIZE: u32 = 20;

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

/// The frame's six lines: the miner's readings with the BTC price among them.
#[must_use]
pub fn mining(miner: &MinerData, public: &PublicData) -> Node {
    let sizes = layout::list_layout(Panel::Bmm101).text;
    titled_lines(
        Panel::Bmm101,
        &icons::MINING,
        "Miner Info - Mining",
        vec![
            text_line("Current Hashrate", format::fixed(miner.hashrate, 2), sizes),
            text_line("Miner Uptime", format::uptime(miner.uptime), sizes),
            text_line("BTC Price", format::money(public.btc_price, 0), sizes),
            text_line("Power Consumption", format::fixed(miner.power, 0), sizes),
            text_line("Block Counter", format::integer(miner.found_blocks), sizes),
            text_line(
                "IP Address",
                miner
                    .ip_address
                    .as_option()
                    .cloned()
                    .unwrap_or_else(format::unavailable)
                    .into(),
                sizes,
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::face::test_support::texts;
    use crate::fixtures::{PriceMove, Reported, miner, public};

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
