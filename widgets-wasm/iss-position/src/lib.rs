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

//! ISS Position widget for the WASM runtime (BDK-304).
//!
//! Renders the live ISS position on a locally-rendered 3D globe with orbital
//! track, day/night terminator, and data panels (full/large/medium/small).
//! Data comes from nexus (`/api/v1/data/iss/position`), which supplies both a
//! position snapshot and the TLE; the live subpoint is propagated on-device
//! via SGP4 between refreshes.

mod model;
mod orbit;
#[cfg(target_arch = "wasm32")]
mod render;

/// What to do with a poll reply: store the freshly parsed snapshot, keep the
/// last good one, or fail outright.
#[cfg(any(target_arch = "wasm32", test))]
enum Outcome {
    Store(model::IssData),
    Keep,
    Fail,
}

/// A parsed payload replaces the data — the call site only yields `Some` for a
/// 2xx whose body parsed. Otherwise the last good snapshot is kept if we have
/// one (the globe keeps propagating from the cached TLE), else it's a hard error.
#[cfg(any(target_arch = "wasm32", test))]
fn outcome(parsed: Option<model::IssData>, has_data: bool) -> Outcome {
    match parsed {
        Some(data) => Outcome::Store(data),
        None if has_data => Outcome::Keep,
        None => Outcome::Fail,
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use std::cell::{Cell, RefCell};

    #[expect(clippy::wildcard_imports, reason = "widget glue uses many SDK exports")]
    use bmc_wasm_sdk::*;

    use crate::model::IssData;
    use crate::render;

    const NEXUS_URL: &str = "https://nexus.braiinsforge.com/api/v1/data/iss/position";
    /// Refresh cadence, matching nexus's 30-min upstream cache; the live
    /// position is propagated locally, so this only refreshes the TLE + solar
    /// position. A fixed interval keeps the fleet from polling nexus in lockstep.
    const REFRESH_MS: u32 = 1_800_000;
    const RETRY_MS: u32 = 30_000;
    /// ~30 fps while the globe animates; static states idle at 1 fps.
    const GLOBE_FRAME_MS: u32 = 33;
    const IDLE_FRAME_MS: u32 = 1_000;

    enum State {
        Loading,
        Loaded(IssData),
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
            on_iss,
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
        reason = "coerced to the SDK `Build` callback type `fn(PollHandle) -> Option<FetchSpec>`; \
                  `None` skips a poll cycle, which this always-fetch widget never needs"
    )]
    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        Some(FetchSpec::get(NEXUS_URL))
    }

    fn on_iss(handle: PollHandle, response: &FetchResponse) {
        let json = response.json();
        let parsed = if response.ok() {
            IssData::try_from(&json).ok()
        } else {
            log_warn!("iss: fetch failed (status {})", response.status);
            None
        };
        let has_data = STATE.with(|s| matches!(&*s.borrow(), State::Loaded(_)));

        match crate::outcome(parsed, has_data) {
            crate::Outcome::Store(data) => {
                STATE.with(|s| *s.borrow_mut() = State::Loaded(data));
            }
            crate::Outcome::Keep => {
                // A 2xx whose body did not parse is worth retrying sooner than
                // the next poll; a transient/network failure waits for the engine.
                if response.ok() {
                    handle.retry();
                }
            }
            crate::Outcome::Fail => {
                let msg = if response.status == 0 {
                    String::from("Network error")
                } else {
                    fmt!("API request failed ({})", response.status)
                };
                STATE.with(|s| *s.borrow_mut() = State::Error(msg));
            }
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(delta_ms: u32) {
        let size = widget_size();
        let node = STATE.with(|s| match &*s.borrow() {
            State::Loaded(data) => render::current_view(data, size, delta_ms),
            State::Loading => render::loading_view(),
            State::Error(msg) => render::error_view(msg),
        });
        let _ = render_ui(size.width, size.height, node);

        // Only the full variant's globe animates; everything else is static, so
        // keep the embedded GPU cool by idling those at 1 fps.
        let globe_live = size.variant == SizeVariant::Full
            && STATE.with(|s| matches!(&*s.borrow(), State::Loaded(d) if d.tle.is_some()));
        request_frame_after(if globe_live {
            GLOBE_FRAME_MS
        } else {
            IDLE_FRAME_MS
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        request_frame();
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::types::{Length, Speed};

    use super::*;
    use crate::model::{IssData, Visibility};

    fn sample() -> IssData {
        IssData {
            latitude: 0.0,
            longitude: 0.0,
            altitude: Length::from_kilometers(420.0),
            velocity: Speed::from_kilometers_per_hour(27_600.0),
            visibility: Visibility::Daylight,
            solar_lat: 0.0,
            solar_lon: 0.0,
            tle: None,
        }
    }

    #[test]
    fn parsed_payload_is_stored() {
        assert!(matches!(outcome(Some(sample()), false), Outcome::Store(_)));
        assert!(matches!(outcome(Some(sample()), true), Outcome::Store(_)));
    }

    #[test]
    fn failure_keeps_data_when_present_else_errors() {
        // Held data survives a failed refresh (the globe keeps propagating);
        // with nothing loaded yet the same failure is a hard error.
        assert!(matches!(outcome(None, true), Outcome::Keep));
        assert!(matches!(outcome(None, false), Outcome::Fail));
    }
}
