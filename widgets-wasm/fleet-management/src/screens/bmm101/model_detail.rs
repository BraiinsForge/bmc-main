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
    Bmm101, ROW_FONT, back_header, data_row, frame, head_cell, head_row, name_cell, paged_table,
    spark_cell, value_cell,
};
use crate::screens::model_detail::{DeviceRow, ModelDetailViewData};
use crate::screens::parts::{UNAVAILABLE, icon, pager, row_tap, si_parts_or_dash, status_glyph};
use crate::summary::DeviceStatus;
use crate::view::PagerScope;

// Column widths at the design size; they fill the card between its row padding.
const COL_HOST: f32 = 120.0;
const COL_HASHRATE: f32 = 86.0;
const COL_SPARK: f32 = 70.0;
const COL_EFF: f32 = 98.0;

#[must_use]
pub fn model_detail_view(m: Bmm101, data: &ModelDetailViewData) -> Node {
    let head = head_row(
        m,
        vec![
            head_cell(m, COL_HOST, "Hostname"),
            head_cell(m, COL_HASHRATE, "Hashrate"),
            head_cell(m, COL_SPARK, ""),
            head_cell(m, COL_EFF, "Eff."),
        ],
    );
    let rows = data.rows.iter().map(|r| device_row(m, r, data)).collect();
    frame(
        m,
        back_header(m, &data.title, Some(data.device_count)),
        paged_table(
            m,
            head,
            rows,
            pager(
                PagerScope::ModelDetail,
                data.page,
                data.page_count,
                m.pager(),
            ),
        ),
    )
}

fn device_row(m: Bmm101, r: &DeviceRow, data: &ModelDetailViewData) -> Node {
    let mut cells = vec![name_cell(m, COL_HOST, &r.hostname, FontWeight::SEMIBOLD)];
    // A device with no live telemetry shows why, where its metrics would be.
    match r.status {
        DeviceStatus::Ok | DeviceStatus::Degraded => {
            let (hashrate, unit) = si_parts_or_dash(r.hashrate.map(|h| h.format_si_parts(3)));
            cells.extend([
                value_cell(m, COL_HASHRATE, &hashrate, &unit),
                spark_cell(m, COL_SPARK, &r.series, data.window),
                value_cell(
                    m,
                    COL_EFF,
                    &r.efficiency
                        .map_or_else(|| UNAVAILABLE.to_owned(), |e| e.format_value(1)),
                    r.efficiency.map_or("", |_| MiningEfficiency::UNIT),
                ),
            ]);
        }
        DeviceStatus::Unreachable
        | DeviceStatus::ApiError
        | DeviceStatus::AuthError
        | DeviceStatus::AccountUnusable => cells.push(status_banner(m, r.status)),
    }
    cells.push(row_tap(&r.click_id));
    data_row(m, cells)
}

fn status_banner(m: Bmm101, status: DeviceStatus) -> Node {
    let (svg, color, label) = status_glyph(status);
    row(
        props!(flex: 1.0, gap: m.px(8.0), cross_align: CrossAlign::Center),
        [
            icon(svg, m.px(16.0), color),
            text(
                label,
                style!(size: m.font(ROW_FONT), weight: FontWeight::SEMIBOLD, color: color),
            ),
        ],
    )
}
