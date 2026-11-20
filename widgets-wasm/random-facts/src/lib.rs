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

//! Random Facts widget, four sizes.

#[cfg(target_arch = "wasm32")]
mod render;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {

    use bmc_wasm_sdk::*;

    use crate::render::{fact_draw, header_draw};

    const FACTS_API_URL: &str = "https://nexus.braiinsforge.com/api/v1/data/facts/random";
    const FACTS_TEXT_PATH: &str = "/data/text";

    /// Steady-state cadence: one fact every 15 minutes per instance.
    const POLL_INTERVAL_MS: u32 = 900_000;

    /// Shown only until the first fact arrives; afterwards the last-known-good
    /// fact is kept on screen across failures.
    const LOADING_TEXT: &str = "Loading...";

    struct FactSource {
        _poll: PollHandle,
        on_fact: Notify<String>,
    }

    impl FactSource {
        /// `on_fact` gets the fact of each usable reply; a failed one sends nothing.
        fn new(on_fact: Notify<String>) -> Part<Self> {
            Part::new_cyclic(|me| Self {
                _poll: PollHandle::register(
                    me,
                    Self::build_request,
                    Self::on_fact_data,
                    PollConfig {
                        interval_ms: Some(POLL_INTERVAL_MS),
                        ..Default::default()
                    },
                ),
                on_fact,
            })
        }

        #[expect(clippy::unused_self, reason = "the SDK calls this as a method")]
        #[expect(
            clippy::unnecessary_wraps,
            reason = "the SDK Build callback returns Option"
        )]
        fn build_request(&mut self, _handle: &PollHandle) -> Option<FetchSpec> {
            Some(FetchSpec::get(FACTS_API_URL))
        }

        fn on_fact_data(&mut self, handle: &PollHandle, response: &FetchResponse) {
            let fact = response
                .ok()
                .then(|| response.json().str(FACTS_TEXT_PATH))
                .flatten()
                .filter(|text| !text.is_empty());

            if let Some(fact) = fact {
                self.on_fact.send(fact);
            } else {
                if response.ok() {
                    log_warn!("random-facts: response had no usable {FACTS_TEXT_PATH}");
                } else {
                    log_warn!("random-facts: fetch failed (status {})", response.status);
                }

                // Fast retry; forces `retry_ms` even on an HTTP-ok-but-unusable
                // reply.
                handle.retry();
            }
        }
    }

    struct RandomFacts {
        _source: Part<FactSource>,
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
}
