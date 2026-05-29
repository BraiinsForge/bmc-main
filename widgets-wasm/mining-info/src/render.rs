// Copyright (C) 2026  Braiins Systems s.r.o.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::format;
use crate::layout::{self, Viewport};
use crate::model::{MinerData, PublicData};

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
