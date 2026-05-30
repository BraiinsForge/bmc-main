// Copyright (C) 2026  Braiins Systems s.r.o.

//! Round (BFM100, 480×480) render variants. Mining and Geek share a circular
//! 28-tick gauge with four quadrant stat clusters; Info Overload uses a
//! three-band layout. Geometry is authored in 480 native units and scaled by
//! `min(w, h) / 480`; typography is fixed (not scaled) per project convention.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use super::{
    BACKGROUND, RenderSize, VALUE, block_row, info_overload_header, text_block, unit_visible,
};
use crate::format;
use crate::gauge::{self, Gauge, GaugeState};
use crate::layout;
use crate::model::{Availability, MinerData, PublicData};

const NATIVE: f32 = 480.0;

// Ring: Ø460 (10px inset from the 480 bg), tick radial thickness ~15.2 → the
// arc centerline sits at radius (230 - 15.2/2).
const RING_RADIUS: f32 = 222.4;
const RING_WIDTH: f32 = 15.2;

// Center glow disc: ~Ø340 (radius 170) in 480 units, tinted by gauge state and
// fading to transparent at the edge.
const GLOW_RADIUS: f32 = 170.0;

const INACTIVE_TICK: Color = Color::from_rgb(0x1e, 0x1e, 0x1e);
const OFF_TICK: Color = Color::from_rgb(0xd9, 0x22, 0x2c);
const OFF_LABEL: Color = Color::from_rgb(0xf9, 0x53, 0x55);
const AMBER_DARK: Color = Color::from_rgb(0xcf, 0x79, 0x0e);
const AMBER_BRIGHT: Color = Color::from_rgb(0xfe, 0xba, 0x53);
const AMBER_LABEL: Color = Color::from_rgb(0xfe, 0xba, 0x53);
const GREEN_DARK: Color = Color::from_rgb(0x19, 0x5e, 0x33);
const GREEN_BRIGHT: Color = Color::from_rgb(0x5a, 0xdf, 0x88);
const GREEN_LABEL: Color = Color::from_rgb(0x34, 0xc0, 0x6a);
const PURPLE: Color = Color::from_rgb(0x8b, 0x7c, 0xff);
const LABEL_GRAY: Color = Color::from_rgb(0x8d, 0x8d, 0x8d);
const DIVIDER: Color = Color::from_rgba(0xff, 0xff, 0xff, 0x1a);

const HASHRATE_SIZE: u32 = 64;
const HASHRATE_UNIT_SIZE: u32 = 24;
const STATUS_SIZE: u32 = 16;
const CLUSTER_VALUE_SIZE: u32 = 32;
const CLUSTER_LABEL_SIZE: u32 = 16;

// Native (480-space) centers of the four quadrant clusters; the frame center is
// (240, 240). Derived from the Top (86,90,308×65) and Down (86,319,308×65)
// frames split at the x=239 divider.
const TL_CENTER: (f32, f32) = (155.0, 122.0);
const TR_CENTER: (f32, f32) = (323.0, 122.0);
const BL_CENTER: (f32, f32) = (155.0, 351.0);
const BR_CENTER: (f32, f32) = (323.0, 351.0);

#[expect(
    clippy::cast_precision_loss,
    reason = "viewport dimensions are small positive integers"
)]
fn px(v: u32) -> f32 {
    v as f32
}

struct ClusterSpec {
    label: &'static str,
    value: String,
    unit: Option<&'static str>,
}

fn ring_fill(state: GaugeState) -> Option<ArcFill> {
    match state {
        GaugeState::Off => Some(ArcFill::Solid(OFF_TICK)),
        GaugeState::UnknownScale => None,
        GaugeState::Underclocked => Some(ArcFill::gradient(AMBER_DARK, AMBER_BRIGHT)),
        GaugeState::Good => Some(ArcFill::gradient(GREEN_DARK, GREEN_BRIGHT)),
        GaugeState::Overclocked => Some(ArcFill::Solid(PURPLE)),
    }
}

fn status_color(state: GaugeState) -> Color {
    match state {
        GaugeState::Off => OFF_LABEL,
        GaugeState::UnknownScale => VALUE,
        GaugeState::Underclocked => AMBER_LABEL,
        GaugeState::Good => GREEN_LABEL,
        GaugeState::Overclocked => PURPLE,
    }
}

// Accent tint and center opacity for the glow disc, or None when there is no
// glow (unknown scale). The accent matches the status label color.
fn glow_spec(state: GaugeState) -> Option<(Color, f32)> {
    match state {
        GaugeState::Off => Some((OFF_LABEL, 0.40)),
        GaugeState::UnknownScale => None,
        GaugeState::Underclocked => Some((AMBER_LABEL, 0.30)),
        GaugeState::Good => Some((GREEN_LABEL, 0.30)),
        GaugeState::Overclocked => Some((PURPLE, 0.30)),
    }
}

fn draw_glow(draws: &mut Vec<Draw>, cx: f32, cy: f32, scale: f32, state: GaugeState) {
    let Some((accent, alpha)) = glow_spec(state) else {
        return;
    };
    draws.push(Draw::circle(
        cx,
        cy,
        GLOW_RADIUS * scale,
        Fill::radial(accent.with_alpha(alpha), accent.with_alpha(0.0)),
    ));
}

fn draw_gauge(draws: &mut Vec<Draw>, cx: f32, cy: f32, scale: f32, g: &Gauge) {
    let spans = gauge::tick_spans();
    let radius = RING_RADIUS * scale;
    let width = RING_WIDTH * scale;
    draws.push(Draw::arc(
        cx,
        cy,
        radius,
        0.0,
        std::f32::consts::TAU,
        width,
        ArcFill::Solid(INACTIVE_TICK),
        ArcSegments::Explicit(spans.clone()),
    ));
    if let Some(fill) = ring_fill(g.state) {
        let lit_count = g.lit_count.min(spans.len());
        let lit = spans[..lit_count].to_vec();
        if !lit.is_empty() {
            draws.push(Draw::arc(
                cx,
                cy,
                radius,
                0.0,
                std::f32::consts::TAU,
                width,
                fill,
                ArcSegments::Explicit(lit),
            ));
        }
    }
}

fn draw_dividers(draws: &mut Vec<Draw>, cx: f32, cy: f32, scale: f32) {
    // (native_x, native_y, native_w, native_h), 480-space.
    const BARS: [(f32, f32, f32, f32); 4] = [
        (239.0, 92.0, 1.0, 86.0),
        (104.0, 178.0, 272.0, 1.0),
        (104.0, 301.0, 272.0, 1.0),
        (239.0, 302.0, 1.0, 86.0),
    ];
    for (nx, ny, nw, nh) in BARS {
        let x = cx + (nx - NATIVE / 2.0) * scale;
        let y = cy + (ny - NATIVE / 2.0) * scale;
        draws.push(Draw::rect(x, y, nw * scale, nh * scale, DIVIDER));
    }
}

fn draw_center(
    draws: &mut Vec<Draw>,
    cx: f32,
    cy: f32,
    scale: f32,
    hashrate: Availability<f64>,
    state: GaugeState,
) {
    draws.push(Draw::text(
        cx,
        cy - 14.0 * scale,
        format::fixed(hashrate, 2),
        style!(
            size: HASHRATE_SIZE,
            weight: FontWeight::BOLD,
            color: VALUE,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));
    draws.push(Draw::text(
        cx,
        cy + 26.0 * scale,
        "TH/s",
        style!(
            size: HASHRATE_UNIT_SIZE,
            weight: FontWeight::REGULAR,
            color: VALUE,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));
    draws.push(Draw::text(
        cx,
        cy + 50.0 * scale,
        "Hashrate",
        style!(
            size: STATUS_SIZE,
            weight: FontWeight::REGULAR,
            color: status_color(state),
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));
}

fn draw_cluster(draws: &mut Vec<Draw>, cx: f32, cy: f32, scale: f32, spec: &ClusterSpec) {
    let value = match spec.unit {
        Some(unit) if unit_visible(&spec.value) => bmc_wasm_sdk::fmt!("{}  {unit}", spec.value),
        _ => spec.value.clone(),
    };
    draws.push(Draw::text(
        cx,
        cy - 12.0 * scale,
        value,
        style!(
            size: CLUSTER_VALUE_SIZE,
            weight: FontWeight::SEMIBOLD,
            color: VALUE,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));
    draws.push(Draw::text(
        cx,
        cy + 16.0 * scale,
        spec.label,
        style!(
            size: CLUSTER_LABEL_SIZE,
            weight: FontWeight::REGULAR,
            color: LABEL_GRAY,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));
}

fn native_to_px(cx: f32, cy: f32, scale: f32, native: (f32, f32)) -> (f32, f32) {
    (
        cx + (native.0 - NATIVE / 2.0) * scale,
        cy + (native.1 - NATIVE / 2.0) * scale,
    )
}

fn gauge_screen(
    size: RenderSize,
    g: &Gauge,
    hashrate: Availability<f64>,
    top_left: &ClusterSpec,
    top_right: &ClusterSpec,
    bottom_left: &ClusterSpec,
    bottom_right: &ClusterSpec,
) -> Node {
    let w = px(size.width);
    let h = px(size.height);
    let scale = w.min(h) / NATIVE;
    let cx = w / 2.0;
    let cy = h / 2.0;

    let mut draws: Vec<Draw> = Vec::with_capacity(24);
    // Backmost layer: the glow sits behind the gauge, dividers, and center text.
    draw_glow(&mut draws, cx, cy, scale, g.state);
    draw_gauge(&mut draws, cx, cy, scale, g);
    draw_dividers(&mut draws, cx, cy, scale);
    draw_center(&mut draws, cx, cy, scale, hashrate, g.state);
    for (center, spec) in [
        (TL_CENTER, top_left),
        (TR_CENTER, top_right),
        (BL_CENTER, bottom_left),
        (BR_CENTER, bottom_right),
    ] {
        let (cluster_x, cluster_y) = native_to_px(cx, cy, scale, center);
        draw_cluster(&mut draws, cluster_x, cluster_y, scale, spec);
    }

    col(
        props!(background: BACKGROUND),
        [canvas(props!(width: w, height: h), draws)],
    )
}

pub(crate) fn mining(size: RenderSize, miner: &MinerData) -> Node {
    let g = gauge::gauge(miner.hashrate_ths, miner.mcr_percent);
    gauge_screen(
        size,
        &g,
        miner.hashrate_ths,
        &ClusterSpec {
            label: "Power Cons.",
            value: format::fixed(miner.power_w, 0),
            unit: Some("W"),
        },
        &ClusterSpec {
            label: "MCR",
            value: format::fixed(miner.mcr_percent, 1),
            unit: Some("%"),
        },
        &ClusterSpec {
            label: "Temperature",
            value: format::temperature(miner.temperature),
            unit: Some("°C"),
        },
        &ClusterSpec {
            label: "Fan Speed",
            value: format::fixed(miner.fan_percent, 0),
            unit: Some("%"),
        },
    )
}

pub(crate) fn geek(size: RenderSize, miner: &MinerData, public: &PublicData) -> Node {
    let g = gauge::gauge(miner.hashrate_ths, miner.mcr_percent);
    gauge_screen(
        size,
        &g,
        miner.hashrate_ths,
        &ClusterSpec {
            label: "Power Cons.",
            value: format::fixed(miner.power_w, 0),
            unit: Some("W"),
        },
        &ClusterSpec {
            label: "Efficiency",
            value: format::fixed(miner.efficiency_j_th, 1),
            unit: Some("J/TH"),
        },
        &ClusterSpec {
            label: "Temperature",
            value: format::temperature(miner.temperature),
            unit: Some("°C"),
        },
        &ClusterSpec {
            label: "BTC Price",
            value: format::money(public.btc_price, 0),
            unit: None,
        },
    )
}

pub(crate) fn info_overload(miner: &MinerData, public: &PublicData) -> Node {
    let metrics = layout::info_overload_layout();

    let upper = block_row(
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
    );

    let fee_value = bmc_wasm_sdk::fmt!("~ {}", format::fixed(public.avg_fee_percent, 1));
    let lower = block_row(
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

    col(
        props!(background: BACKGROUND),
        [
            spacer(1.0),
            upper,
            spacer(1.0),
            info_overload_header(public, true, metrics),
            spacer(1.0),
            lower,
            spacer(1.0),
        ],
    )
}
