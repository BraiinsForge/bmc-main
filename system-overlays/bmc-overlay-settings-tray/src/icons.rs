// Copyright (C) 2026  Braiins Systems s.r.o.

//! Compiles and registers the embedded tray icons into a renderer and returns
//! the resulting icon-id handles for use when building the UI tree.

use bmc_render::renderer::Renderer;
use bmc_render_macros::include_svg;
use bmc_wasm_protocol::SvgId;
use bmc_wasm_sdk::assets::Svg;

use crate::ui::{ControlIcons, WifiIcons};

const PROBLEM: &str = include_str!("../assets/wifi/signal-problem.svg");
const LOW: &str = include_str!("../assets/wifi/signal-low.svg");
const FAIR: &str = include_str!("../assets/wifi/signal-fair.svg");
const STRONG: &str = include_str!("../assets/wifi/signal-strong.svg");

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
    // Control icons are compiled to their binary form at build time by
    // include_svg!; register_svg just takes the pre-compiled bytes. The WiFi
    // icons still carry raw XML and are compiled by compile_svg here. A
    // malformed asset is a build-quality failure, so panicking on these
    // vendored, known-good icons is acceptable.
    let wifi = {
        let mut reg = |tag: &str, svg: &str| -> Option<SvgId> {
            renderer.register_svg(tag, &bmc_svg_compiler::compile_svg(svg))
        };
        WifiIcons {
            problem: reg("wifi-problem", PROBLEM),
            low: reg("wifi-low", LOW),
            fair: reg("wifi-fair", FAIR),
            strong: reg("wifi-strong", STRONG),
        }
    };
    let controls = {
        let mut reg = |icon: &Svg| -> Option<SvgId> { renderer.register_svg(icon.name, icon.data) };
        ControlIcons {
            sound_low: reg(&SOUND_LOW),
            sound_high: reg(&SOUND_HIGH),
            brightness_low: reg(&BRIGHTNESS_LOW),
            brightness_high: reg(&BRIGHTNESS_HIGH),
        }
    };
    TrayIcons { wifi, controls }
}
