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

//! The entry points the host calls: the launch poll, and a render of what it holds.

use std::cell::{Cell, RefCell};

#[expect(clippy::wildcard_imports, reason = "widget glue uses many SDK exports")]
use bmc_wasm_sdk::*;

use crate::api::{self, Outcome, Reply};
use crate::model::{LaunchData, State};
use crate::screens::{ViewData, launch_view};

/// Fixed 5-min refresh; a fixed period keeps Decks from refreshing in lockstep.
const REFRESH_MS: u32 = 300_000;
const RETRY_MS: u32 = 30_000;
/// Countdown re-renders once a second.
const TICK_MS: u32 = 1_000;

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
    static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let handle = register_poll(
        build_request,
        on_launch,
        PollConfig {
            interval_ms: Some(REFRESH_MS),
            retry_ms: RETRY_MS,
            ..Default::default()
        },
    );
    POLL.with(|p| p.set(Some(handle)));
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the SDK Build callback signature; this widget always fetches"
)]
fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
    Some(FetchSpec::get(api::NEXUS_URL))
}

fn on_launch(handle: PollHandle, response: &FetchResponse) {
    let reply = if response.ok() {
        match LaunchData::parse(&response.json()) {
            Ok(Some(data)) => Reply::Data(data),
            Ok(None) => Reply::Empty,
            Err(_) => Reply::Error,
        }
    } else {
        log_warn!("spacex: fetch failed (status {})", response.status);
        Reply::Error
    };
    let has_data = STATE.with(|s| matches!(&*s.borrow(), State::Loaded(_)));

    match api::outcome(reply, has_data) {
        Outcome::Store(data) => {
            STATE.with(|s| *s.borrow_mut() = State::Loaded(data));
        }
        Outcome::NoLaunch => {
            STATE.with(|s| *s.borrow_mut() = State::NoLaunch);
        }
        Outcome::Keep => {
            // Retry a malformed 2xx sooner; a network failure waits for the engine.
            if response.ok() {
                handle.retry();
            }
        }
        Outcome::Fail => {
            let msg = if response.status == 0 {
                String::from("Network error")
            } else if response.ok() {
                // Malformed 2xx, not nexus's valid "no upcoming launch".
                String::from("Could not read launch data")
            } else {
                fmt!("API request failed ({})", response.status)
            };
            STATE.with(|s| *s.borrow_mut() = State::Error(msg));
        }
    }
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let view = ViewData {
        viewport: widget_viewport(),
        state: STATE.with(|s| s.borrow().clone()),
        now_secs: SystemTime::now().unix_secs,
    };
    let _ = render_ui(
        view.viewport.width,
        view.viewport.height,
        launch_view(&view),
    );
    // Tick once a second so the countdown stays current.
    request_frame_after(TICK_MS);
}
