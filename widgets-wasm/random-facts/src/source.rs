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

//! Fetches a random fact every fifteen minutes.

use bmc_wasm_sdk::*;

const FACTS_API_URL: &str = "https://nexus.braiinsforge.com/api/v1/data/facts/random";
const FACTS_TEXT_PATH: &str = "/data/text";

/// Steady-state cadence: one fact every 15 minutes per instance.
const POLL_INTERVAL_MS: u32 = 900_000;

pub(super) type Facts = Part<FactSource>;

pub(super) struct FactSource {
    _poll: Poll,
    on_fact: Notify<String>,
}

impl FactSource {
    /// `on_fact` gets the fact of each usable reply; a failed one sends nothing.
    pub(super) fn new(on_fact: Notify<String>) -> Part<Self> {
        Part::new_cyclic(|me| Self {
            _poll: Poll::register(
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
    fn build_request(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        Some(FetchSpec::get(FACTS_API_URL))
    }

    fn on_fact_data(&mut self, _handle: &Poll, response: &FetchResponse) -> Verdict {
        let fact = response.ok().then(|| fact_in(&response.json())).flatten();

        if let Some(fact) = fact {
            self.on_fact.send(fact);
            Verdict::Accept
        } else {
            if response.ok() {
                log_warn!("random-facts: response had no usable {FACTS_TEXT_PATH}");
            } else {
                log_warn!("random-facts: fetch failed (status {})", response.status);
            }

            // Fast retry; forces `retry_ms` even on an HTTP-ok-but-unusable
            // reply.
            Verdict::Retry
        }
    }
}

/// The fact a reply carries, or `None` when it has no usable text.
fn fact_in(json: &JsonDoc) -> Option<String> {
    json.str(FACTS_TEXT_PATH).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::{JsonDoc, wasm_test};

    use super::fact_in;

    #[wasm_test]
    fn a_reply_carries_its_fact() {
        assert_eq!(
            fact_in(&JsonDoc::parse(
                br#"{"data":{"text":"Honey never spoils."}}"#
            ))
            .as_deref(),
            Some("Honey never spoils.")
        );
    }

    #[wasm_test]
    fn an_empty_fact_is_no_fact() {
        assert_eq!(
            fact_in(&JsonDoc::parse(br#"{"data":{"text":""}}"#)),
            None,
            "an empty fact would blank the card that shows the last good one"
        );
    }

    #[wasm_test]
    fn a_reply_without_the_text_is_no_fact() {
        assert_eq!(
            fact_in(&JsonDoc::parse(
                br#"{"data":{"fact":"Honey never spoils."}}"#
            )),
            None,
            "a renamed field reads as a failed reply, which retries soon"
        );
    }
}
