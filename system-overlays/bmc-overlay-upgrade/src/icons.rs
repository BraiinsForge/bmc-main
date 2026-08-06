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

use bmc_render::renderer::Renderer;
use bmc_render_macros::include_svg;
use bmc_system_overlay::register_icon;
use bmc_wasm_protocol::SvgId;
use bmc_wasm_sdk::assets::Svg;

const TOOLS: Svg = include_svg!("assets/tools.svg");
const CHECKMARK: Svg = include_svg!("assets/checkmark.svg");
const ERROR: Svg = include_svg!("assets/error-circle.svg");

#[derive(Debug, Clone, Copy, Default)]
pub struct UpgradeIcons {
    pub tools: Option<SvgId>,
    pub checkmark: Option<SvgId>,
    pub error: Option<SvgId>,
}

#[must_use]
pub fn register_icons(renderer: &mut dyn Renderer) -> UpgradeIcons {
    let mut register =
        |icon: &Svg| register_icon(icon.name, || renderer.register_svg(icon.name, icon.data));
    UpgradeIcons {
        tools: register(&TOOLS),
        checkmark: register(&CHECKMARK),
        error: register(&ERROR),
    }
}
