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

//! The entry points the host calls: one render a second,
//! plus an immediate one after every params or system delivery.

use bmc_wasm_sdk::{
    Building, HostClock as _, SystemTime, Widget, render_ui, request_frame, request_frame_after,
    widget, widget_viewport,
};

use crate::manifest_params::Params;
use crate::model::ClockHandTransition;
use crate::screens::{HandAngles, ViewData, clock_view};

struct Clock {
    angles: HandAngles,
}

#[widget]
impl Widget for Clock {
    fn init(_: &Building<Self>) -> Self {
        Self {
            angles: HandAngles::default(),
        }
    }

    fn render(&mut self, delta_ms: u32) {
        let view = ViewData {
            now: SystemTime::now(),
            viewport: widget_viewport(),
            params: Params::current(),
            hand_transition: ClockHandTransition::for_render_gap(delta_ms),
        };
        let root = clock_view(&view, &mut self.angles);
        let _ = render_ui(view.viewport.width, view.viewport.height, root);
        // Re-render once per second so the displayed time advances.
        request_frame_after(1000);
    }

    /// An operator change must not wait for the next one-second tick.
    fn on_params_update(&mut self) {
        request_frame();
    }

    /// Same for a system delivery: a night-mode flip must not sit on screen
    /// for up to a second before the palette swaps.
    fn on_system_update(&mut self) {
        request_frame();
    }
}
