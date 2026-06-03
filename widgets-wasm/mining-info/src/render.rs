// Copyright (C) 2026  Braiins Systems s.r.o.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::format;
use crate::layout::{self, Viewport};
use crate::model::{Availability, MinerData, PublicData};

const TITLE: Color = GRAY_50;
const UNIT: Color = GRAY_50;
const VALUE: Color = WHITE;
const BACKGROUND: Color = BLACK;

#[derive(Clone, Copy)]
pub(crate) struct RenderSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) const AUTH_ERROR_TEXT: &str = "Cannot authenticate";
pub(crate) const STALE_DATA_TEXT: &str = "Stale data";
const OVERLAY_TEXT_SIZE: u32 = 14;
const OVERLAY_ICON_PX: f32 = 16.0;
const OVERLAY_INSET: f32 = 8.0;

// A small absolutely-positioned banner pinned to the bottom-left corner. Only the
// bottom and left insets are set, so it sizes to its content and anchors there,
// overlapping whatever the view draws underneath.
fn error_overlay(message: &'static str) -> Node {
    row(
        props!(
            inset_bottom: OVERLAY_INSET,
            inset_left: OVERLAY_INSET,
            background: GRAY_100,
            padding: 6.0,
            gap: 6.0,
            cross_align: CrossAlign::Center
        ),
        [
            canvas(
                props!(width: OVERLAY_ICON_PX, height: OVERLAY_ICON_PX),
                [Draw::svg_builtin(
                    0.0,
                    0.0,
                    OVERLAY_ICON_PX,
                    OVERLAY_ICON_PX,
                    ICON_WARN_FILLED,
                    RED_50,
                )],
            ),
            text(
                message,
                style!(size: OVERLAY_TEXT_SIZE, weight: FontWeight::BOLD, color: RED_50),
            ),
        ],
    )
}

// Overlay an error banner onto a view's root column as an absolute child so it
// floats over the existing layout without disturbing it.
pub(crate) fn with_overlay(mut root: Node, message: &'static str) -> Node {
    if let Node::Column(_, children) = &mut root {
        children.push(error_overlay(message));
    }
    root
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
    value: String,
    unit: Option<&'static str>,
    size: u32,
    align: TextAlign,
    value_color: Color,
    weight: FontWeight,
) -> Node {
    let show_unit = unit_visible(&value);
    let mut spans = vec![span(value, style!(color: value_color))];
    if let Some(unit) = unit
        && show_unit
    {
        spans.push(span(bmc_wasm_sdk::fmt!("  {unit}"), style!(color: UNIT)));
    }
    paragraph(
        style!(size: size, weight: weight, color: value_color, align: align),
        spans,
    )
}

fn text_line(
    name: &'static str,
    value: String,
    unit: Option<&'static str>,
    sizes: layout::TextSizes,
) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            text(
                name,
                style!(size: sizes.title, weight: FontWeight::SEMIBOLD, color: TITLE, flex: 1.0),
            ),
            value_with_unit(
                value,
                unit,
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
                Some("TH/s"),
                sizes,
            ),
            text_line(
                "Temperature",
                format::temperature(miner.temperature),
                Some("°C"),
                sizes,
            ),
            text_line(
                "Power Consumption",
                format::fixed(miner.power_w, 0),
                Some("W"),
                sizes,
            ),
            text_line("MCR", format::fixed(miner.mcr_percent, 1), Some("%"), sizes),
            text_line(
                "Fan Speed",
                format::fixed(miner.fan_percent, 0),
                Some("%"),
                sizes,
            ),
            text_line(
                "IP Address",
                miner
                    .ip_address
                    .as_option()
                    .cloned()
                    .unwrap_or_else(format::unavailable),
                None,
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
                Some("TH/s"),
                sizes,
            ),
            text_line(
                "Temperature",
                format::temperature(miner.temperature),
                Some("°C"),
                sizes,
            ),
            text_line(
                "Power Consumption",
                format::fixed(miner.power_w, 0),
                Some("W"),
                sizes,
            ),
            text_line("Miner Uptime", format::uptime(miner.uptime_s), None, sizes),
            text_line(
                "IP Address",
                miner
                    .ip_address
                    .as_option()
                    .cloned()
                    .unwrap_or_else(format::unavailable),
                None,
                sizes,
            ),
            text_line("BTC Price", format::money(public.btc_price, 0), None, sizes),
        ],
    )
}

const BTC_PRICE_SIZE: u32 = 28;

fn block(
    name: &'static str,
    value: String,
    unit: Option<&'static str>,
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
                unit,
                metrics.text.value,
                TextAlign::Left,
                value_color,
                value_weight,
            ),
        ],
    )
}

fn text_block(
    name: &'static str,
    value: String,
    unit: Option<&'static str>,
    metrics: layout::BlockLayout,
) -> Node {
    block(name, value, unit, metrics, VALUE, FontWeight::REGULAR)
}

// Mirrors BOSer's `extra_info`: the primary value and its extra value share one
// row separated by " | " (see boser-assets text_block.slint).
fn text_block_with_extra(
    name: &'static str,
    value: String,
    unit: Option<&'static str>,
    extra_value: String,
    extra_unit: Option<&'static str>,
    metrics: layout::BlockLayout,
) -> Node {
    let show_unit = unit_visible(&value);
    let show_extra_unit = unit_visible(&extra_value);
    let mut spans = vec![span(value, style!(color: VALUE))];
    if let Some(unit) = unit
        && show_unit
    {
        spans.push(span(bmc_wasm_sdk::fmt!("  {unit}"), style!(color: UNIT)));
    }
    spans.push(span(" | ", style!(color: UNIT)));
    spans.push(span(" ", style!(color: VALUE)));
    spans.push(span(extra_value, style!(color: VALUE)));
    if let Some(extra_unit) = extra_unit
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
                Some("EH/s"),
                metrics,
            ),
            text_block(
                "Diff. Adjustment",
                format::signed_percent(public.prev_diff_adjust_percent, 2),
                Some("%"),
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
                    Some("%"),
                    metrics,
                ),
                text_block(
                    "Epoch Progress",
                    format::fixed(public.epoch_progress_percent, 0),
                    Some("%"),
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
            Some("BTC"),
            format::fixed(public.avg_fee_percent, 1),
            Some("%"),
            metrics,
        )
    } else {
        text_block(
            "Fees (144 Blocks)",
            format::approx_fixed(public.avg_fee_btc, 3),
            Some("BTC"),
            metrics,
        )
    };
    rows.push(block_row(
        vec![
            fee_block,
            text_block(
                "Block Height",
                format::public_integer(public.block_height),
                None,
                metrics,
            ),
        ],
        metrics,
    ));
    rows.push(block_row(
        vec![
            text_block(
                "Hashprice",
                format::money(public.hashprice, 3),
                Some("TH/Day"),
                metrics,
            ),
            text_block(
                "BTC Price",
                format::money(public.btc_price, 0),
                None,
                metrics,
            ),
        ],
        metrics,
    ));

    space_between_rows(rows, metrics)
}

fn info_overload_header(public: &PublicData, metrics: layout::BlockLayout) -> Node {
    let change_color = match public.btc_change_24h_percent {
        Availability::Available(value) if value >= 0.0 => GREEN_50,
        Availability::Available(_) => RED_60,
        Availability::Unavailable => TITLE,
    };

    // Glue the `%` directly onto the value to keep the headline tight and in the
    // change color, but only when the value is real so an unavailable change reads
    // as a clean `N/A` rather than `N/A%`.
    let change = format::signed_percent(public.btc_change_24h_percent, 2);
    let change = if unit_visible(&change) {
        bmc_wasm_sdk::fmt!("{change}%")
    } else {
        change
    };

    col(
        props!(background: GRAY_100),
        [
            fixed_height(18.0),
            block_row(
                vec![
                    block(
                        "Bitcoin (24h)",
                        change,
                        None,
                        metrics,
                        change_color,
                        FontWeight::BOLD,
                    ),
                    text(
                        format::money(public.btc_price, 0),
                        style!(
                            size: BTC_PRICE_SIZE,
                            weight: FontWeight::BOLD,
                            color: WHITE,
                            width: metrics.block_width
                        ),
                    ),
                ],
                metrics,
            ),
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
            text_block(
                "Hashrate",
                format::fixed(miner.hashrate_ths, 2),
                Some("TH/s"),
                metrics,
            ),
            text_block(
                "Power Consump.",
                format::fixed(miner.power_w, 0),
                Some("W"),
                metrics,
            ),
            text_block(
                "Block Height",
                format::public_integer(public.block_height),
                None,
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
                Some("%"),
                metrics,
            ),
            text_block(
                "Prev. Diff. Adjust.",
                format::signed_percent(public.prev_diff_adjust_percent, 2),
                Some("%"),
                metrics,
            ),
            text_block(
                "Epoch Progress",
                format::fixed(public.epoch_progress_percent, 0),
                Some("%"),
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
        let fee_value = format::approx_fixed(public.avg_fee_percent, 1);
        return block_row(
            vec![
                text_block(
                    "Miner Uptime",
                    format::uptime(miner.uptime_s),
                    None,
                    metrics,
                ),
                text_block("Fees (144 Blocks)", fee_value, Some("%"), metrics),
                text_block(
                    "Hashvalue",
                    format::fixed_strip_zero_fraction(public.hashvalue_sat_th_day, 2),
                    Some("SAT/TH/Day"),
                    metrics,
                ),
            ],
            metrics,
        );
    }
    block_row(
        vec![
            text_block(
                "Miner Uptime",
                format::uptime(miner.uptime_s),
                None,
                metrics,
            ),
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
            info_overload_header(public, metrics),
            space_between_rows(rows, metrics),
        ],
    )
}
