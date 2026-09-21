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

//! Miner Info - Info Overload: one miner's readings
//! beside the whole Bitcoin-network picture,
//! so it polls the miner and every public endpoint.

mod manifest_params;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;
#[cfg(target_arch = "wasm32")]
use manifest_params::credentials as slots;
#[cfg(target_arch = "wasm32")]
use miner_info::engine;
#[cfg(target_arch = "wasm32")]
use miner_info::face;
#[cfg(target_arch = "wasm32")]
use miner_info::layout::{self, Panel};
#[cfg(target_arch = "wasm32")]
use mining::bos::{AuthMode, Placeholders};

#[cfg(target_arch = "wasm32")]
fn auth_mode() -> AuthMode {
    let bound = credentials::current();
    let local_bound = bound.is_bound("bos_local");
    let remote_bound = bound.is_bound("bos_remote");
    AuthMode::derive(
        local_bound,
        remote_bound,
        &manifest_params::Params::current().miner_url,
        Placeholders {
            token: slots::bos_local::TOKEN,
            username: slots::bos_remote::USERNAME,
            password: slots::bos_remote::PASSWORD,
        },
    )
}

#[cfg(target_arch = "wasm32")]
fn config() -> engine::Config {
    engine::Config {
        view: engine::View::InfoOverload,
        auth: auth_mode(),
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    engine::init(config);
}

// Absent a previous snapshot the URL counts as moved,
// so remote mode re-authenticates on the first update.
// The quoted currency is a build-time constant rather than a param,
// so no update can move it.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let previous = manifest_params::Params::previous();
    let miner_url = previous.as_ref().is_none_or(|previous| {
        manifest_params::Params::current()
            .changed_keys(previous)
            .contains(&"miner_url")
    });
    engine::on_params_update(engine::Changed {
        miner_url,
        currency: false,
    });
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_credentials_update() {
    engine::on_credentials_update();
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
    let (miner, public, auth) = engine::frame();
    let panel = layout::classify(viewport);
    let root = match panel {
        Panel::Round => face::round::info_overload(&miner, &public),
        Panel::Bmm101 => face::bmm101::info_overload(&miner, &public),
        Panel::Small => face::info_overload(&miner, &public),
    };
    let overlay = engine::overlay(engine::View::InfoOverload, panel, &auth);
    let root = mining::overlay::apply_overlay(root, overlay, viewport.shape);
    let _ = render_ui(viewport.width, viewport.height, root);
}
