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

//! One model's devices on BMM101: hostname, hashrate, its trend and efficiency.
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
    Bmm101, ROW_FONT, back_header, frame, head, name_text, paged, sparkline, value_text,
};
use crate::screens::model_detail::{DeviceRow, ModelDetailViewData};
use crate::screens::parts::{
    Cell, Column, Fit, UNAVAILABLE, column_table, icon, pager, si_parts_or_dash, status_glyph,
};
use crate::summary::DeviceStatus;
use crate::view::PagerScope;

#[must_use]
pub fn model_detail_view(m: Bmm101, data: &ModelDetailViewData) -> Node {
    let host = |r: &DeviceRow| Cell::Node(name_text(m, &r.hostname, FontWeight::SEMIBOLD));
    // A device with no live telemetry shows why where its hashrate would be.
    // The banner widens the column rather than spilling over the ones after it,
    // which a page of nothing but errors would push off the card.
    let hashrate = |r: &DeviceRow| {
        Cell::Node(if r.delivering() {
            let (v, u) = si_parts_or_dash(r.hashrate.map(|h| h.format_si_parts(3)));
            value_text(m, &v, &u)
        } else {
            status_banner(m, r.status)
        })
    };
    let spark = |r: &DeviceRow| r.figures(|| sparkline(m, &r.series, data.window));
    let efficiency = |r: &DeviceRow| {
        r.figures(|| {
            value_text(
                m,
                &r.efficiency
                    .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(1)),
                r.efficiency.map_or("", |_| MiningEfficiency::UNIT),
            )
        })
    };
    let columns = vec![
        Column::single(head(m, "Hostname"), Fit::Grow, &host),
        Column::single(head(m, "Hashrate"), Fit::End, &hashrate),
        Column::single(head(m, ""), Fit::Start, &spark),
        Column::single(head(m, "Eff."), Fit::End, &efficiency),
    ];
    frame(
        m,
        back_header(m, &data.title, Some(data.device_count)),
        paged(
            m,
            column_table(
                &data.rows,
                columns,
                |r| r.click_id.clone(),
                m.table_width(),
                m.table(),
            ),
            pager(
                PagerScope::ModelDetail,
                data.page,
                data.page_count,
                m.pager(),
            ),
        ),
    )
}

fn status_banner(m: Bmm101, status: DeviceStatus) -> Node {
    let (svg, color, label) = status_glyph(status);
    row(
        props!(gap: m.px(8.0), cross_align: CrossAlign::Center),
        [
            icon(svg, m.px(16.0), color),
            text(
                label,
                style!(size: m.font(ROW_FONT), weight: FontWeight::SEMIBOLD, color: color),
            ),
        ],
    )
}
