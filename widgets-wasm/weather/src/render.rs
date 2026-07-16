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

pub mod bar;
pub mod common;
pub mod full;
pub mod icons;
pub mod large;
pub mod medium;
pub mod small;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::{manifest_params::Params, model::Weather};

#[must_use]
pub fn current_view(weather: &Weather, params: &Params, size: WidgetSize) -> Node {
    match size.variant {
        SizeVariant::Full => full::full(weather, params, size),
        SizeVariant::Large => large::large(weather, params, size),
        SizeVariant::Medium => medium::medium(weather, params, size),
        SizeVariant::Small => small::small(weather, params, size),
    }
}

#[must_use]
pub fn message_view(message: &str, _size: WidgetSize) -> Node {
    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [common::txt(
                message.to_string(),
                32,
                FontWeight::REGULAR,
                GRAY_30,
            )],
        )],
    )
}
