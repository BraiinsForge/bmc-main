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

use crate::{
    display,
    render::common::{self, TEXT_PRIMARY, TEXT_SECONDARY},
    weather_code,
};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

#[must_use]
pub fn small(
    weather: &crate::model::Weather,
    _params: &crate::manifest_params::Params,
    _size: WidgetSize,
) -> Node {
    let current = weather.current.as_ref();

    let mut stack: Vec<Node> = Vec::new();
    if let Some(c) = current {
        stack.push(common::weather_icon(
            weather_code::icon_id(c.weather_code, c.is_day),
            96.0,
        ));
    }
    stack.push(common::txt(
        display::temperature_or_placeholder(current.map(|c| c.temperature), display::temperature),
        48,
        FontWeight::BOLD,
        TEXT_PRIMARY,
    ));
    stack.push(common::txt(
        current.map_or_else(
            || display::NOT_AVAILABLE.to_string(),
            |c| weather_code::description(c.weather_code).to_string(),
        ),
        20,
        FontWeight::REGULAR,
        TEXT_SECONDARY,
    ));
    stack.push(common::txt(
        weather.location.display_name.clone(),
        16,
        FontWeight::REGULAR,
        TEXT_SECONDARY,
    ));

    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [col(
                props!(cross_align: CrossAlign::Center, gap: 8.0),
                stack,
            )],
        )],
    )
}
