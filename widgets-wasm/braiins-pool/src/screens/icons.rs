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

//! Worker-state icons, shared artwork with the fleet-management widget.

use bmc_wasm_sdk::{Svg, include_svg};

/// Flat favicon-grade variant of the widget icon; the manifest's
/// `assets/icon.svg` keeps its gradients, which `include_svg!` drops.
pub const LOGO: Svg = include_svg!("assets/icon-simple.svg");

pub const WORKERS_ALL: Svg = include_svg!("assets/icons/workers-all.svg");
pub const WORKERS_OK: Svg = include_svg!("assets/icons/workers-ok.svg");
pub const WORKERS_LOW: Svg = include_svg!("assets/icons/workers-low.svg");
pub const WORKERS_OFF: Svg = include_svg!("assets/icons/workers-off.svg");
pub const PAYOUT_BTC: Svg = include_svg!("assets/icons/payout-btc.svg");
pub const PAYOUT_LN: Svg = include_svg!("assets/icons/payout-ln.svg");
