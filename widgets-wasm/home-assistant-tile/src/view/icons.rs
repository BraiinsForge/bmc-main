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

//! The icons the view draws itself, whatever entity the tile shows, and those more than one group draws.

use bmc_wasm_sdk::{Svg, include_svg};

pub const ARROW_LEFT: Svg = include_svg!("assets/mdi/arrow-left.svg");
pub const ARROW_RIGHT: Svg = include_svg!("assets/mdi/arrow-right.svg");
pub const BACKSPACE_OUTLINE: Svg = include_svg!("assets/mdi/backspace-outline.svg");
pub const CHECK: Svg = include_svg!("assets/mdi/check.svg");
pub const CLOSE: Svg = include_svg!("assets/mdi/close.svg");
pub const POWER_PLUG: Svg = include_svg!("assets/mdi/power-plug.svg");
pub const POWER_PLUG_OFF: Svg = include_svg!("assets/mdi/power-plug-off.svg");
