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

//! Polling the Nexus resources and holding what came back.

use std::cell::RefCell;
use std::time::Duration;

#[expect(
    clippy::wildcard_imports,
    reason = "runtime code uses many SDK builders, macros, and host shims"
)]
use bmc_wasm_sdk::*;

use crate::api::{
    LIVE_INTERVAL_MS, LIVE_PROBE_INTERVAL_MS, Resource, STATIC_INTERVAL_MS, resource_needed,
};
use crate::manifest_params::Params;
use crate::model::Data;
use crate::parse;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

thread_local! {
    static DATA: RefCell<Data> = RefCell::new(Data::default());
    static HANDLES: RefCell<Option<[PollHandle; Resource::ALL.len()]>> =
        const { RefCell::new(None) };
}

fn resource_of(handle: PollHandle) -> Resource {
    Resource::ALL[handle.index()]
}

/// Read the held data.
pub fn with_data<T>(read: impl FnOnce(&Data) -> T) -> T {
    DATA.with(|data| read(&data.borrow()))
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the SDK Build callback signature; these resources need no credential, so a request is always issuable"
)]
fn build(handle: PollHandle) -> Option<FetchSpec> {
    let slug = Params::current().driver.as_manifest_value();
    Some(FetchSpec::get(resource_of(handle).url(slug)).timeout(FETCH_TIMEOUT))
}

fn on_reply(handle: PollHandle, response: &FetchResponse) {
    let resource = resource_of(handle);
    if !response.ok() {
        // A cold server answers 503 until its first upstream snapshot
        // lands, which can take minutes. Holding the last good payload
        // keeps the screens up while the engine retries.
        log_warn!(
            "formula-1: {} fetch failed with status {}",
            resource.name(),
            response.status
        );
        return;
    }
    let json = response.json();
    DATA.with(|data| {
        let mut data = data.borrow_mut();
        match resource {
            Resource::Standings => data.standings = parse::standings(&json),
            Resource::DriverStats => data.driver_stats = parse::driver_stats(&json),
            Resource::Driver => data.driver = parse::driver(&json),
            Resource::NextRace => data.next_race = parse::next_race(&json),
            Resource::LiveRace => data.live_race = parse::live_board(&json),
            Resource::LiveQuali => data.live_quali = parse::live_board(&json),
            Resource::LivePractice => data.live_practice = parse::live_board(&json),
        }
    });
    if resource.is_live_board() {
        retune_live_cadence();
    }
    request_frame();
}

/// Poll the session boards quickly while one is running
/// and slowly while none is, so a quiet week costs
/// a request a minute instead of one every three seconds.
fn retune_live_cadence() {
    let running = DATA.with(|data| data.borrow().any_session_running());
    let interval = if running {
        LIVE_INTERVAL_MS
    } else {
        LIVE_PROBE_INTERVAL_MS
    };
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            for handle in handles {
                if resource_of(*handle).is_live_board() {
                    handle.set_interval(interval);
                }
            }
        }
    });
}

/// Enable exactly the resources the configured view reads.
pub fn reconcile() {
    let view = Params::current().view;
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            for handle in handles {
                handle.set_enabled(resource_needed(resource_of(*handle), view));
            }
        }
    });
}

/// Start every poll dormant,
/// then let [`reconcile`] open the ones the configured view needs.
pub fn start() {
    let handles = std::array::from_fn(|index| {
        let resource = Resource::ALL[index];
        register_poll(
            build,
            on_reply,
            PollConfig {
                interval_ms: Some(if resource.is_live_board() {
                    LIVE_PROBE_INTERVAL_MS
                } else {
                    STATIC_INTERVAL_MS
                }),
                enabled: false,
                ..Default::default()
            },
        )
    });
    HANDLES.with(|cell| *cell.borrow_mut() = Some(handles));
    reconcile();
}

/// Re-fetch the per-driver resource after the operator picks
/// another driver; its URL carries the slug,
/// so the held payload is stale the moment the param changes.
pub fn invalidate_driver() {
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            for handle in handles {
                if resource_of(*handle) == Resource::Driver {
                    handle.invalidate();
                }
            }
        }
    });
}
