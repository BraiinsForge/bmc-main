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

//! The Geek face on the rectangular panels: one miner's readings beside the
//! BTC price, in the titled list the BMM101 frame draws; the small panel gets
//! the same list at its own type size.

use bmc_wasm_sdk::Node;

use super::{icons, ip_line, text_line, titled_lines};
use crate::format;
use crate::layout::{self, Panel};
use crate::model::{MinerData, PublicData};

#[must_use]
pub fn geek(panel: Panel, miner: &MinerData, public: &PublicData) -> Node {
    let sizes = layout::list_layout(panel).text;
    titled_lines(
        panel,
        &icons::GEEK,
        "Miner Info - Geek",
        vec![
            text_line("Current Hashrate", format::fixed(miner.hashrate, 2), sizes),
            text_line("Temperature", format::temperature(miner.temperature), sizes),
            text_line("Power Consumption", format::fixed(miner.power, 0), sizes),
            text_line("Miner Uptime", format::uptime(miner.uptime), sizes),
            ip_line(miner, sizes),
            text_line("BTC Price", format::money(public.btc_price, 0), sizes),
        ],
    )
}
