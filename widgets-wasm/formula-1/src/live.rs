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
    LIVE_INTERVAL_MS, LIVE_PROBE_INTERVAL_MS, Resource, STATIC_INTERVAL_MS, media_type_names_json,
    resource_needed, wire,
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

/// Everything provable before any parser runs.
/// `None` keeps the last good data, whatever the fault.
fn valid_reply(resource: Resource, response: &FetchResponse) -> Option<JsonDoc> {
    if !response.ok() {
        // A cold server answers 503 until its first upstream snapshot
        // lands, which can take minutes. Holding the last good payload
        // keeps the screens up while the engine retries.
        log_warn!(
            "formula-1: {} fetch failed with status {}",
            resource.name(),
            response.status
        );
        return None;
    }
    // Absent is tolerated; a captive portal's `text/html` and a header
    // the host could not parse are not — neither names JSON.
    if let Some(content_type) = response.content_type()
        && !media_type_names_json(
            response.media_type(MediaTypePart::Subtype).as_deref(),
            response.media_type(MediaTypePart::Suffix).as_deref(),
        )
    {
        log_warn!(
            "formula-1: {} answered as {}; keeping the last good data",
            resource.name(),
            content_type
        );
        return None;
    }
    let json = response.json();
    if !json.is_valid() {
        log_warn!(
            "formula-1: {} body is not JSON; keeping the last good data",
            resource.name()
        );
        return None;
    }
    let want = resource.envelope_id(Params::current().driver.as_manifest_value());
    if json.str(wire::RESOURCE).as_deref() != Some(want.as_str()) {
        log_warn!(
            "formula-1: {} reply answers for another resource; keeping the last good data",
            resource.name()
        );
        return None;
    }
    Some(json)
}

fn keep_last_good(resource: Resource) {
    log_warn!(
        "formula-1: {} reply did not parse; keeping the last good data",
        resource.name()
    );
}

fn on_reply(handle: PollHandle, response: &FetchResponse) {
    let resource = resource_of(handle);
    let Some(json) = valid_reply(resource, response) else {
        return;
    };
    DATA.with(|data| {
        let mut data = data.borrow_mut();
        match resource {
            Resource::Standings => match parse::standings(&json) {
                Ok(rows) => data.standings = rows,
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::DriverStats => match parse::driver_stats(&json) {
                Ok(rows) => data.driver_stats = rows,
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::Driver => match parse::driver(&json) {
                Ok(driver) => data.driver = Some(driver),
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::Teams => match parse::teams(&json) {
                Ok(rows) => data.teams = rows,
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::Drivers => match parse::driver_teams(&json) {
                Ok(rows) => data.driver_teams = rows,
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::NextRace => match parse::next_race(&json, Params::current().local_time) {
                Ok(race) => data.next_race = race,
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::LiveRace => match parse::live_board(&json) {
                Ok(board) => data.live_race = board,
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::LiveQuali => match parse::live_board(&json) {
                Ok(board) => data.live_quali = board,
                Err(parse::Malformed) => keep_last_good(resource),
            },
            Resource::LivePractice => match parse::live_board(&json) {
                Ok(board) => data.live_practice = board,
                Err(parse::Malformed) => keep_last_good(resource),
            },
        }
    });
    if resource.is_live_board() {
        retune_live_cadence();
    }
    crate::artwork::sync();
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

/// Drop the held card and re-fetch: the reply may never come, and the
/// old person's card must not sit under the new setting until it does.
pub fn invalidate_driver() {
    DATA.with(|data| data.borrow_mut().driver = None);
    invalidate(Resource::Driver);
}

/// Re-fetch the weekend after a zone change. Session starts are converted
/// as the payload is read, so the held model carries the old zone's clock
/// until a reply replaces it — a minute away at the static cadence.
pub fn invalidate_next_race() {
    invalidate(Resource::NextRace);
}

fn invalidate(resource: Resource) {
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            for handle in handles {
                if resource_of(*handle) == resource {
                    handle.invalidate();
                }
            }
        }
    });
}
