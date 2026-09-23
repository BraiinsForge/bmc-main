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

//! The entry points the host calls: the prediction poll,
//! and a render that counts down against the device clock.

use std::cell::RefCell;
use std::time::Duration;

#[expect(
    clippy::wildcard_imports,
    reason = "runtime code uses the SDK's builders, host shims, and macros throughout"
)]
use bmc_wasm_sdk::*;
use units::availability::Availability;

use crate::api;
use crate::manifest_params::Params;
use crate::model::{Freshness, Prediction, RATE_LIMIT_RETRY, Status};
use crate::screens::{ViewData, halving_view};

const INITIAL_INTERVAL_MS: u32 = 60_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Redraw cadence so the minutes place stays fresh between polls.
const TICK_MS: u32 = 30_000;

#[derive(Default)]
struct Live {
    handle: Option<PollHandle>,
    prediction: Availability<Prediction>,
    freshness: Option<Freshness>,
    rate_limited: bool,
}

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live::default());
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "signature must match the poll Build fn pointer, which returns Option"
)]
fn build(_handle: PollHandle) -> Option<FetchSpec> {
    Some(FetchSpec::get(api::URL).timeout(FETCH_TIMEOUT))
}

fn on_reply(handle: PollHandle, response: &FetchResponse) {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.rate_limited = response.status == 429;
        if live.rate_limited {
            live.prediction.mark_failed();
            handle.retry_after(
                u32::try_from(RATE_LIMIT_RETRY.as_millis())
                    .expect("BUG: the rate-limit retry is minutes, well within u32 milliseconds"),
            );
            log_warn!(
                "halving: rate limited; retrying in {} minutes",
                RATE_LIMIT_RETRY.as_secs() / 60
            );
        } else if !response.ok() {
            live.prediction.mark_failed();
            log_warn!("halving: fetch failed with status {}", response.status);
        } else if let Some((prediction, freshness)) = api::parse(
            &response.json(),
            &parse_datetime,
            SystemTime::now().unix_secs,
        ) {
            live.prediction = Availability::Available(prediction);
            live.freshness = Some(freshness);
            handle.set_interval(freshness.interval_ms().max(INITIAL_INTERVAL_MS));
        } else {
            // A 2xx with an unusable payload isn't a failure to the poll engine,
            // which reschedules off the HTTP status, so ask for a retry
            // rather than waiting the full interval.
            live.prediction.mark_failed();
            handle.retry();
            log_warn!("halving: 2xx payload missing/invalid prediction fields");
        }
    });
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let handle = register_poll(
        build,
        on_reply,
        PollConfig {
            interval_ms: Some(INITIAL_INTERVAL_MS),
            debounce_ms: 0,
            ..Default::default()
        },
    );
    LIVE.with(|live| live.borrow_mut().handle = Some(handle));
}

fn status(live: &Live, now_secs: i64) -> Status {
    if live.rate_limited {
        return Status::RateLimited;
    }
    if let Some(anchor) = live
        .freshness
        .and_then(|freshness| freshness.stale_anchor(now_secs, FETCH_TIMEOUT.as_secs()))
    {
        return Status::Stale(anchor);
    }
    if let Some(handle) = live.handle
        && handle.is_stale()
        && let Some(anchor) = handle.last_success_time()
    {
        return Status::Stale(anchor.unix_secs);
    }
    if live.prediction.failed() {
        Status::Failed
    } else {
        Status::Ready
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let now_secs = SystemTime::now().unix_secs;
    let view = LIVE.with(|live| {
        let live = live.borrow();
        ViewData {
            viewport: widget_viewport(),
            params: Params::current(),
            prediction: live.prediction,
            status: status(&live, now_secs),
            now_secs,
        }
    });
    let _ = render_ui(
        view.viewport.width,
        view.viewport.height,
        halving_view(&view),
    );
    request_frame_after(TICK_MS);
}

#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    request_frame();
}

/// The predicted date and time follow the system's formats and timezone.
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}
