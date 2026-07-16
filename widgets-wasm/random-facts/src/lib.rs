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

mod render;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {

    use std::cell::RefCell;

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render code uses many SDK exports and macros in one file"
    )]
    use bmc_wasm_sdk::*;

    use crate::render::{fact_draw, header_draw};

    /// The viewbits API also accepts an optional `&key=...` query param; it is
    /// unused.
    const FACTS_API_URL: &str = "https://api.viewbits.com/v1/uselessfacts?mode=random";

    /// Steady-state cadence: one fact every 5 minutes per instance.
    const POLL_INTERVAL_MS: u32 = 300_000;

    /// Shown only until the first fact arrives; afterwards the last-known-good
    /// fact is kept on screen across failures.
    const LOADING_TEXT: &str = "Loading...";

    thread_local! {
        /// Last successfully fetched fact, kept as the fallback on later failures.
        static FACT: RefCell<Option<String>> = const { RefCell::new(None) };

        static POLL_HANDLE: RefCell<Option<PollHandle>> = const { RefCell::new(None) };
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "matches the SDK Build callback signature; this widget always fetches"
    )]
    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        Some(FetchSpec::get(FACTS_API_URL))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let handle = register_poll(
            build_request,
            on_fact_data,
            PollConfig {
                interval_ms: Some(POLL_INTERVAL_MS),
                ..Default::default()
            },
        );

        POLL_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
    }

    fn on_fact_data(handle: PollHandle, response: &FetchResponse) {
        // The response carries `text` (the fact) plus `source` / `url`, which we
        // ignore. Anything other than a non-empty `/text` on a 2xx response is a
        // failure: keep the last-known-good fact and retry.
        let fact = response
            .ok()
            .then(|| response.json().str("/text"))
            .flatten()
            .filter(|text| !text.is_empty());

        if let Some(fact) = fact {
            FACT.with(|f| *f.borrow_mut() = Some(fact));
            // Engine reschedules at the normal interval on an ok reply.
        } else {
            if response.ok() {
                log_warn!("random-facts: response had no usable /text");
            } else {
                log_warn!("random-facts: fetch failed (status {})", response.status);
            }

            // Fast retry; forces `retry_ms` even on an HTTP-ok-but-unusable
            // reply.
            handle.retry();
        }

        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();

        let fact = FACT.with(|f| f.borrow().clone());
        let fact = fact.as_deref().unwrap_or(LOADING_TEXT);

        let root = col(
            props!(background: BLACK),
            vec![header_draw(ws), fact_draw(ws, fact)],
        );

        let _ = render_ui(ws.width, ws.height, root);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        // Only `theme` is relevant; re-render so a theme change is reflected
        // without a re-spawn.
        request_frame();
    }
}
