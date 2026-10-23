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

//! Grid view on BMM101: fleet status and efficiency over a full-bleed hashrate chart.
//! Power and temperatures stay Deck-only; the panel has no room for them.

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
    Bmm101, EDGE, HEADER_GAP, LABEL_FONT, TILE_GAP, VALUE_FONT, fleet_header, tile, tile_label,
    value_row,
};
use crate::screens::dashboard::DashboardViewData;
use crate::screens::parts::{
    CountSlot, CountStyle, LABEL, UNAVAILABLE, auth_failures, scaled_area_chart, status_count_font,
    status_counts,
};

const TILES_H: f32 = 100.0;
const STATUS_W: f32 = 262.0;
const STATUS_ICON: f32 = 22.0;

#[must_use]
pub fn dashboard_view(m: Bmm101, data: &DashboardViewData) -> Node {
    // The chart bleeds to the panel's edges, so only the header and tiles sit inside the edge.
    let top = col(
        props!(padding: m.px(EDGE), gap: m.px(HEADER_GAP)),
        [
            fleet_header(m, &data.title, data.device_count, false),
            row(
                props!(gap: m.px(TILE_GAP)),
                [fleet_status(m, data), efficiency(m, data)],
            ),
        ],
    );
    col(
        props!(background: BLACK, width: m.width(), height: m.height()),
        [top, hashrate(m, data)],
    )
}

fn fleet_status(m: Bmm101, data: &DashboardViewData) -> Node {
    let mut children = vec![
        tile_label(m, "Fleet Status"),
        status_counts(
            data.ok,
            data.degraded,
            data.off,
            CountStyle {
                icon: m.px(STATUS_ICON),
                font: m.font(status_count_font(
                    data.ok,
                    data.degraded,
                    data.off,
                    VALUE_FONT,
                    18,
                )),
                weight: FontWeight::SEMIBOLD,
                slot: CountSlot::Fit,
                gap: m.px(6.0),
            },
            m.px(16.0),
        ),
    ];
    if data.auth > 0 {
        children.push(auth_failures(data.auth, m.px(16.0), m.font(14)));
    }
    tile(m, m.px(STATUS_W), m.px(TILES_H), children)
}

fn efficiency(m: Bmm101, data: &DashboardViewData) -> Node {
    let width = m.body_width() - m.px(STATUS_W) - m.px(TILE_GAP);
    tile(
        m,
        width,
        m.px(TILES_H),
        vec![
            tile_label(m, "Efficiency"),
            value_row(
                m,
                &data
                    .efficiency
                    .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(2)),
                data.efficiency.map_or("", |_| MiningEfficiency::UNIT),
            ),
        ],
    )
}

// The fleet hashrate filling the rest of the panel, its figure drawn over the fill.
fn hashrate(m: Bmm101, data: &DashboardViewData) -> Node {
    let w = m.width();
    let h = m.body_height() - m.px(TILES_H);
    // Little headroom: the line rides high, so both texts sit inside the fill.
    let mut draws = scaled_area_chart(&data.hashrate_series, data.window, w, h, 0.12, None, false);
    draws.push(Draw::text(
        w / 2.0,
        h * 0.45,
        "Hashrate",
        style!(size: m.font(LABEL_FONT), color: LABEL, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    draws.push(Draw::text(
        w / 2.0,
        h * 0.72,
        data.hashrate
            .map_or_else(|| UNAVAILABLE.to_owned(), |h| h.format_si(3)),
        style!(size: m.font(VALUE_FONT), weight: FontWeight::SEMIBOLD, color: WHITE, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    canvas(props!(width: w, height: h), draws)
}
