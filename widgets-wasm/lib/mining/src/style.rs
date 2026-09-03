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

use bmc_wasm_sdk::{ArcFill, Color};

use crate::gauge::GaugeState;

pub const INACTIVE_TICK: Color = Color::from_rgb(0x1e, 0x1e, 0x1e);
pub const OFF_TICK: Color = Color::from_rgb(0xd9, 0x22, 0x2c);
pub const OFF_LABEL: Color = Color::from_rgb(0xf9, 0x53, 0x55);
pub const AMBER_DARK: Color = Color::from_rgb(0xcf, 0x79, 0x0e);
pub const AMBER_BRIGHT: Color = Color::from_rgb(0xfe, 0xba, 0x53);
pub const AMBER_LABEL: Color = Color::from_rgb(0xfe, 0xba, 0x53);
pub const GREEN_DARK: Color = Color::from_rgb(0x19, 0x5e, 0x33);
pub const GREEN_BRIGHT: Color = Color::from_rgb(0x5a, 0xdf, 0x88);
pub const GREEN_LABEL: Color = Color::from_rgb(0x34, 0xc0, 0x6a);
pub const PURPLE: Color = Color::from_rgb(0x8b, 0x7c, 0xff);

// `None` for `NotAvailable`, which renders neutral.
#[must_use]
pub const fn ring_fill(state: GaugeState) -> Option<ArcFill> {
    match state {
        GaugeState::NotAvailable => None,
        GaugeState::Off => Some(ArcFill::Solid(OFF_TICK)),
        GaugeState::Underclocked => Some(ArcFill::gradient(AMBER_DARK, AMBER_BRIGHT)),
        GaugeState::Good => Some(ArcFill::gradient(GREEN_DARK, GREEN_BRIGHT)),
        GaugeState::Overclocked => Some(ArcFill::Solid(PURPLE)),
    }
}
