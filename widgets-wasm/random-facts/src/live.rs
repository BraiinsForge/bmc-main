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

//! The entry points the host calls.

use bmc_wasm_sdk::*;

use crate::render::{fact_draw, header_draw};
use crate::source::{FactSource, Facts};

/// Shown only until the first fact arrives.
const LOADING_TEXT: &str = "Loading...";

struct RandomFacts {
    _source: Facts,
    /// The last good fact, kept on screen across failed fetches.
    fact: Option<String>,
}

impl RandomFacts {
    fn on_fact(&mut self, fact: String) {
        self.fact = Some(fact);
        request_frame();
    }
}

#[widget]
impl Widget for RandomFacts {
    fn init(me: &Building<Self>) -> Self {
        Self {
            _source: FactSource::new(Notify::new(me, Self::on_fact)),
            fact: None,
        }
    }

    fn render(&mut self, _delta_ms: u32) {
        let ws = widget_size();

        let fact = self.fact.as_deref().unwrap_or(LOADING_TEXT);
        let root = col(
            props!(background: BLACK),
            vec![header_draw(ws), fact_draw(ws, fact)],
        );

        let _ = render_ui(ws.width, ws.height, root);
    }

    fn on_system_update(&mut self) {
        // Only `theme` is relevant; re-render so a theme change is reflected
        // without a re-spawn.
        request_frame();
    }
}
