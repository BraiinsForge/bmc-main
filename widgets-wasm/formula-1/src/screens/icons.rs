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

//! The widget's own chrome, compiled in.
//!
//! Nothing a payload points at rides here: the constructor marks are
//! fetched like the headshots and the flags, and a team the fetch has
//! not answered for yet draws in its livery colour.

use bmc_wasm_sdk::{Svg, include_svg};

/// The two swept bars trailing the header, from the design's brand mark.
pub const BRAND_STRIPE: Svg = include_svg!("assets/brand-stripe.svg");
