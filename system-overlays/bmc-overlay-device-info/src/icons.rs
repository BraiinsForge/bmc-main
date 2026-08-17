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

//! Compiles and registers the embedded device-info icons into a renderer
//! and returns the resulting icon-id handles for use when building the UI tree.
//! The SVGs are the stable-26.02 `init_setup` set,
//! carried over so the screens keep the legacy design.

use bmc_render::renderer::Renderer;
use bmc_render_macros::include_svg;
use bmc_system_overlay::register_icon;
use bmc_wasm_protocol::SvgId;
use bmc_wasm_sdk::assets::Svg;

const WIFI: Svg = include_svg!("assets/wifi.svg");
const WIFI_CONNECT: Svg = include_svg!("assets/wifi_connect.svg");
const WIFI_ERROR: Svg = include_svg!("assets/wifi_error.svg");
const SUCCESS: Svg = include_svg!("assets/success.svg");
const REFRESH: Svg = include_svg!("assets/refresh.svg");
const DESKTOP_CLOCK: Svg = include_svg!("assets/desktop_clock.svg");

/// A registered icon and the viewBox it was authored at.
/// The host scales X and Y independently,
/// so a screen that ignores the viewBox stretches the glyph.
#[derive(Debug, Clone, Copy, Default)]
pub struct Icon {
    pub id: Option<SvgId>,
    pub size: (f32, f32),
}

/// All icon handles the device-info screens render with.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceInfoIcons {
    pub wifi: Icon,
    pub wifi_connect: Icon,
    pub wifi_error: Icon,
    pub success: Icon,
    pub refresh: Icon,
    pub desktop_clock: Icon,
}

#[must_use]
pub fn register_icons(renderer: &mut dyn Renderer) -> DeviceInfoIcons {
    // Registration returning None (parse failure or ID exhaustion)
    // leaves that icon's Option unset and the slot renders empty.
    let mut reg = |icon: &Svg| -> Icon {
        Icon {
            id: register_icon(icon.name, || {
                renderer.register_svg(icon.name, icon.source.data())
            }),
            size: icon.viewbox(),
        }
    };
    DeviceInfoIcons {
        wifi: reg(&WIFI),
        wifi_connect: reg(&WIFI_CONNECT),
        wifi_error: reg(&WIFI_ERROR),
        success: reg(&SUCCESS),
        refresh: reg(&REFRESH),
        desktop_clock: reg(&DESKTOP_CLOCK),
    }
}
