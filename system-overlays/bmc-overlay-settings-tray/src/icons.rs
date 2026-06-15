// Copyright (C) 2026  Braiins Systems s.r.o.

//! Compiles and registers the embedded Wi-Fi signal icons into a renderer and
//! returns the resulting icon-id handles for use when building the UI tree.

use bmc_render::renderer::Renderer;
use bmc_wasm_protocol::SvgId;

use crate::ui::WifiIcons;

const PROBLEM: &str = include_str!("../assets/wifi/signal-problem.svg");
const LOW: &str = include_str!("../assets/wifi/signal-low.svg");
const FAIR: &str = include_str!("../assets/wifi/signal-fair.svg");
const STRONG: &str = include_str!("../assets/wifi/signal-strong.svg");

#[must_use]
pub fn register_wifi_icons(renderer: &mut dyn Renderer) -> WifiIcons {
    // bmc_svg_compiler::compile_svg turns the SVG XML into the binary form
    // register_svg consumes. A malformed asset is a build-quality failure, so
    // panicking on these vendored, known-good icons is acceptable.
    let mut reg = |tag: &str, svg: &str| -> Option<SvgId> {
        renderer.register_svg(tag, &bmc_svg_compiler::compile_svg(svg))
    };
    WifiIcons {
        problem: reg("wifi-problem", PROBLEM),
        low: reg("wifi-low", LOW),
        fair: reg("wifi-fair", FAIR),
        strong: reg("wifi-strong", STRONG),
    }
}
