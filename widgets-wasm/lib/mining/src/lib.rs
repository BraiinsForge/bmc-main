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

//! Shared mining-gauge model and color palette for the mining widgets.
//!
//! `gauge` holds the state classification and tick geometry (no SDK types, so
//! it unit-tests on the host); `style` holds the palette and the per-state ring
//! fill. Both `mining-info` and `mining-clock` build their gauges from here.
//! `hashboards` holds the JSON lookup trait and chip-summary fold shared by
//! `mining-info` and `fleet-management`.

pub mod gauge;
pub mod hashboards;
pub mod overlay;
pub mod style;
