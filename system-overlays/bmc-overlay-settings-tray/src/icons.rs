// Copyright (C) 2026  Braiins Systems s.r.o.

//! Compiles and registers the embedded tray icons into a renderer and returns
//! the resulting icon-id handles for use when building the UI tree.

use bmc_render::renderer::Renderer;
use bmc_render_macros::include_svg;
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
    let mut reg = |icon: &Svg| -> Option<SvgId> { renderer.register_svg(icon.name, icon.data) };
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
        },
    }
}
