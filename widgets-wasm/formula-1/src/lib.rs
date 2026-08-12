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

//! Formula 1 widget — championship standings, driver stats,
//! race info, and live timing.
//!
//! These layouts port the `deckfeeder` widget of the same name,
//! which rendered the screens server-side as images.
//! Its CSS is the source for any rule given here as one the port keeps.

pub mod api;
pub mod model;
pub mod screens;

mod manifest_params;

#[cfg(target_arch = "wasm32")]
mod live;
#[cfg(target_arch = "wasm32")]
mod parse;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    #[expect(
        clippy::wildcard_imports,
        reason = "widget render code uses many SDK exports and macros in one file"
    )]
    use bmc_wasm_sdk::*;

    use crate::live;
    use crate::manifest_params::Params;

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        live::start();
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let standings = live::with_data(|data| data.standings.len());
        let root = col(
            props!(background: BLACK),
            [center(
                props!(flex: 1.0),
                [col(
                    props!(gap: 12.0, cross_align: CrossAlign::Center),
                    [
                        text("Formula 1", style!(size: 28, color: WHITE)),
                        text(
                            fmt!("{} drivers", standings),
                            style!(size: 16, color: GRAY_60),
                        ),
                    ],
                )],
            )],
        );
        let _ = render_ui(ws.width, ws.height, root);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let changed = Params::previous()
            .map(|previous| Params::current().changed_keys(&previous))
            .unwrap_or_default();
        live::reconcile();
        if changed.contains(&"driver") {
            live::invalidate_driver();
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        request_frame();
    }
}
