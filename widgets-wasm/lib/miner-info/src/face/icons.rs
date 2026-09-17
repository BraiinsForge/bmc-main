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

//! The glyphs the faces draw.
//! A widget's icon is a copy of its crate's manifest `assets/icon.svg`,
//! which has to stay in the package the manifest names.

use bmc_wasm_sdk::{Svg, include_svg};

pub const CHIP: Svg = include_svg!("assets/chip.svg");
pub const GEEK: Svg = include_svg!("assets/geek.svg");
pub const INFO_OVERLOAD: Svg = include_svg!("assets/info-overload.svg");
pub const MINING: Svg = include_svg!("assets/mining.svg");
