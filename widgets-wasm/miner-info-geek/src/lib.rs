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

//! Miner Info - Geek: one miner's own readings alongside the BTC price,
//! so it polls the miner and the Braiins public API.

mod manifest_params;

// `include_svg!` resolves against the crate hosting the file,
// so the asset lives with the widget
// and is handed to the faces that draw it.
#[cfg(target_arch = "wasm32")]
const CHIP_ICON: bmc_wasm_sdk::Svg = bmc_wasm_sdk::include_svg!("assets/chip.svg");

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;
#[cfg(target_arch = "wasm32")]
use miner_info::engine;
#[cfg(target_arch = "wasm32")]
use miner_info::face;
#[cfg(target_arch = "wasm32")]
use miner_info::face::RenderSize;

#[cfg(target_arch = "wasm32")]
fn config() -> engine::Config {
    let params = manifest_params::Params::current();
    engine::Config {
        view: engine::View::Geek,
        miner_url: params.miner_url,
        miner_password: params.miner_password,
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    engine::init(config);
}

// Absent a previous snapshot the credentials count as moved,
// so the first update authenticates.
// The quoted currency is a build-time constant rather than a param,
// so no update can move it.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let previous = manifest_params::Params::previous();
    let miner_credentials = previous.as_ref().is_none_or(|previous| {
        let keys = manifest_params::Params::current().changed_keys(previous);
        keys.contains(&"miner_url") || keys.contains(&"miner_password")
    });
    engine::on_params_update(engine::Changed {
        miner_credentials,
        currency: false,
    });
}

// Numbers are formatted from raw state on every render
// against the live `number_format` setting, so a frame request
// is enough to reflect a changed system setting promptly,
// instead of waiting for the next data refresh.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let viewport = widget_viewport();
    let size = RenderSize {
        width: viewport.width,
        height: viewport.height,
    };
    let (miner, public, auth) = engine::frame();
    // Only the round face draws a gauge, and it seeds from a single lit tick
    // so the host animates the real fill in from an empty-ish baseline.
    let seed_gauge = matches!(viewport.shape, ViewportShape::Round) && engine::take_first_frame();
    let root = match viewport.shape {
        ViewportShape::Round => face::round::geek(size, &miner, &public, seed_gauge, &CHIP_ICON),
        ViewportShape::Rectangular => face::geek(size, &miner, &public),
    };
    let overlay = engine::overlay(engine::View::Geek, &auth);
    let root = mining::overlay::apply_overlay(root, overlay, viewport.shape);
    let _ = render_ui(viewport.width, viewport.height, root);
    // The seeded frame is not the reading, so ask for the one that is.
    if seed_gauge {
        request_frame();
    }
}
