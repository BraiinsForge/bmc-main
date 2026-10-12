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

//! The entry points the host calls: the forecast poll, and a render of what it holds.

use std::cell::RefCell;

use bmc_wasm_sdk::*;

use crate::api::{self, FetchOutcome, WeatherFetchAction};
use crate::manifest_params::Params;
use crate::model::{State, Weather};
use crate::screens::{ViewData, weather_view};

const REFRESH_MS: u32 = 300_000;

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
    static POLL: RefCell<Option<PollHandle>> = const { RefCell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let handle = register_poll(
        build_request,
        on_weather,
        PollConfig {
            interval_ms: Some(REFRESH_MS),
            ..Default::default()
        },
    );
    POLL.set(Some(handle));
}

fn build_request(_handle: &PollHandle) -> Option<FetchSpec> {
    let location = Params::current().location;
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    Some(FetchSpec::get(api::weather_url(api::NEXUS_BASE, location)))
}

fn on_weather(handle: &PollHandle, response: &FetchResponse) {
    let action = api::weather_fetch_action(response.status);
    let parsed = match action {
        WeatherFetchAction::ReadPayload => Weather::try_from(&response.json()).ok(),
        WeatherFetchAction::TransientFailure => {
            log_warn!("weather: fetch failed (status {})", response.status);
            None
        }
        WeatherFetchAction::BadLocation => None,
    };
    let has_data = STATE.with(|s| matches!(&*s.borrow(), State::Loaded(_)));
    match api::fetch_outcome(action, parsed.is_some(), has_data) {
        FetchOutcome::Store => {
            let weather = parsed.expect("BUG: Store outcome implies a parsed payload");
            STATE.with(|s| *s.borrow_mut() = State::Loaded(weather));
        }
        FetchOutcome::Keep => {
            // A 2xx whose body failed to parse is worth retrying sooner than
            // the next poll; a transient/network failure waits for the engine.
            if action == WeatherFetchAction::ReadPayload {
                handle.retry();
            }
        }
        FetchOutcome::Fail => {
            STATE.with(|s| *s.borrow_mut() = State::Error);
        }
        FetchOutcome::BadLocation => {
            STATE.with(|s| *s.borrow_mut() = State::BadLocation);
            handle.set_enabled(false);
        }
    }
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let prev = Params::previous();
    let cur = Params::current();
    if prev.as_ref().is_none_or(|p| p.location != cur.location) {
        STATE.with(|s| *s.borrow_mut() = State::Loading);
        // poll engine does not rebuild on param change; invalidate forces a fresh fetch
        POLL.with_borrow(|p| {
            if let Some(handle) = p {
                handle.set_enabled(true);
                handle.invalidate();
            }
        });
    }
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}

// The stale overlay's anchor: the last good load, but only while stale.
fn stale_anchor() -> Option<SystemTime> {
    POLL.with_borrow(|p| {
        p.as_ref()
            .filter(|handle| handle.is_stale())
            .and_then(PollHandle::last_success_time)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let view = ViewData {
        viewport: widget_viewport(),
        params: Params::current(),
        state: STATE.with(|s| s.borrow().clone()),
        stale_since: stale_anchor(),
    };
    let _ = render_ui(
        view.viewport.width,
        view.viewport.height,
        weather_view(&view),
    );
}
