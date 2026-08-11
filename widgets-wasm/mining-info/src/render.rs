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
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::chart;
use crate::format;
use crate::layout::{self, Viewport};
use crate::model::{Availability, MinerData, PublicData};
use units::units::Quantity;

pub(crate) mod round;

const TITLE: Color = GRAY_50;
const UNIT: Color = GRAY_50;
const VALUE: Color = WHITE;
const BACKGROUND: Color = BLACK;

// Storybook BTC sparkline palette: green when the 1d series ends above where it
// started, red otherwise. The area gradient fades from a tinted top down to
// fully transparent; the falling tint is stronger to read on a dark header.
const CHART_UP: Color = Color::from_rgb(0x34, 0xC0, 0x6A);
const CHART_DOWN: Color = Color::from_rgb(0xF9, 0x53, 0x55);
const CHART_UP_TOP_ALPHA: f32 = 0.16;
const CHART_DOWN_TOP_ALPHA: f32 = 0.30;
const CHART_STROKE: f32 = 2.0;
const CHART_INSET: f32 = 2.0;

#[derive(Clone, Copy)]
pub(crate) struct RenderSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

fn viewport(size: RenderSize) -> Viewport {
    Viewport {
        width: size.width,
        height: size.height,
    }
}

fn fixed_width(width: f32) -> Node {
    col(props!(width: width), Vec::<Node>::new())
}

fn fixed_height(height: f32) -> Node {
    col(props!(height: height), Vec::<Node>::new())
}

fn with_horizontal_padding(node: Node, padding: f32) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            fixed_width(padding),
            col(props!(flex: 1.0), [node]),
            fixed_width(padding),
        ],
    )
}

fn unit_visible(value: &str) -> bool {
    value != format::NOT_AVAILABLE
}

fn value_with_unit(
    value: format::Rendered,
    size: u32,
    align: TextAlign,
    value_color: Color,
    weight: FontWeight,
) -> Node {
    let show_unit = unit_visible(&value.value);
    let mut spans = vec![span(value.value, style!(color: value_color))];
    if let Some(unit) = value.unit
        && show_unit
    {
        spans.push(span(bmc_wasm_sdk::fmt!("  {unit}"), style!(color: UNIT)));
    }
    paragraph(
        style!(size: size, weight: weight, color: value_color, align: align),
        spans,
    )
}

fn text_line(name: &'static str, value: format::Rendered, sizes: layout::TextSizes) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            text(
                name,
                style!(size: sizes.title, weight: FontWeight::SEMIBOLD, color: TITLE, flex: 1.0),
            ),
            value_with_unit(
                value,
                sizes.value,
                TextAlign::Right,
                VALUE,
                FontWeight::REGULAR,
            ),
        ],
    )
}

// Mirrors BOSer's `VerticalLayout { alignment: space-between }`: flex spacers
// between rows distribute the rows across the full viewport height instead of
// packing them at the top. The SDK has no main-axis justify.
fn vertical_lines(size: RenderSize, lines: Vec<Node>) -> Node {
    let metrics = layout::mining_layout(layout::classify(viewport(size)));
    let mut children = Vec::with_capacity(lines.len().saturating_mul(2));
    children.push(fixed_height(metrics.padding_top));
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            children.push(spacer(1.0));
        }
        children.push(with_horizontal_padding(line, metrics.padding_horizontal));
    }
    children.push(fixed_height(metrics.padding_bottom));
    col(
        props!(
            background: BACKGROUND
        ),
        children,
    )
}

pub(crate) fn mining(size: RenderSize, miner: &MinerData) -> Node {
    let sizes = layout::mining_layout(layout::classify(viewport(size))).text;
    vertical_lines(
        size,
        vec![
            text_line(
                "Current Hashrate",
                format::fixed(miner.hashrate_ths, 2),
                sizes,
            ),
            text_line("Temperature", format::temperature(miner.temperature), sizes),
            text_line("Power Consumption", format::fixed(miner.power_w, 0), sizes),
            text_line("MCR", format::fixed(miner.mcr_percent, 1), sizes),
            text_line("Fan Speed", format::fixed(miner.fan_percent, 0), sizes),
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

pub(crate) fn geek(size: RenderSize, miner: &MinerData, public: &PublicData) -> Node {
    let sizes = layout::mining_layout(layout::classify(viewport(size))).text;
    vertical_lines(
        size,
        vec![
            text_line(
                "Current Hashrate",
                format::fixed(miner.hashrate_ths, 2),
                sizes,
            ),
            text_line("Temperature", format::temperature(miner.temperature), sizes),
            text_line("Power Consumption", format::fixed(miner.power_w, 0), sizes),
            text_line("Miner Uptime", format::uptime(miner.uptime_s), sizes),
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
            text_line("BTC Price", format::money(public.btc_price, 0), sizes),
        ],
    )
}

const BTC_PRICE_SIZE: u32 = 28;

fn block(
    name: &'static str,
    value: format::Rendered,
    metrics: layout::BlockLayout,
    value_color: Color,
    value_weight: FontWeight,
) -> Node {
    col(
        props!(width: metrics.block_width, height: metrics.block_height),
        [
            text(
                name,
                style!(size: metrics.text.title, weight: FontWeight::SEMIBOLD, color: TITLE),
            ),
            value_with_unit(
                value,
                metrics.text.value,
                TextAlign::Left,
                value_color,
                value_weight,
            ),
        ],
    )
}

fn text_block(name: &'static str, value: format::Rendered, metrics: layout::BlockLayout) -> Node {
    block(name, value, metrics, VALUE, FontWeight::REGULAR)
}

// A block with no fixed cell: the col shrinks to its content so the value stays
// on one line (an unconstrained paragraph measures single-line). The title is
// left-aligned with the value; the block as a whole is centered between row
// spacers by the round faces, which group blocks rather than aligning a grid.
fn centered_block(name: &'static str, value: format::Rendered, sizes: layout::TextSizes) -> Node {
    col(
        props!(),
        [
            text(
                name,
                style!(size: sizes.title, weight: FontWeight::SEMIBOLD, color: TITLE),
            ),
            value_with_unit(
                value,
                sizes.value,
                TextAlign::Left,
                VALUE,
                FontWeight::REGULAR,
            ),
        ],
    )
}

// Mirrors BOSer's `extra_info`: the primary value and its extra value share one
// row separated by " | " (see boser-assets text_block.slint).
fn text_block_with_extra(
    name: &'static str,
    value: format::Rendered,
    extra_value: format::Rendered,
    metrics: layout::BlockLayout,
) -> Node {
    let show_unit = unit_visible(&value.value);
    let show_extra_unit = unit_visible(&extra_value.value);
    let mut spans = vec![span(value.value, style!(color: VALUE))];
    if let Some(unit) = value.unit
        && show_unit
    {
        spans.push(span(bmc_wasm_sdk::fmt!("  {unit}"), style!(color: UNIT)));
    }
    spans.push(span(" | ", style!(color: UNIT)));
    spans.push(span(" ", style!(color: VALUE)));
    spans.push(span(extra_value.value, style!(color: VALUE)));
    if let Some(extra_unit) = extra_value.unit
        && show_extra_unit
    {
        spans.push(span(
            bmc_wasm_sdk::fmt!("  {extra_unit}"),
            style!(color: UNIT),
        ));
    }
    col(
        props!(width: metrics.block_width, height: metrics.block_height),
        [
            text(
                name,
                style!(size: metrics.text.title, weight: FontWeight::SEMIBOLD, color: TITLE),
            ),
            paragraph(
                style!(
                    size: metrics.text.value,
                    weight: FontWeight::REGULAR,
                    color: VALUE
                ),
                spans,
            ),
        ],
    )
}

fn block_row(blocks: Vec<Node>, metrics: layout::BlockLayout) -> Node {
    with_horizontal_padding(
        row(props!(gap: metrics.horizontal_gap), blocks),
        metrics.padding_horizontal,
    )
}

fn space_between_rows(rows: Vec<Node>, metrics: layout::BlockLayout) -> Node {
    let mut children = Vec::with_capacity(rows.len().saturating_mul(2) + 2);
    children.push(fixed_height(metrics.padding_top));
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            if metrics.vertical_gap > 0.0 {
                children.push(fixed_height(metrics.vertical_gap));
            } else {
                children.push(spacer(1.0));
            }
        }
        children.push(row);
    }
    children.push(fixed_height(metrics.padding_bottom));
    col(props!(background: BACKGROUND), children)
}

pub(crate) fn network(size: RenderSize, public: &PublicData) -> Node {
    let class = layout::classify(viewport(size));
    let fields = layout::network_fields(class);
    let metrics = layout::network_layout(class);

    let mut rows = vec![block_row(
        vec![
            text_block(
                "Network HR",
                format::fixed(public.network_hashrate_ehs, 2),
                metrics,
            ),
            text_block(
                "Diff. Adjustment",
                format::signed_percent(public.prev_diff_adjust_percent, 2),
                metrics,
            ),
        ],
        metrics,
    )];
    if fields.show_extra_difficulty {
        rows.push(block_row(
            vec![
                text_block(
                    "Est. Diff. Adjustment",
                    format::signed_percent(public.est_diff_adjust_percent, 2),
                    metrics,
                ),
                text_block(
                    "Epoch Progress",
                    format::fixed(public.epoch_progress_percent, 0),
                    metrics,
                ),
            ],
            metrics,
        ));
    }
    let fee_block = if fields.show_fee_percent {
        text_block_with_extra(
            "Fees (144 Blocks)",
            format::approx_fixed(public.avg_fee_btc, 3),
            format::fixed(public.avg_fee_percent, 1),
            metrics,
        )
    } else {
        text_block(
            "Fees (144 Blocks)",
            format::approx_fixed(public.avg_fee_btc, 3),
            metrics,
        )
    };
    rows.push(block_row(
        vec![
            fee_block,
            text_block(
                "Block Height",
                format::public_integer(public.block_height),
                metrics,
            ),
        ],
        metrics,
    ));
    rows.push(block_row(
        vec![
            text_block(
                "Hashprice",
                format::money(public.hashprice, 3).with_unit("TH/Day"),
                metrics,
            ),
            text_block("BTC Price", format::money(public.btc_price, 0), metrics),
        ],
        metrics,
    ));

    space_between_rows(rows, metrics)
}

// The header sparkline. Returns an empty fixed-width column when the series has
// too few points to draw, so the price column keeps its grid slot whether or not
// the history has loaded.
fn price_chart(history: &[f64], metrics: layout::BlockLayout) -> Node {
    let width = metrics.block_width;
    let height = metrics.block_height;
    let line = chart::series_points(history, width, height, CHART_INSET);
    if line.len() < 2 {
        return fixed_width(width);
    }
    let (color, top_alpha) = if chart::is_rising(history) {
        (CHART_UP, CHART_UP_TOP_ALPHA)
    } else {
        (CHART_DOWN, CHART_DOWN_TOP_ALPHA)
    };
    let mut area = line.clone();
    area.push((width, height));
    area.push((0.0, height));
    canvas(
        props!(width: width, height: height),
        [
            fill!(area, linear: (color.with_alpha(top_alpha), color.with_alpha(0.0)), smooth),
            path!(line, stroke: CHART_STROKE, color: color, smooth),
        ],
    )
}

fn info_overload_header(
    public: &PublicData,
    show_price_graph: bool,
    metrics: layout::BlockLayout,
) -> Node {
    let change_color = match public.btc_change_24h_percent {
        Availability::Available(value) if value.raw() >= 0.0 => GREEN_50,
        Availability::Available(_) => RED_60,
        Availability::Unavailable | Availability::Failed => TITLE,
    };

    let mut blocks = vec![block(
        "Bitcoin (24h)",
        format::signed_percent_unit(public.btc_change_24h_percent, 2).into(),
        metrics,
        change_color,
        FontWeight::BOLD,
    )];
    if show_price_graph {
        blocks.push(price_chart(&public.btc_price_history, metrics));
    }
    blocks.push(text(
        format::money(public.btc_price, 0).value,
        style!(
            size: BTC_PRICE_SIZE,
            weight: FontWeight::BOLD,
            color: WHITE,
            width: metrics.block_width
        ),
    ));

    col(
        props!(background: GRAY_100),
        [
            fixed_height(18.0),
            block_row(blocks, metrics),
            fixed_height(18.0),
        ],
    )
}

fn info_overload_primary_row(
    miner: &MinerData,
    public: &PublicData,
    metrics: layout::BlockLayout,
) -> Node {
    block_row(
        vec![
            text_block("Hashrate", format::fixed(miner.hashrate_ths, 2), metrics),
            text_block("Power Consump.", format::fixed(miner.power_w, 0), metrics),
            text_block(
                "Block Height",
                format::public_integer(public.block_height),
                metrics,
            ),
        ],
        metrics,
    )
}

fn info_overload_difficulty_row(public: &PublicData, metrics: layout::BlockLayout) -> Node {
    block_row(
        vec![
            text_block(
                "Est. Diff. Adjust.",
                format::signed_percent(public.est_diff_adjust_percent, 2),
                metrics,
            ),
            text_block(
                "Prev. Diff. Adjust.",
                format::signed_percent(public.prev_diff_adjust_percent, 2),
                metrics,
            ),
            text_block(
                "Epoch Progress",
                format::fixed(public.epoch_progress_percent, 0),
                metrics,
            ),
        ],
        metrics,
    )
}

fn info_overload_bottom_row(
    miner: &MinerData,
    public: &PublicData,
    fields: layout::InfoOverloadFields,
    metrics: layout::BlockLayout,
) -> Node {
    if fields.show_fee_percent && fields.show_hashvalue {
        return block_row(
            vec![
                text_block("Miner Uptime", format::uptime(miner.uptime_s), metrics),
                text_block(
                    "Fees (144 Blocks)",
                    format::approx_fixed(public.avg_fee_percent, 1),
                    metrics,
                ),
                text_block(
                    "Hashvalue",
                    format::fixed_strip_zero_fraction(public.hashvalue_sat_th_day, 2),
                    metrics,
                ),
            ],
            metrics,
        );
    }
    block_row(
        vec![
            text_block("Miner Uptime", format::uptime(miner.uptime_s), metrics),
            fixed_width(metrics.block_width),
            fixed_width(metrics.block_width),
        ],
        metrics,
    )
}

pub(crate) fn info_overload(size: RenderSize, miner: &MinerData, public: &PublicData) -> Node {
    let fields = layout::info_overload_fields(layout::classify(viewport(size)));
    let metrics = layout::info_overload_layout();
    let mut rows = vec![info_overload_primary_row(miner, public, metrics)];
    if fields.show_difficulty_row {
        rows.push(info_overload_difficulty_row(public, metrics));
    }
    rows.push(info_overload_bottom_row(miner, public, fields, metrics));

    col(
        props!(background: BACKGROUND),
        [
            info_overload_header(public, fields.show_price_graph, metrics),
            space_between_rows(rows, metrics),
        ],
    )
}
