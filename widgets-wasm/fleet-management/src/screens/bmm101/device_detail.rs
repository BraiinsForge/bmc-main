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

//! One miner on BMM101: address, state, hashrate trend and efficiency as a 2×2 grid.
//! Nominal hashrate, power, uptime and temperature stay Deck-only.

use bmc_wasm_sdk::types::MiningEfficiency;
#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::screens::bmm101::{
    Bmm101, LABEL_FONT, TILE_GAP, VALUE_FONT, back_header, frame, tile, tile_label, value_row,
};
use crate::screens::device_detail::{DeviceDetailData, nominal_ths};
use crate::screens::parts::{
    BORDER, CARD_BG, LABEL, UNAVAILABLE, icon, scaled_area_chart, status_glyph,
};
use crate::summary::DeviceStatus;

const STATE_ICON: f32 = 22.0;

#[must_use]
pub fn device_detail_view(m: Bmm101, d: &DeviceDetailData) -> Node {
    let gap = m.px(TILE_GAP);
    let tile_w = (m.body_width() - gap) / 2.0;
    let tile_h = (m.body_height() - gap) / 2.0;
    let efficiency = d
        .efficiency
        .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(2));
    frame(
        m,
        back_header(m, &fmt!("{} - {}", d.model, d.hostname), None),
        col(
            props!(gap: gap),
            [
                row(
                    props!(gap: gap),
                    [
                        ip_mac_tile(m, tile_w, tile_h, &d.ip, d.mac.as_deref()),
                        state_tile(m, tile_w, tile_h, d.state),
                    ],
                ),
                row(
                    props!(gap: gap),
                    [
                        hashrate_tile(m, tile_w, tile_h, d),
                        tile(
                            m,
                            tile_w,
                            tile_h,
                            vec![
                                tile_label(m, "Efficiency"),
                                value_row(
                                    m,
                                    &efficiency,
                                    d.efficiency.map_or("", |_| MiningEfficiency::UNIT),
                                ),
                            ],
                        ),
                    ],
                ),
            ],
        ),
    )
}

fn ip_mac_tile(m: Bmm101, w: f32, h: f32, ip: &str, mac: Option<&str>) -> Node {
    let line = |s: &str| {
        text(
            s,
            style!(size: m.font(LABEL_FONT), weight: FontWeight::SEMIBOLD, color: WHITE),
        )
    };
    tile(
        m,
        w,
        h,
        vec![
            tile_label(m, "IP/MAC"),
            col(
                props!(gap: m.px(4.0), cross_align: CrossAlign::Center),
                [line(ip), line(mac.unwrap_or(UNAVAILABLE))],
            ),
        ],
    )
}

fn state_tile(m: Bmm101, w: f32, h: f32, state: DeviceStatus) -> Node {
    let (svg, color, label) = status_glyph(state);
    tile(
        m,
        w,
        h,
        vec![
            tile_label(m, "State"),
            row(
                props!(gap: m.px(8.0), cross_align: CrossAlign::Center),
                [
                    icon(svg, m.px(STATE_ICON), color),
                    text(
                        label,
                        style!(size: m.font(VALUE_FONT), weight: FontWeight::SEMIBOLD, color: WHITE),
                    ),
                ],
            ),
        ],
    )
}

// The hashrate trend filling its tile, anchored to the nameplate, with the figure over it.
fn hashrate_tile(m: Bmm101, w: f32, h: f32, d: &DeviceDetailData) -> Node {
    let (chart_w, chart_h) = (w - 2.0, h - 2.0);
    let mut draws = scaled_area_chart(
        &d.hashrate_series,
        d.window,
        chart_w,
        chart_h,
        0.42,
        nominal_ths(d.nominal_hashrate),
        false,
    );
    draws.push(Draw::text(
        chart_w / 2.0,
        chart_h * 0.38,
        "Hashrate",
        style!(size: m.font(LABEL_FONT), color: LABEL, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    draws.push(Draw::text(
        chart_w / 2.0,
        chart_h * 0.64,
        d.hashrate
            .map_or_else(|| UNAVAILABLE.to_owned(), |h| h.format_si(3)),
        style!(size: m.font(VALUE_FONT), weight: FontWeight::SEMIBOLD, color: WHITE, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    col(
        props!(background: BORDER, width: w, height: h, padding: 1.0),
        [col(
            props!(background: CARD_BG, flex: 1.0),
            [canvas(props!(width: chart_w, height: chart_h), draws)],
        )],
    )
}
