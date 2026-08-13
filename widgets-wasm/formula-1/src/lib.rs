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
pub mod images;
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
    use crate::screens::driver::DriverViewData;
    use crate::screens::next_race::NextRaceViewData;
    use crate::screens::standings::StandingsViewData;

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        live::start();
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let bucket = crate::model::size_bucket(ws.width, ws.height);
        let view = Params::current().view;
        let root = live::with_data(|data| match crate::api::select_screen(view, data) {
            crate::api::Screen::NextRace => {
                crate::screens::next_race::next_race_view(&NextRaceViewData {
                    bucket,
                    race: data.next_race.clone(),
                })
            }
            crate::api::Screen::Standings => {
                crate::screens::standings::standings_view(&StandingsViewData {
                    bucket,
                    rows: data.standings.clone(),
                })
            }
            crate::api::Screen::Driver => crate::screens::driver::driver_view(&DriverViewData {
                bucket,
                driver: data.selected_driver_stats().cloned(),
            }),
        });
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

    /// Dormancy invalidates bitmap ids, leaving the image memo dead;
    /// the next render restores from the cache instead.
    #[unsafe(no_mangle)]
    pub extern "C" fn on_wake() {
        crate::images::invalidate_all();
        request_frame();
    }
}
