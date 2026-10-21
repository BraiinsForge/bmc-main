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

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::screens::bmm101::{
    Bmm101, UNIT_FONT, cell, data_row, fleet_header, frame, head_cell, head_row, name_cell,
    paged_table, spark_cell, value_cell,
};
use crate::screens::parts::{pager, row_tap, si_parts_or_dash, status_counts};
use crate::screens::table::{ModelRow, TableViewData};
use crate::view::{PagerScope, model_detail_click_id};

// Column widths at the design size; they fill the card between its row padding.
const COL_MODEL: f32 = 104.0;
const COL_STATUS: f32 = 120.0;
const COL_HASHRATE: f32 = 86.0;
const COL_SPARK: f32 = 64.0;
// The counts sit a size below the row text, so two digits keep clear of the next icon.
const STATUS_SLOT: f32 = 40.0;
const STATUS_ICON: f32 = 14.0;

#[must_use]
pub fn table_view(m: Bmm101, data: &TableViewData) -> Node {
    let head = head_row(
        m,
        vec![
            head_cell(m, COL_MODEL, "Model"),
            head_cell(m, COL_STATUS, "State"),
            head_cell(m, COL_HASHRATE, "Hashrate"),
        ],
    );
    let rows = data.rows.iter().map(|r| model_row(m, r, data)).collect();
    frame(
        m,
        fleet_header(m, &data.title, data.device_count, true),
        paged_table(
            m,
            head,
            rows,
            pager(PagerScope::Fleet, data.page, data.page_count, m.pager()),
        ),
    )
}

fn model_row(m: Bmm101, r: &ModelRow, data: &TableViewData) -> Node {
    let (hashrate, unit) = si_parts_or_dash(r.hashrate.map(|h| h.format_si_parts(3)));
    data_row(
        m,
        vec![
            name_cell(m, COL_MODEL, &r.name, FontWeight::BOLD),
            cell(
                m,
                COL_STATUS,
                status_counts(
                    r.ok,
                    r.degraded,
                    r.off,
                    m.px(STATUS_ICON),
                    m.font(UNIT_FONT),
                    m.px(STATUS_SLOT),
                    FontWeight::REGULAR,
                ),
            ),
            value_cell(m, COL_HASHRATE, &hashrate, &unit),
            spark_cell(m, COL_SPARK, &r.series, data.window),
            row_tap(&model_detail_click_id(r.family, &r.name)),
        ],
    )
}
