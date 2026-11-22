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

use crate::manifest_params;
use crate::render::{self, country_draw, date_draw};
use crate::source::{NamedayData, NamedaySource, Names, is_today};

const LOADING_TEXT: &str = "Loading...";
const NOT_APPLICABLE_TEXT: &str = "N/A";
const NOT_AVAILABLE_TEXT: &str = "--";

struct Nameday {
    source: Names,
    /// The last accepted day's names, kept on screen across failed fetches.
    names: Option<NamedayData>,
}

impl Nameday {
    fn on_names(&mut self, names: NamedayData) {
        self.names = Some(names);
        request_frame();
    }

    /// Shown names from a past day now get the stale pill.
    #[expect(clippy::unused_self, reason = "the SDK calls this as a method")]
    fn on_day_change(&mut self, (): ()) {
        request_frame();
    }
}

#[widget]
impl Widget for Nameday {
    fn init(me: &Building<Self>) -> Self {
        Self {
            source: NamedaySource::new(
                Notify::new(me, Self::on_names),
                Notify::new(me, Self::on_day_change),
            ),
            names: None,
        }
    }

    fn render(&mut self, _delta_ms: u32) {
        let ws = widget_size();
        let params = manifest_params::Params::current();
        let mut root_children: Vec<Node> = vec![country_draw(ws, params.country)];
        let mut stale_anchor: Option<SystemTime> = None;

        let names_string: &str = match self.names.as_ref() {
            Some(names) => names
                .names_for(params.country)
                .unwrap_or(NOT_APPLICABLE_TEXT),
            None => LOADING_TEXT,
        };

        root_children.push(render::names_draw(ws, names_string));

        if params.show_date {
            match self.names.as_ref() {
                Some(names) => {
                    let date_string = format_date(
                        names.fetched_at.time,
                        FormatDateOpts {
                            format: None,
                            timezone: Some(names.fetched_at.tz.clone()),
                        },
                    );

                    root_children.push(date_draw(ws, &date_string));

                    // Names for a past day are stale; flag with the shared pill.
                    if !is_today(&names.fetched_at) {
                        stale_anchor = Some(names.fetched_at.time);
                    }
                }
                None => {
                    root_children.push(date_draw(ws, NOT_AVAILABLE_TEXT));
                }
            }
        }

        let mut root = col(props!(background: BLACK), root_children);
        if let Some(anchor) = stale_anchor {
            root = with_stale_overlay(root, anchor, widget_viewport().shape);
        }

        let _ = render_ui(ws.width, ws.height, root);
    }

    fn on_params_update(&mut self) {
        // Names for all supported countries are cached in the state, so a country
        // change (or any other param change) only needs a re-render, never a fetch.
        request_frame();
    }

    fn on_system_update(&mut self) {
        // A new timezone may move today to another date.
        self.source.with_mut(NamedaySource::check_day);
        request_frame();
    }
}
