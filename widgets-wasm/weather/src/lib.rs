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

//! Weather widget — current conditions and forecast, four sizes.
//! Ported from `deckfeeder/assets/widgets/weather/` (a JS/HTML widget).

pub mod display;
mod manifest_params;
pub mod model;
#[cfg(any(target_arch = "wasm32", test))]
pub mod render;
pub mod url;
pub mod weather_code;
pub mod wind;

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeatherFetchAction {
    BadLocation,
    ReadPayload,
    TransientFailure,
}

#[cfg(any(target_arch = "wasm32", test))]
fn weather_fetch_action(status: u32) -> WeatherFetchAction {
    match status {
        404 => WeatherFetchAction::BadLocation,
        200..=299 => WeatherFetchAction::ReadPayload,
        _ => WeatherFetchAction::TransientFailure,
    }
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetchOutcome {
    Store,
    Keep,
    Fail,
    BadLocation,
}

#[cfg(any(target_arch = "wasm32", test))]
fn fetch_outcome(action: WeatherFetchAction, parsed_ok: bool, has_data: bool) -> FetchOutcome {
    match action {
        WeatherFetchAction::BadLocation => FetchOutcome::BadLocation,
        WeatherFetchAction::ReadPayload if parsed_ok => FetchOutcome::Store,
        WeatherFetchAction::ReadPayload | WeatherFetchAction::TransientFailure => {
            if has_data {
                FetchOutcome::Keep
            } else {
                FetchOutcome::Fail
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_miss_disables_weather_poll_until_params_change() {
        assert_eq!(weather_fetch_action(404), WeatherFetchAction::BadLocation);
    }

    #[test]
    fn a_404_is_a_bad_location_regardless_of_held_data() {
        assert_eq!(
            fetch_outcome(WeatherFetchAction::BadLocation, false, false),
            FetchOutcome::BadLocation
        );
        assert_eq!(
            fetch_outcome(WeatherFetchAction::BadLocation, false, true),
            FetchOutcome::BadLocation
        );
    }

    #[test]
    fn a_parsed_payload_replaces_the_data() {
        assert_eq!(
            fetch_outcome(WeatherFetchAction::ReadPayload, true, false),
            FetchOutcome::Store
        );
        assert_eq!(
            fetch_outcome(WeatherFetchAction::ReadPayload, true, true),
            FetchOutcome::Store
        );
    }

    #[test]
    fn a_failure_keeps_data_when_present_else_errors() {
        // Held data survives a failed refresh (and goes stale); with nothing
        // loaded yet the same failure is a hard error.
        for action in [
            WeatherFetchAction::ReadPayload,
            WeatherFetchAction::TransientFailure,
        ] {
            assert_eq!(fetch_outcome(action, false, true), FetchOutcome::Keep);
            assert_eq!(fetch_outcome(action, false, false), FetchOutcome::Fail);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{display, manifest_params, model, render, url};
    use std::cell::{Cell, RefCell};

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports"
    )]
    use bmc_wasm_sdk::*;

    const REFRESH_MS: u32 = 300_000;
    const NEXUS_BASE: &str = "https://nexus.braiinsforge.com/api/v1/data/weather/";

    enum State {
        Loading,
        Loaded(model::Weather),
        BadLocation,
        Error,
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
        static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
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
        POLL.with(|p| p.set(Some(handle)));
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        let location = manifest_params::Params::current().location;
        let location = location.trim();
        if location.is_empty() {
            return None;
        }
        Some(FetchSpec::get(url::weather_url(NEXUS_BASE, location)))
    }

    fn on_weather(handle: PollHandle, response: &FetchResponse) {
        let action = super::weather_fetch_action(response.status);
        let parsed = match action {
            super::WeatherFetchAction::ReadPayload => {
                model::Weather::try_from(&response.json()).ok()
            }
            super::WeatherFetchAction::TransientFailure => {
                log_warn!("weather: fetch failed (status {})", response.status);
                None
            }
            super::WeatherFetchAction::BadLocation => None,
        };
        let has_data = STATE.with(|s| matches!(&*s.borrow(), State::Loaded(_)));
        match super::fetch_outcome(action, parsed.is_some(), has_data) {
            super::FetchOutcome::Store => {
                let weather = parsed.expect("BUG: Store outcome implies a parsed payload");
                STATE.with(|s| *s.borrow_mut() = State::Loaded(weather));
            }
            super::FetchOutcome::Keep => {
                // A 2xx whose body failed to parse is worth retrying sooner than
                // the next poll; a transient/network failure waits for the engine.
                if action == super::WeatherFetchAction::ReadPayload {
                    handle.retry();
                }
            }
            super::FetchOutcome::Fail => {
                STATE.with(|s| *s.borrow_mut() = State::Error);
            }
            super::FetchOutcome::BadLocation => {
                STATE.with(|s| *s.borrow_mut() = State::BadLocation);
                handle.set_enabled(false);
            }
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        if prev.as_ref().is_none_or(|p| p.location != cur.location) {
            STATE.with(|s| *s.borrow_mut() = State::Loading);
            // poll engine does not rebuild on param change; invalidate forces a fresh fetch
            POLL.with(|p| {
                if let Some(handle) = p.get() {
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
        let handle = POLL.with(Cell::get)?;
        if handle.is_stale() {
            handle.last_success_time()
        } else {
            None
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let size = widget_size();
        let params = manifest_params::Params::current();
        let node = if params.location.trim().is_empty() {
            render::message_view(display::ENTER_LOCATION, size)
        } else {
            STATE.with(|s| match &*s.borrow() {
                State::Loaded(weather) => {
                    let view = render::current_view(weather, &params, size);
                    match stale_anchor() {
                        Some(anchor) => with_stale_overlay(view, anchor, widget_viewport().shape),
                        None => view,
                    }
                }
                State::BadLocation => render::message_view("Location not found", size),
                State::Loading => render::message_view(display::LOADING, size),
                State::Error => render::message_view(display::CANNOT_LOAD, size),
            })
        };
        let _ = render_ui(size.width, size.height, node);
    }
}
