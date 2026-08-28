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

use std::cell::RefCell;
use std::time::Duration;

#[expect(
    clippy::wildcard_imports,
    reason = "runtime code uses the SDK's builders, host shims, and macros throughout"
)]
use bmc_wasm_sdk::*;

use crate::api;
use crate::model::{
    BitcoinData, Freshness, Resource, SizeBucket, Status, resource_needed, size_bucket,
};
use crate::screens::{ViewData, bitcoin_mining_view};

const RATE_LIMIT_RETRY_MS: u32 = 10 * 60_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

thread_local! {
    static DATA: RefCell<BitcoinData> = RefCell::new(BitcoinData::default());
    static HANDLES: RefCell<Option<[PollHandle; Resource::ALL.len()]>> = const { RefCell::new(None) };
    static FRESHNESS: RefCell<[Option<Freshness>; Resource::ALL.len()]> = const { RefCell::new([None; Resource::ALL.len()]) };
    static RATE_LIMITED: RefCell<[bool; Resource::ALL.len()]> = const { RefCell::new([false; Resource::ALL.len()]) };
}

fn resource_of(handle: PollHandle) -> Resource {
    Resource::ALL[handle.index()]
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the SDK poll builder signature; every resource is always requestable"
)]
fn build(handle: PollHandle) -> Option<FetchSpec> {
    Some(FetchSpec::get(api::url(resource_of(handle))).timeout(FETCH_TIMEOUT))
}

fn on_reply(handle: PollHandle, response: &FetchResponse) {
    let resource = resource_of(handle);
    if response.status == 429 {
        RATE_LIMITED.with(|limited| limited.borrow_mut()[handle.index()] = true);
        DATA.with(|data| data.borrow_mut().mark_failed(resource));
        handle.retry_after(RATE_LIMIT_RETRY_MS);
        log_warn!(
            "bitcoin-mining-data: {} rate limited; retrying in 10 minutes",
            resource.name()
        );
        request_frame();
        return;
    }
    RATE_LIMITED.with(|limited| limited.borrow_mut()[handle.index()] = false);
    if !response.ok() {
        DATA.with(|data| data.borrow_mut().mark_failed(resource));
        log_warn!(
            "bitcoin-mining-data: {} fetch failed with status {}",
            resource.name(),
            response.status
        );
        request_frame();
        return;
    }

    let parsed = DATA.with(|data| {
        api::parse(
            resource,
            &response.json(),
            &mut data.borrow_mut(),
            &parse_datetime,
            SystemTime::now().unix_secs,
        )
    });
    if let Some(freshness) = parsed {
        handle.set_interval(freshness.interval_ms());
        FRESHNESS.with(|values| values.borrow_mut()[handle.index()] = Some(freshness));
    } else {
        DATA.with(|data| data.borrow_mut().mark_failed(resource));
        handle.retry();
        log_warn!(
            "bitcoin-mining-data: {} returned no usable data",
            resource.name()
        );
    }
    request_frame();
}

fn reconcile() {
    let size = widget_size();
    let bucket = size_bucket(size.width, size.height);
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            for handle in handles {
                handle.set_enabled(resource_needed(resource_of(*handle), bucket));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let handles = std::array::from_fn(|index| {
        let resource = Resource::ALL[index];
        register_poll(
            build,
            on_reply,
            PollConfig {
                interval_ms: Some(resource.initial_interval_ms()),
                enabled: false,
                ..Default::default()
            },
        )
    });
    HANDLES.with(|cell| *cell.borrow_mut() = Some(handles));
    reconcile();
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    reconcile();
    request_frame();
}

fn rate_limited(bucket: SizeBucket) -> bool {
    RATE_LIMITED.with(|limited| {
        let limited = limited.borrow();
        Resource::ALL
            .into_iter()
            .enumerate()
            .any(|(index, resource)| resource_needed(resource, bucket) && limited[index])
    })
}

fn nexus_stale_anchor(bucket: SizeBucket, now_secs: i64) -> Option<i64> {
    FRESHNESS.with(|values| {
        values
            .borrow()
            .iter()
            .copied()
            .zip(Resource::ALL)
            .filter(|(_, resource)| resource_needed(*resource, bucket))
            .filter_map(|(freshness, _)| freshness?.stale_anchor(now_secs))
            .min()
    })
}

fn poll_status() -> Option<Status> {
    HANDLES.with(|handles| {
        let handles = handles.borrow();
        let handles = handles.as_ref()?;
        let stale = handles
            .iter()
            .filter(|handle| handle.enabled())
            .filter(|handle| handle.is_stale())
            .filter_map(|handle| handle.last_success_time())
            .min_by_key(|anchor| anchor.unix_secs)
            .map(|anchor| Status::Stale(anchor.unix_secs));
        if stale.is_some() {
            return stale;
        }
        handles
            .iter()
            .copied()
            .filter(|handle| handle.enabled())
            .any(PollHandle::is_offline)
            .then_some(Status::Failed)
    })
}

fn status(bucket: SizeBucket, data: &BitcoinData, now_secs: i64) -> Status {
    if rate_limited(bucket) {
        return Status::RateLimited;
    }
    if let Some(anchor) = nexus_stale_anchor(bucket, now_secs) {
        return Status::Stale(anchor);
    }
    poll_status().unwrap_or_else(|| {
        if data.has_initial_failure(bucket) {
            Status::Failed
        } else {
            Status::Ready
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let size = widget_size();
    let bucket = size_bucket(size.width, size.height);
    let data = DATA.with(|data| data.borrow().clone());
    let now_secs = SystemTime::now().unix_secs;
    let view = ViewData {
        bucket,
        status: status(bucket, &data, now_secs),
        data,
        now_secs,
    };
    let _ = render_ui(size.width, size.height, bitcoin_mining_view(&view));
}
