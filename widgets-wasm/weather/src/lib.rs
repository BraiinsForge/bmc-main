// Copyright (C) 2026  Braiins Systems s.r.o.

//! Weather widget — current conditions and forecast, four sizes.
//! Ported from `deckfeeder/assets/widgets/weather/` (a JS/HTML widget).

pub mod display;
mod manifest_params;
pub mod model;
#[cfg(target_arch = "wasm32")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_miss_disables_weather_poll_until_params_change() {
        assert_eq!(weather_fetch_action(404), WeatherFetchAction::BadLocation);
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
    const RETRY_MS: u32 = 10_000;
    const DEBOUNCE_MS: u32 = 300;
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
                retry_ms: RETRY_MS,
                debounce_ms: DEBOUNCE_MS,
                enabled: true,
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
        let parsed = match super::weather_fetch_action(response.status) {
            super::WeatherFetchAction::BadLocation => {
                STATE.with(|s| *s.borrow_mut() = State::BadLocation);
                handle.set_enabled(false);
                request_frame();
                return;
            }
            super::WeatherFetchAction::ReadPayload => {
                model::Weather::try_from(&response.json()).ok()
            }
            super::WeatherFetchAction::TransientFailure => {
                log_warn!("weather: fetch failed (status {})", response.status);
                None
            }
        };
        if let Some(weather) = parsed {
            STATE.with(|s| *s.borrow_mut() = State::Loaded(weather));
        } else {
            if response.ok() {
                handle.retry();
            }
            STATE.with(|s| {
                if matches!(&*s.borrow(), State::Loading) {
                    *s.borrow_mut() = State::Error;
                }
            });
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

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let size = widget_size();
        let params = manifest_params::Params::current();
        let node = STATE.with(|s| match &*s.borrow() {
            State::Loaded(weather) => render::current_view(weather, &params, size),
            State::BadLocation => render::message_view("Location not found", size),
            State::Loading | State::Error => render::message_view(display::NOT_AVAILABLE, size),
        });
        let _ = render_ui(size.width, size.height, node);
    }
}
