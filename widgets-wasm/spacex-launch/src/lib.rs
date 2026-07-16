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

#![allow(clippy::cast_precision_loss)]

//! SpaceX Launch widget for the WASM runtime (BDK-285).
//!
//! Renders the next SpaceX launch as a countdown plus mission details
//! (full/large/medium/small). Data comes from nexus
//! (`/api/v1/data/spacex/next-launch`), which normalizes and caches the
//! upstream Launch Library 2 feed; the countdown is ticked locally from the
//! device clock between refreshes.

mod model;
#[cfg(target_arch = "wasm32")]
mod render;

/// How a poll reply classifies.
#[cfg(any(target_arch = "wasm32", test))]
enum Reply<T> {
    Data(T),
    Empty,
    Error,
}

/// What to do with a classified reply.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug, PartialEq, Eq)]
enum Outcome<T> {
    Store(T),
    NoLaunch,
    Keep,
    Fail,
}

/// Empty clears even when a launch is held; failure keeps the last launch, else errors.
#[cfg(any(target_arch = "wasm32", test))]
fn outcome<T>(reply: Reply<T>, has_data: bool) -> Outcome<T> {
    match reply {
        Reply::Data(data) => Outcome::Store(data),
        Reply::Empty => Outcome::NoLaunch,
        Reply::Error if has_data => Outcome::Keep,
        Reply::Error => Outcome::Fail,
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use std::cell::{Cell, RefCell};

    #[expect(clippy::wildcard_imports, reason = "widget glue uses many SDK exports")]
    use bmc_wasm_sdk::*;

    use crate::model::LaunchData;
    use crate::render;

    const NEXUS_URL: &str = "https://nexus.braiinsforge.com/api/v1/data/spacex/next-launch";
    /// Fixed 5-min refresh; a fixed period keeps Decks from refreshing in lockstep.
    const REFRESH_MS: u32 = 300_000;
    const RETRY_MS: u32 = 30_000;
    /// Countdown re-renders once a second.
    const TICK_MS: u32 = 1_000;

    enum State {
        Loading,
        Loaded(LaunchData),
        NoLaunch,
        Error(String),
    }

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
        Some(FetchSpec::get(NEXUS_URL))
    }

    fn on_launch(handle: PollHandle, response: &FetchResponse) {
        let reply = if response.ok() {
            match LaunchData::parse(&response.json()) {
                Ok(Some(data)) => crate::Reply::Data(data),
                Ok(None) => crate::Reply::Empty,
                Err(_) => crate::Reply::Error,
            }
        } else {
            log_warn!("spacex: fetch failed (status {})", response.status);
            crate::Reply::Error
        };
        let has_data = STATE.with(|s| matches!(&*s.borrow(), State::Loaded(_)));

        match crate::outcome(reply, has_data) {
            crate::Outcome::Store(data) => {
                STATE.with(|s| *s.borrow_mut() = State::Loaded(data));
            }
            crate::Outcome::NoLaunch => {
                STATE.with(|s| *s.borrow_mut() = State::NoLaunch);
            }
            crate::Outcome::Keep => {
                // Retry a malformed 2xx sooner; a network failure waits for the engine.
                if response.ok() {
                    handle.retry();
                }
            }
            crate::Outcome::Fail => {
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
        let size = widget_size();
        let node = STATE.with(|s| match &*s.borrow() {
            State::Loaded(data) => render::current_view(data, size),
            State::Loading => render::loading_view(),
            State::NoLaunch => render::empty_view(),
            State::Error(msg) => render::error_view(msg),
        });
        let _ = render_ui(size.width, size.height, node);
        // Tick once a second so the countdown stays current.
        request_frame_after(TICK_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_reply_is_stored() {
        assert_eq!(outcome(Reply::Data(7), false), Outcome::Store(7));
        assert_eq!(outcome(Reply::Data(7), true), Outcome::Store(7));
    }

    #[test]
    fn empty_reply_clears_regardless_of_prior_data() {
        assert_eq!(outcome::<i32>(Reply::Empty, false), Outcome::NoLaunch);
        assert_eq!(outcome::<i32>(Reply::Empty, true), Outcome::NoLaunch);
    }

    #[test]
    fn failure_keeps_data_when_present_else_errors() {
        assert_eq!(outcome::<i32>(Reply::Error, true), Outcome::Keep);
        assert_eq!(outcome::<i32>(Reply::Error, false), Outcome::Fail);
    }
}
