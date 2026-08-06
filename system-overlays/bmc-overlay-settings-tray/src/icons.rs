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

//! Compiles and registers the embedded tray icons into a renderer and returns
//! the resulting icon-id handles for use when building the UI tree.

use bmc_render::renderer::Renderer;
use bmc_render_macros::include_svg;
use bmc_system_overlay::register_icon;
use bmc_wasm_protocol::SvgId;
use bmc_wasm_sdk::assets::Svg;

use crate::ui::{ControlIcons, WifiIcons};

const PROBLEM: Svg = include_svg!("assets/wifi/signal-problem.svg");
const LOW: Svg = include_svg!("assets/wifi/signal-low.svg");
const FAIR: Svg = include_svg!("assets/wifi/signal-fair.svg");
const STRONG: Svg = include_svg!("assets/wifi/signal-strong.svg");

const SOUND_LOW: Svg = include_svg!("assets/controls/sound-low.svg");
const SOUND_HIGH: Svg = include_svg!("assets/controls/sound-high.svg");
const BRIGHTNESS_LOW: Svg = include_svg!("assets/controls/brightness-low.svg");
const BRIGHTNESS_HIGH: Svg = include_svg!("assets/controls/brightness-high.svg");
const NIGHT_MODE: Svg = include_svg!("assets/controls/nightmode.svg");
const RESTART: Svg = include_svg!("assets/controls/restart.svg");
const CLOSE: Svg = include_svg!("assets/controls/x.svg");

/// All icon handles the tray renders with.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrayIcons {
    pub wifi: WifiIcons,
    pub controls: ControlIcons,
}

#[must_use]
pub fn register_icons(renderer: &mut dyn Renderer) -> TrayIcons {
    // include_svg! compiles the SVG XML into the binary form register_svg
    // consumes at build time; here we only hand the pre-compiled bytes to the
    // renderer. Registration returning None (parse failure or ID exhaustion)
    // leaves that icon's Option unset and the slot renders empty.
    let mut reg = |icon: &Svg| -> Option<SvgId> {
        register_icon(icon.name, || renderer.register_svg(icon.name, icon.data))
    };
    TrayIcons {
        wifi: WifiIcons {
            problem: reg(&PROBLEM),
            low: reg(&LOW),
            fair: reg(&FAIR),
            strong: reg(&STRONG),
        },
        controls: ControlIcons {
            sound_low: reg(&SOUND_LOW),
            sound_high: reg(&SOUND_HIGH),
            brightness_low: reg(&BRIGHTNESS_LOW),
            brightness_high: reg(&BRIGHTNESS_HIGH),
            night_mode: reg(&NIGHT_MODE),
            night_mode_aspect: {
                let (w, h) = NIGHT_MODE.viewbox();
                w / h
            },
            restart: reg(&RESTART),
            close: reg(&CLOSE),
        },
    }
}
