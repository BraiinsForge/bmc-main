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

#[cfg(any(target_arch = "wasm32", test))]
mod model;
#[cfg(any(target_arch = "wasm32", test))]
mod orbit;
#[cfg(any(target_arch = "wasm32", test))]
mod orbit_cache;
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

#[cfg(any(target_arch = "wasm32", test))]
const POSITION_UPDATE_MS: u32 = 1_000;

#[cfg(any(target_arch = "wasm32", test))]
fn next_frame_delay(globe_live: bool) -> Option<u32> {
    globe_live.then_some(POSITION_UPDATE_MS)
}

#[cfg(any(target_arch = "wasm32", test))]
fn globe_is_live(variant: bmc_wasm_sdk::SizeVariant, tle: Option<&model::Tle>) -> bool {
    variant == bmc_wasm_sdk::SizeVariant::Full && tle.is_some_and(orbit_cache::has_orbit_model)
}

#[cfg(any(target_arch = "wasm32", test))]
fn position_transition_ms(delta_ms: u32) -> u32 {
    if delta_ms > POSITION_UPDATE_MS.saturating_mul(2) {
        0
    } else {
        delta_ms.max(POSITION_UPDATE_MS)
    }
}

#[cfg(any(target_arch = "wasm32", test))]
struct Propagation {
    unix_secs: f64,
    transition_ms: u32,
}

#[cfg(any(target_arch = "wasm32", test))]
#[expect(
    clippy::cast_precision_loss,
    reason = "whole unix seconds remain exact at current timestamps in f64"
)]
fn propagation(previous: Option<f64>, wall_unix_secs: i64, delta_ms: u32) -> Propagation {
    let wall_unix_secs = wall_unix_secs as f64;
    let advanced = previous.map_or(wall_unix_secs, |previous| {
        previous + f64::from(delta_ms) / 1_000.0
    });

    if (advanced - wall_unix_secs).abs() > 1.0 {
        Propagation {
            unix_secs: wall_unix_secs,
            transition_ms: 0,
        }
    } else {
        Propagation {
            unix_secs: advanced,
            transition_ms: position_transition_ms(delta_ms),
        }
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
    enum State {
        Loading,
        Loaded(IssData),
        Error(String),
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
        static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        static PROPAGATION_UNIX_SECS: Cell<Option<f64>> = const { Cell::new(None) };
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
        let wall_unix_secs = SystemTime::now().unix_secs;
        let propagation = PROPAGATION_UNIX_SECS.with(|time| {
            let propagation = crate::propagation(time.get(), wall_unix_secs, delta_ms);
            time.set(Some(propagation.unix_secs));
            propagation
        });
        let node = STATE.with(|s| match &*s.borrow() {
            State::Loaded(data) => {
                render::current_view(data, size, propagation.unix_secs, propagation.transition_ms)
            }
            State::Loading => render::loading_view(),
            State::Error(msg) => render::error_view(msg),
        });
        let _ = render_ui(size.width, size.height, node);

        let globe_live = STATE.with(|s| {
            matches!(
                &*s.borrow(),
                State::Loaded(d) if crate::globe_is_live(size.variant, d.tle.as_ref())
            )
        });
        if let Some(delay_ms) = crate::next_frame_delay(globe_live) {
            request_frame_after(delay_ms);
        }
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
        for has_data in [false, true] {
            let Outcome::Store(data) = outcome(Some(sample()), has_data) else {
                panic!("BUG: a parsed payload is stored whether or not one is held");
            };
            assert!(data == sample(), "stored as parsed");
        }
    }

    #[test]
    fn failure_keeps_data_when_present_else_errors() {
        // Held data survives a failed refresh (the globe keeps propagating);
        // with nothing loaded yet the same failure is a hard error.
        assert!(matches!(outcome(None, true), Outcome::Keep));
        assert!(matches!(outcome(None, false), Outcome::Fail));
    }

    #[test]
    fn live_globe_recomputes_position_each_second() {
        assert_eq!(next_frame_delay(true), Some(1_000));
    }

    #[test]
    fn static_view_waits_for_an_external_update() {
        assert_eq!(next_frame_delay(false), None);
    }

    #[test]
    fn invalid_tle_stops_the_live_globe_cadence() {
        let tle = model::Tle {
            line1: "invalid".to_owned(),
            line2: "invalid".to_owned(),
        };

        assert_eq!(
            next_frame_delay(globe_is_live(bmc_wasm_sdk::SizeVariant::Full, Some(&tle))),
            None
        );
    }

    #[test]
    fn propagation_advances_by_elapsed_time_across_wall_clock_quantization() {
        let next = propagation(Some(1_000.0), 1_002, 1_030);
        assert!((next.unix_secs - 1_001.03).abs() < 1.0e-6);
        assert_eq!(next.transition_ms, 1_030);
    }

    #[test]
    fn clock_step_snaps_so_the_track_stays_on_the_globe() {
        let next = propagation(Some(1_000.0), 1_605, 1_000);
        assert_eq!(next.unix_secs, 1_605.0);
        assert_eq!(next.transition_ms, 0);
    }

    #[test]
    fn ordinary_updates_keep_globe_moving_until_the_next_target() {
        assert_eq!(position_transition_ms(1_030), 1_030);
        assert_eq!(position_transition_ms(300), POSITION_UPDATE_MS);
    }

    #[test]
    fn gap_updates_snap_so_the_track_stays_on_the_globe() {
        assert_eq!(position_transition_ms(30_000), 0);
    }
}
