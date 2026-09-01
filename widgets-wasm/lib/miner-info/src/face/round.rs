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
    BACKGROUND, RenderSize, VALUE, fixed_height, fixed_width, info_overload_header, text_block,
    unit_visible,
};
use crate::format;
use crate::layout;
use crate::model::{Availability, MinerData, PublicData};
use mining::gauge::{self, Gauge, GaugeState};
use mining::style::{AMBER_LABEL, GREEN_LABEL, INACTIVE_TICK, OFF_LABEL, PURPLE, ring_fill};

const NATIVE: f32 = 480.0;

// Ring: Ø460 (10px inset from the 480 bg), tick radial thickness ~15.2 → the
// arc centerline sits at radius (230 - 15.2/2).
const RING_RADIUS: f32 = 220.0;
const RING_WIDTH: f32 = 10.0;

// Center glow disc: ~Ø340 (radius 170) in 480 units, tinted by gauge state and
// fading to transparent at the edge.
const GLOW_RADIUS: f32 = 150.0;

// Duration of the lit-gauge sweep transition when the fill changes.
const GAUGE_TRANSITION_MS: u32 = 500;

const LABEL_GRAY: Color = Color::from_rgb(0x8d, 0x8d, 0x8d);
const DIVIDER: Color = Color::from_rgba(0xff, 0xff, 0xff, 0x1a);

const HASHRATE_SIZE: u32 = 64;
const HASHRATE_UNIT_SIZE: u32 = 24;
const STATUS_SIZE: u32 = 16;
// The "1 min" caption reads as a sub-note under "Hashrate", a step below the
// other gray labels.
const CAPTION_SIZE: u32 = 14;
const CLUSTER_VALUE_SIZE: u32 = 32;
const CLUSTER_LABEL_SIZE: u32 = 16;
const CLUSTER_UNIT_SIZE: u32 = 16;

// Top-center "<icon> <chip model> x<count>" header. Native y sits below the rim
// and clear of the top quadrant clusters at native y=122. Icon and text are
// fixed-size (typography is not scaled); the gap and placement scale.
const CHIP_HEADER_NATIVE_Y: f32 = 58.0;
const CHIP_ICON_SIZE: f32 = 22.0;
const CHIP_TEXT_SIZE: u32 = 18;
// Text line-height for the header, set explicitly so the cell-centering math
// below can use the same value the glyphs are laid out with.
const CHIP_LINE_HEIGHT: f32 = 1.4;
const CHIP_GAP: f32 = 6.0;
const CHIP_CELL_W: f32 = 260.0;

// Gap between the Info Overload center band and the upper/lower value rows;
// keeps all three bands clustered near the vertical center on the round face.
const BAND_GAP: f32 = 16.0;

// Rightward nudge for only the left column of the wide three-value bands. The
// left text otherwise hugs the bezel on the narrower chords above and below
// center; this clears it while leaving the middle and right columns in place.
const INFO_LEFT_SHIFT: f32 = 20.0;

// Gap between a cluster value and its unit, and between the value row and the
// label. Native (480-space) units, scaled at render time.
const CLUSTER_UNIT_GAP: f32 = 4.0;
const CLUSTER_VALUE_LABEL_GAP: f32 = 2.0;

// Each cluster occupies a fixed cell centered on its quadrant point. The width
// matches the inter-cluster spacing so the two cells in a row meet exactly at
// the x=239 divider; Node layout then centers the value+unit+label group inside
// the cell, so no text measurement is needed to center the group on its point.
const CLUSTER_CELL_W: f32 = 168.0;
const CLUSTER_CELL_H: f32 = 86.0;

// Center hashrate cell: the value and "TH/s" share a row with the label below,
// centered as a group on the frame center the same way the quadrant clusters
// center inside a fixed cell. The cell is taller than the value+label group so
// the surrounding spacers center it rather than collapsing.
const CENTER_CELL_W: f32 = 300.0;
const CENTER_CELL_H: f32 = 140.0;
const CENTER_UNIT_GAP: f32 = 6.0;
const CENTER_LABEL_GAP: f32 = 5.0;

// Fixed pixel width reserved for the trailing "TH/s" unit, mirrored by an empty
// slot of equal width on the left of the value so the value alone lands on the
// frame center. Sized to clear "TH/s" at `HASHRATE_UNIT_SIZE`; not scaled, since
// typography is fixed.
const CENTER_UNIT_SLOT_W: f32 = 60.0;

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
    prefix: Option<&'static str>,
    value: format::Rendered,
}

// The center glow disc: an accent tint matching the status label color, faded
// over `alpha` at the center to transparent at the edge.
#[derive(Clone, Copy)]
struct Glow {
    accent: Color,
    alpha: f32,
}

// The per-state render treatment of the gauge label and glow: the `Hashrate`
// status-label color and the center glow. The lit ring fill comes from the
// shared `mining::style::ring_fill`. `NotAvailable` has a neutral white glow.
#[derive(Clone, Copy)]
struct StateStyle {
    status_color: Color,
    glow: Option<Glow>,
}

const NOT_AVAILABLE_STYLE: StateStyle = StateStyle {
    status_color: LABEL_GRAY,
    glow: Some(Glow {
        accent: WHITE,
        alpha: 0.14,
    }),
};

const OFF_STYLE: StateStyle = StateStyle {
    status_color: OFF_LABEL,
    glow: Some(Glow {
        accent: OFF_LABEL,
        alpha: 0.40,
    }),
};

const UNDERCLOCKED_STYLE: StateStyle = StateStyle {
    status_color: AMBER_LABEL,
    glow: Some(Glow {
        accent: AMBER_LABEL,
        alpha: 0.30,
    }),
};

const GOOD_STYLE: StateStyle = StateStyle {
    status_color: GREEN_LABEL,
    glow: Some(Glow {
        accent: GREEN_LABEL,
        alpha: 0.30,
    }),
};

const OVERCLOCKED_STYLE: StateStyle = StateStyle {
    status_color: PURPLE,
    glow: Some(Glow {
        accent: PURPLE,
        alpha: 0.30,
    }),
};

const fn style(state: GaugeState) -> StateStyle {
    match state {
        GaugeState::NotAvailable => NOT_AVAILABLE_STYLE,
        GaugeState::Off => OFF_STYLE,
        GaugeState::Underclocked => UNDERCLOCKED_STYLE,
        GaugeState::Good => GOOD_STYLE,
        GaugeState::Overclocked => OVERCLOCKED_STYLE,
    }
}

fn draw_glow(draws: &mut Vec<Draw>, cx: f32, cy: f32, scale: f32, state: GaugeState) {
    let Some(glow) = style(state).glow else {
        return;
    };
    draws.push(Draw::circle(
        cx,
        cy,
        GLOW_RADIUS * scale,
        Fill::radial(
            glow.accent.with_alpha(glow.alpha),
            glow.accent.with_alpha(0.0),
        ),
    ));
}

fn draw_gauge(draws: &mut Vec<Draw>, cx: f32, cy: f32, scale: f32, g: &Gauge) {
    let spans = gauge::TICK_SPANS;
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
        ArcSegments::Explicit(spans.to_vec()),
        ArcCap::Butt,
    ));
    let lit_count = g.lit_count.min(spans.len());
    // The lit overlay carries the full tick ring; its sweep (0..lit boundary) clips it down
    // to the lit prefix in the renderer. Emitting every tick lets the host's sweep transition
    // reveal or hide ticks in place, and keeps the draw anchored across the load: hashrate
    // and MCR arrive on separate endpoints, so the gauge passes through a no-fill window
    // (hashing, scale unknown) where the sweep is 0 — the overlay clips to nothing there
    // rather than vanishing, so the real fill animates in instead of popping.
    //
    // The no-scale state has no lit ticks, so its fill is never visible;
    // the neutral ring color is a placeholder to keep the draw.
    let fill = ring_fill(g.state).unwrap_or(ArcFill::Solid(INACTIVE_TICK));
    draws.push(
        Draw::arc(
            cx,
            cy,
            radius,
            0.0,
            gauge::lit_sweep_end(lit_count),
            width,
            fill,
            ArcSegments::Explicit(spans.to_vec()),
            ArcCap::Butt,
        )
        .transition("gauge-lit", GAUGE_TRANSITION_MS, Easing::EaseOutCubic),
    );
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

// The center hashrate: the value (e.g. "1.00") sits on the frame center
// (cx, cy) with "TH/s" trailing to its right, and the status label below.
// The value alone is horizontally centered — not the value+unit group
// — by reserving a fixed unit slot on the right and mirroring it with
// an empty slot of equal width on the left, the symmetric-row trick
// the digital clock uses for the time + AM/PM (clock digital.rs `time_row`).
// Vertically the value+label group is centered as a unit,
// leaving the value a little above dead-center, matching the design.
fn center_node(
    cx: f32,
    cy: f32,
    scale: f32,
    hashrate: Availability<Hashrate>,
    state: GaugeState,
) -> Node {
    let cell_w = CENTER_CELL_W * scale;
    let cell_h = CENTER_CELL_H * scale;
    let inset_left = cx - cell_w / 2.0;
    let inset_top = cy - cell_h / 2.0;

    let value = text(
        format::fixed(hashrate, 2).value,
        style!(size: HASHRATE_SIZE, weight: FontWeight::BOLD, color: VALUE, line_height: 1.0),
    );
    let unit_slot = col(
        props!(width: CENTER_UNIT_SLOT_W, cross_align: CrossAlign::Start),
        [text(
            Hashrate::UNIT,
            style!(size: HASHRATE_UNIT_SIZE, weight: FontWeight::REGULAR, color: VALUE, line_height: 1.0),
        )],
    );
    let left_reserve = col(props!(width: CENTER_UNIT_SLOT_W), Vec::<Node>::new());
    let value_row = row(
        props!(cross_align: CrossAlign::Center, gap: CENTER_UNIT_GAP * scale),
        [left_reserve, value, unit_slot],
    );

    col(
        props!(
            inset_left: inset_left,
            inset_top: inset_top,
            width: cell_w,
            height: cell_h,
            cross_align: CrossAlign::Center,
            gap: CENTER_LABEL_GAP * scale
        ),
        [
            spacer(1.0),
            value_row,
            text(
                "Hashrate",
                style!(size: STATUS_SIZE, weight: FontWeight::REGULAR, color: style(state).status_color),
            ),
            spacer(1.0),
        ],
    )
}

// The "1 min" caption under the center "Hashrate" label, marking the readout
// as the 1-minute hashrate average. Placed as an absolute sibling rather than
// a third row in `center_node` so the value and "Hashrate" group keeps
// its exact centered position.
//
// The value+label group is centered on `cy`, so its bottom edge sits half
// the group height below center; the caption sits at that edge, snug under
// the label (the label and caption line-height padding supply the visible gap),
// derived from the same constants the group uses so the two stay aligned across scales.
fn center_caption(cx: f32, cy: f32, scale: f32) -> Node {
    let cell_w = CENTER_CELL_W * scale;
    let group_half =
        f32::midpoint(px(HASHRATE_SIZE), px(STATUS_SIZE)) + CENTER_LABEL_GAP * scale / 2.0;
    let top = cy + group_half - 2.0 * scale;
    col(
        props!(
            inset_left: cx - cell_w / 2.0,
            inset_top: top,
            width: cell_w,
            cross_align: CrossAlign::Center
        ),
        [text(
            "1 min",
            style!(size: CAPTION_SIZE, weight: FontWeight::REGULAR, color: LABEL_GRAY),
        )],
    )
}

// Top-center chip header: "<icon> <model> x<count>", centered on the frame's
// horizontal center and inset below the rim. The icon is drawn into a small
// fixed-size canvas so it flows in the flex row and the whole group centers
// together. The model reads in the value tone, the count in the label gray.
fn chip_header(cx: f32, cy: f32, scale: f32, chip_icon: &Svg, model: &str, count: usize) -> Node {
    let icon = canvas(
        props!(width: CHIP_ICON_SIZE, height: CHIP_ICON_SIZE),
        vec![
            Draw::svg(
                0.0,
                0.0,
                CHIP_ICON_SIZE,
                CHIP_ICON_SIZE,
                chip_icon,
                LABEL_GRAY,
            )
            .with_anti_alias(),
        ],
    );
    let label = row(
        props!(cross_align: CrossAlign::Center, gap: CHIP_GAP * scale),
        [
            icon,
            text(
                model,
                style!(size: CHIP_TEXT_SIZE, weight: FontWeight::SEMIBOLD, color: VALUE, line_height: CHIP_LINE_HEIGHT),
            ),
            text(
                fmt!("x{count}"),
                style!(size: CHIP_TEXT_SIZE, weight: FontWeight::REGULAR, color: LABEL_GRAY, line_height: CHIP_LINE_HEIGHT),
            ),
        ],
    );
    let cell_w = CHIP_CELL_W * scale;
    let (center_x, center_y) = native_to_px(cx, cy, scale, (NATIVE / 2.0, CHIP_HEADER_NATIVE_Y));
    // The flex row is as tall as its tallest child. The text line box (CHIP_TEXT_SIZE * CHIP_LINE_HEIGHT)
    // is taller than the icon, so it sets the row height; centering the cell on that — not on the icon
    // — lands the header on CHIP_HEADER_NATIVE_Y without disturbing the icon/text alignment.
    let row_height = (px(CHIP_TEXT_SIZE) * CHIP_LINE_HEIGHT).max(CHIP_ICON_SIZE);
    col(
        props!(
            inset_left: center_x - cell_w / 2.0,
            inset_top: center_y - row_height / 2.0,
            width: cell_w,
            cross_align: CrossAlign::Center
        ),
        [label],
    )
}

// One quadrant cluster, laid out as a Node so the value+unit
// row and label center as a group (cross_align) inside a fixed cell.
//
// The cell is positioned absolutely with its top-left corner derived from
// the quadrant center and the known cell size, so the group lands centered
// on the point without measuring any text. Overlaid on the gauge canvas
// as an absolute child of the root.
fn cluster_node(center_px: (f32, f32), scale: f32, spec: &ClusterSpec) -> Node {
    let cell_w = CLUSTER_CELL_W * scale;
    let cell_h = CLUSTER_CELL_H * scale;
    let inset_left = center_px.0 - cell_w / 2.0;
    let inset_top = center_px.1 - cell_h / 2.0;

    // Prefix (currency symbol) and unit render at the smaller unit size
    // and only when the value is real, so a "N/A"/"--" placeholder stays unadorned.
    let show_affixes = unit_visible(&spec.value.value);
    let mut parts: Vec<Node> = Vec::with_capacity(3);
    if let Some(prefix) = spec.prefix.filter(|_| show_affixes) {
        parts.push(text(
            prefix,
            style!(size: CLUSTER_UNIT_SIZE, weight: FontWeight::REGULAR, color: VALUE),
        ));
    }
    parts.push(text(
        spec.value.value.clone(),
        style!(size: CLUSTER_VALUE_SIZE, weight: FontWeight::SEMIBOLD, color: VALUE),
    ));
    if let Some(unit) = spec.value.unit.filter(|_| show_affixes) {
        parts.push(text(
            unit,
            style!(size: CLUSTER_UNIT_SIZE, weight: FontWeight::REGULAR, color: VALUE),
        ));
    }
    let value_row = row(
        props!(cross_align: CrossAlign::Center, gap: CLUSTER_UNIT_GAP * scale),
        parts,
    );

    col(
        props!(
            inset_left: inset_left,
            inset_top: inset_top,
            width: cell_w,
            height: cell_h,
            cross_align: CrossAlign::Center,
            gap: CLUSTER_VALUE_LABEL_GAP * scale
        ),
        [
            spacer(1.0),
            value_row,
            text(
                spec.label,
                style!(size: CLUSTER_LABEL_SIZE, weight: FontWeight::REGULAR, color: LABEL_GRAY),
            ),
            spacer(1.0),
        ],
    )
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
    hashrate: Availability<Hashrate>,
    chip: Option<(&str, usize)>,
    chip_icon: &Svg,
    clusters: &[ClusterSpec; 4],
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

    let mut children = vec![
        canvas(props!(width: w, height: h), draws),
        center_node(cx, cy, scale, hashrate, g.state),
        center_caption(cx, cy, scale),
    ];
    if let Some((model, count)) = chip {
        children.push(chip_header(cx, cy, scale, chip_icon, model, count));
    }
    let centers = [TL_CENTER, TR_CENTER, BL_CENTER, BR_CENTER];
    for (center, spec) in centers.into_iter().zip(clusters.iter()) {
        children.push(cluster_node(
            native_to_px(cx, cy, scale, center),
            scale,
            spec,
        ));
    }

    col(props!(background: BACKGROUND), children)
}

// The gauge for the round Mining/Geek faces. On the seed frame the lit count
// is pinned to a single tick so the host transition has an empty-ish baseline
// to animate the real fill in from, regardless of whether data is already loaded.
fn seeded_gauge(miner: &MinerData, seed_gauge: bool) -> Gauge {
    let mut g = gauge::gauge(
        miner
            .hashrate
            .as_option()
            .map(|h| h.as_terahashes_per_second()),
        miner.constraints.hashrate.as_ref(),
    );
    if seed_gauge {
        g.lit_count = g.lit_count.min(1);
    }
    g
}

// The chip model + count header, present only when both are known.
fn chip_header_data(miner: &MinerData) -> Option<(&str, usize)> {
    let model = miner.chip_type.as_option()?;
    let count = *miner.chip_count.as_option()?;
    Some((model.as_str(), count))
}

#[must_use]
pub fn mining(size: RenderSize, miner: &MinerData, seed_gauge: bool, chip_icon: &Svg) -> Node {
    let g = seeded_gauge(miner, seed_gauge);
    gauge_screen(
        size,
        &g,
        miner.hashrate,
        chip_header_data(miner),
        chip_icon,
        &[
            ClusterSpec {
                label: "Power Cons.",
                prefix: None,
                value: format::fixed(miner.power, 0),
            },
            ClusterSpec {
                label: "MCR",
                prefix: None,
                value: format::fixed(miner.mcr, 1),
            },
            ClusterSpec {
                label: "Temperature",
                prefix: None,
                value: format::chip_temperature(miner.temperature),
            },
            ClusterSpec {
                label: "Fan Speed",
                prefix: None,
                value: format::fixed(miner.fan_speed, 0),
            },
        ],
    )
}

#[must_use]
pub fn geek(
    size: RenderSize,
    miner: &MinerData,
    public: &PublicData,
    seed_gauge: bool,
    chip_icon: &Svg,
) -> Node {
    let g = seeded_gauge(miner, seed_gauge);
    gauge_screen(
        size,
        &g,
        miner.hashrate,
        chip_header_data(miner),
        chip_icon,
        &[
            ClusterSpec {
                label: "Power Cons.",
                prefix: None,
                value: format::fixed(miner.power, 0),
            },
            ClusterSpec {
                label: "Efficiency",
                prefix: None,
                value: format::fixed(miner.efficiency, 1),
            },
            ClusterSpec {
                label: "Temperature",
                prefix: None,
                value: format::chip_temperature(miner.temperature),
            },
            ClusterSpec {
                label: "BTC Price",
                prefix: format::money_symbol(public.btc_price),
                value: format::money_amount(public.btc_price, 0).into(),
            },
        ],
    )
}

// A horizontally centered row of blocks for the narrow top/bottom
// edge bands, where the circle's chord only fits one or two blocks.
// The block group is centered between spacers rather than
// left-aligned-with-padding like the wide middle bands.
fn centered_block_row(blocks: Vec<Node>, metrics: layout::BlockLayout) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            spacer(1.0),
            row(props!(gap: metrics.horizontal_gap), blocks),
            spacer(1.0),
        ],
    )
}

// A three-column band that keeps the middle column centered on the frame
// and the right column at its centered position, while nudging only the left column
// right by `shift`(which narrows the left-to-middle gap by the same amount).
fn left_shifted_block_row(
    left: Node,
    middle: Node,
    right: Node,
    metrics: layout::BlockLayout,
    shift: f32,
) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            fixed_width(metrics.padding_horizontal + shift),
            left,
            fixed_width(metrics.horizontal_gap - shift),
            middle,
            fixed_width(metrics.horizontal_gap),
            right,
            spacer(1.0),
        ],
    )
}

#[must_use]
pub fn info_overload(miner: &MinerData, public: &PublicData) -> Node {
    // The round face exists only at 480x480, which is never the narrow grid.
    let metrics = layout::info_overload_layout(layout::ViewportClass::Large);

    let top_edge = centered_block_row(
        vec![
            text_block(
                "Est. Diff. Adjust.",
                format::signed_percent(public.est_diff_adjust, 2),
                metrics,
            ),
            text_block(
                "Prev. Diff. Adjust.",
                format::signed_percent(public.prev_diff_adjust, 2),
                metrics,
            ),
        ],
        metrics,
    );

    let bottom_edge = centered_block_row(
        vec![text_block(
            "Hashvalue",
            format::fixed_strip_zero_fraction(public.hashvalue, 2),
            metrics,
        )],
        metrics,
    );

    let upper = left_shifted_block_row(
        text_block("Hashrate", format::fixed(miner.hashrate, 2), metrics),
        text_block("Power Consump.", format::fixed(miner.power, 0), metrics),
        text_block(
            "Block Height",
            format::public_integer(public.block_height),
            metrics,
        ),
        metrics,
        INFO_LEFT_SHIFT,
    );

    let lower = left_shifted_block_row(
        text_block("Miner Uptime", format::uptime(miner.uptime), metrics),
        text_block(
            "Fees (144 Blocks)",
            format::approx_fixed(public.avg_fee_share, 1),
            metrics,
        ),
        text_block(
            "Epoch Prog.",
            format::fixed(public.epoch_progress, 0),
            metrics,
        ),
        metrics,
        INFO_LEFT_SHIFT,
    );

    col(
        props!(background: BACKGROUND),
        [
            spacer(1.0),
            top_edge,
            fixed_height(BAND_GAP),
            upper,
            fixed_height(BAND_GAP),
            info_overload_header(public, true, metrics),
            fixed_height(BAND_GAP),
            lower,
            fixed_height(BAND_GAP),
            bottom_edge,
            spacer(1.0),
        ],
    )
}
