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

//! List view on BMM101: each model's name, status counts, hashrate and its trend.
//! Power, efficiency and temperature stay Deck-only; the panel has no room for them.

use bmc_wasm_sdk::*;

use crate::screens::bmm101::{
    Bmm101, UNIT_FONT, fleet_header, frame, head, name_text, paged, sparkline, value_text,
};
use crate::screens::icons;
use crate::screens::parts::{
    Cell, Column, CountSlot, CountStyle, DEGRADED, Fit, OFF, OK, column_table, pager,
    si_parts_or_dash, status_item,
};
use crate::screens::table::{ModelRow, TableViewData};
use crate::view::{PagerScope, model_detail_click_id};

const STATUS_ICON: f32 = 14.0;
const STATUS_SPACING: f32 = 8.0;

#[must_use]
pub fn table_view(m: Bmm101, data: &TableViewData) -> Node {
    let count = CountStyle {
        icon: m.px(STATUS_ICON),
        font: m.font(UNIT_FONT),
        weight: FontWeight::REGULAR,
        slot: CountSlot::Fit,
        gap: m.px(4.0),
    };
    let name = |r: &ModelRow| Cell::Node(name_text(m, &r.name, FontWeight::BOLD));
    let ok = |r: &ModelRow| Cell::Node(status_item(&icons::PERF_OK, r.ok, OK, count));
    let degraded =
        |r: &ModelRow| Cell::Node(status_item(&icons::PERF_LOW, r.degraded, DEGRADED, count));
    let off = |r: &ModelRow| Cell::Node(status_item(&icons::PERF_OFF, r.off, OFF, count));
    let hashrate = |r: &ModelRow| {
        let (v, u) = si_parts_or_dash(r.hashrate.map(|h| h.format_si_parts(3)));
        Cell::Node(value_text(m, &v, &u))
    };
    let spark = |r: &ModelRow| Cell::Node(sparkline(m, &r.series, data.window));
    let states: [&dyn Fn(&ModelRow) -> Cell; 3] = [&ok, &degraded, &off];
    let columns = vec![
        Column::single(head(m, "Model"), Fit::Grow, &name),
        // One sub-column per state, so the icons line up down the rows.
        Column::Group {
            head: head(m, "State"),
            spacing: m.px(STATUS_SPACING),
            cells: &states,
        },
        Column::single(head(m, "Hashrate"), Fit::End, &hashrate),
        Column::single(head(m, ""), Fit::Start, &spark),
    ];
    frame(
        m,
        fleet_header(m, &data.title, data.device_count, true),
        paged(
            m,
            column_table(
                &data.rows,
                columns,
                |r| model_detail_click_id(r.family, &r.name),
                m.table_width(),
                m.table(),
            ),
            pager(PagerScope::Fleet, data.page, data.page_count, m.pager()),
        ),
    )
}
