// Copyright (C) 2026  Braiins Systems s.r.o.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::format;
use crate::layout::{self, Viewport};
use crate::model::{Availability, MinerData, PublicData};

const TITLE: Color = GRAY_60;
const VALUE: Color = WHITE;
const BACKGROUND: Color = BLACK;

// Stable typography. The spec forbids viewport-width-scaled font sizes; only
// field visibility and spacing respond to the viewport class.
const LINE_TITLE_SIZE: u32 = 14;
const LINE_VALUE_SIZE: u32 = 16;

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

fn text_line(name: &'static str, value: String, unit: Option<&'static str>) -> Node {
    let value_text = match unit {
        Some(unit) if value != format::NOT_AVAILABLE => bmc_wasm_sdk::fmt!("{} {}", value, unit),
        _ => value,
    };
    row(
        props!(cross_align: CrossAlign::Center),
        [
            text(
                name,
                style!(size: LINE_TITLE_SIZE, weight: FontWeight::REGULAR, color: TITLE, flex: 1.0),
            ),
            text(
                value_text,
                style!(size: LINE_VALUE_SIZE, weight: FontWeight::BOLD, color: VALUE, align: TextAlign::Right),
            ),
        ],
    )
}

// Mirrors BOSer's `VerticalLayout { alignment: space-between }`: flex spacers
// between rows distribute the rows across the full viewport height instead of
// packing them at the top. The SDK has no main-axis justify.
fn vertical_lines(size: RenderSize, lines: Vec<Node>) -> Node {
    let spacing = layout::spacing(layout::classify(viewport(size)));
    let mut children = Vec::with_capacity(lines.len().saturating_mul(2));
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            children.push(spacer(1.0));
        }
        children.push(line);
    }
    col(
        props!(
            background: BACKGROUND,
            padding: spacing.padding
        ),
        children,
    )
}

pub(crate) fn mining(size: RenderSize, miner: &MinerData) -> Node {
    vertical_lines(
        size,
        vec![
            text_line(
                "Current Hashrate",
                format::fixed(miner.hashrate_ths, 2),
                Some("TH/s"),
            ),
            text_line(
                "Temperature",
                format::temperature(miner.temperature),
                Some("°C"),
            ),
            text_line(
                "Power Consumption",
                format::fixed(miner.power_w, 0),
                Some("W"),
            ),
            text_line("MCR", format::fixed(miner.mcr_percent, 1), Some("%")),
            text_line("Fan Speed", format::fixed(miner.fan_percent, 0), Some("%")),
            text_line(
                "IP Address",
                miner
                    .ip_address
                    .as_option()
                    .cloned()
                    .unwrap_or_else(format::unavailable),
                None,
            ),
        ],
    )
}

pub(crate) fn geek(size: RenderSize, miner: &MinerData, public: &PublicData) -> Node {
    vertical_lines(
        size,
        vec![
            text_line(
                "Current Hashrate",
                format::fixed(miner.hashrate_ths, 2),
                Some("TH/s"),
            ),
            text_line(
                "Temperature",
                format::temperature(miner.temperature),
                Some("°C"),
            ),
            text_line(
                "Power Consumption",
                format::fixed(miner.power_w, 0),
                Some("W"),
            ),
            text_line("Miner Uptime", format::uptime(miner.uptime_s), None),
            text_line(
                "IP Address",
                miner
                    .ip_address
                    .as_option()
                    .cloned()
                    .unwrap_or_else(format::unavailable),
                None,
            ),
            text_line("BTC Price", format::money(public.btc_price, 0), None),
        ],
    )
}

// Stable block typography. Per spec, only field visibility, spacing, and the
// column count respond to the viewport class; font sizes stay fixed.
const BLOCK_TITLE_SIZE: u32 = 13;
const BLOCK_VALUE_SIZE: u32 = 16;
const BTC_LABEL_SIZE: u32 = 16;
const BTC_PRICE_SIZE: u32 = 28;

fn block_value_text(value: String, unit: Option<&'static str>) -> String {
    match unit {
        Some(unit) if value != format::NOT_AVAILABLE && value != format::PUBLIC_NOT_AVAILABLE => {
            bmc_wasm_sdk::fmt!("{} {}", value, unit)
        }
        _ => value,
    }
}

fn block(name: &'static str, value_text: String) -> Node {
    col(
        props!(gap: 2.0),
        [
            text(name, style!(size: BLOCK_TITLE_SIZE, color: TITLE)),
            text(
                value_text,
                style!(size: BLOCK_VALUE_SIZE, weight: FontWeight::BOLD, color: VALUE),
            ),
        ],
    )
}

fn text_block(name: &'static str, value: String, unit: Option<&'static str>) -> Node {
    block(name, block_value_text(value, unit))
}

// Mirrors BOSer's `extra_info`: the primary value and its extra value share one
// row separated by " | " (see boser-assets text_block.slint).
fn text_block_with_extra(
    name: &'static str,
    value: String,
    unit: Option<&'static str>,
    extra_value: String,
    extra_unit: Option<&'static str>,
) -> Node {
    block(
        name,
        bmc_wasm_sdk::fmt!(
            "{} | {}",
            block_value_text(value, unit),
            block_value_text(extra_value, extra_unit)
        ),
    )
}

// Chunk blocks into fixed-width grid rows. Each cell takes equal flex so a
// field value can never resize its column; a short final row is padded with
// empty cells to keep column widths aligned with the rows above.
fn into_rows(blocks: Vec<Node>, columns: usize, gap: f32) -> Vec<Node> {
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    for block in blocks {
        cells.push(col(props!(flex: 1.0), [block]));
        if cells.len() == columns {
            rows.push(row(props!(gap: gap), std::mem::take(&mut cells)));
        }
    }
    if !cells.is_empty() {
        while cells.len() < columns {
            cells.push(col(props!(flex: 1.0), Vec::<Node>::new()));
        }
        rows.push(row(props!(gap: gap), cells));
    }
    rows
}

pub(crate) fn network(size: RenderSize, public: &PublicData) -> Node {
    let class = layout::classify(viewport(size));
    let fields = layout::network_fields(class);
    let spacing = layout::spacing(class);

    let mut blocks = vec![
        text_block(
            "Network HR",
            format::fixed(public.network_hashrate_ehs, 2),
            Some("EH/s"),
        ),
        text_block(
            "Diff. Adjustment",
            format::signed_percent(public.prev_diff_adjust_percent, 2),
            Some("%"),
        ),
    ];
    if fields.show_extra_difficulty {
        blocks.push(text_block(
            "Est. Diff. Adjustment",
            format::signed_percent(public.est_diff_adjust_percent, 2),
            Some("%"),
        ));
        blocks.push(text_block(
            "Epoch Progress",
            format::fixed(public.epoch_progress_percent, 0),
            Some("%"),
        ));
    }
    if fields.show_fee_percent {
        blocks.push(text_block_with_extra(
            "Fees (144 Blocks)",
            format::fixed(public.avg_fee_btc, 3),
            Some("BTC"),
            format::fixed(public.avg_fee_percent, 1),
            Some("%"),
        ));
    } else {
        blocks.push(text_block(
            "Fees (144 Blocks)",
            format::fixed(public.avg_fee_btc, 3),
            Some("BTC"),
        ));
    }
    blocks.push(text_block(
        "Block Height",
        format::public_integer(public.block_height),
        None,
    ));
    blocks.push(text_block(
        "Hashprice",
        format::money(public.hashprice, 3),
        Some("TH/Day"),
    ));
    blocks.push(text_block(
        "BTC Price",
        format::money(public.btc_price, 0),
        None,
    ));

    col(
        props!(
            background: BACKGROUND,
            padding: spacing.padding,
            gap: spacing.gap
        ),
        into_rows(blocks, spacing.columns, spacing.gap),
    )
}

pub(crate) fn info_overload(size: RenderSize, miner: &MinerData, public: &PublicData) -> Node {
    let class = layout::classify(viewport(size));
    let fields = layout::info_overload_fields(class);
    let spacing = layout::spacing(class);
    let change_color = match public.btc_change_24h_percent {
        Availability::Available(value) if value >= 0.0 => GREEN_50,
        Availability::Available(_) => RED_60,
        Availability::Unavailable => TITLE,
    };

    let header = row(
        props!(
            background: GRAY_100,
            padding: spacing.padding,
            cross_align: CrossAlign::Center,
            gap: spacing.gap
        ),
        [
            col(
                props!(gap: 2.0),
                [
                    text("Bitcoin (24h)", style!(size: BTC_LABEL_SIZE, color: TITLE)),
                    text(
                        bmc_wasm_sdk::fmt!(
                            "{}%",
                            format::signed_percent(public.btc_change_24h_percent, 2)
                        ),
                        style!(size: BTC_LABEL_SIZE, weight: FontWeight::BOLD, color: change_color),
                    ),
                ],
            ),
            text(
                format::money(public.btc_price, 0),
                style!(size: BTC_PRICE_SIZE, weight: FontWeight::BOLD, color: WHITE),
            ),
        ],
    );

    let mut blocks = vec![
        text_block(
            "Hashrate",
            format::fixed(miner.hashrate_ths, 2),
            Some("TH/s"),
        ),
        text_block("Power Consump.", format::fixed(miner.power_w, 0), Some("W")),
        text_block(
            "Block Height",
            format::public_integer(public.block_height),
            None,
        ),
    ];
    if fields.show_difficulty_row {
        blocks.push(text_block(
            "Est. Diff. Adjust.",
            format::signed_percent(public.est_diff_adjust_percent, 2),
            Some("%"),
        ));
        blocks.push(text_block(
            "Prev. Diff. Adjust.",
            format::signed_percent(public.prev_diff_adjust_percent, 2),
            Some("%"),
        ));
        blocks.push(text_block(
            "Epoch Progress",
            format::fixed(public.epoch_progress_percent, 0),
            Some("%"),
        ));
    }
    blocks.push(text_block(
        "Miner Uptime",
        format::uptime(miner.uptime_s),
        None,
    ));
    if fields.show_fee_percent {
        blocks.push(text_block(
            "Fees (144 Blocks)",
            format::fixed(public.avg_fee_percent, 1),
            Some("%"),
        ));
    }
    if fields.show_hashvalue {
        blocks.push(text_block(
            "Hashvalue",
            format::fixed(public.hashvalue_sat_th_day, 2),
            Some("SAT/TH/Day"),
        ));
    }

    col(
        props!(background: BACKGROUND),
        [
            header,
            col(
                props!(
                    padding: spacing.padding,
                    gap: spacing.gap
                ),
                into_rows(blocks, spacing.columns, spacing.gap),
            ),
        ],
    )
}
