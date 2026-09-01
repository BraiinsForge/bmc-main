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

//! Mining clock widget — round analog dial with live miner gauge rings.
//!
//! Module layout:
//! - `shared` — palette, tz helpers, alarm-row drawer, numeric utils
//! - `analog` — analog parent: hand assets, pivots, angle bookkeeping
//! - `analog::round` — round dial renderer

#[cfg(target_arch = "wasm32")]
mod analog;
mod manifest_params;
mod miner;
#[cfg(target_arch = "wasm32")]
mod shared;

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
use manifest_params::Params;
#[cfg(target_arch = "wasm32")]
use miner::AuthState;
#[cfg(any(target_arch = "wasm32", test))]
use miner::MinerData;
#[cfg(target_arch = "wasm32")]
use shared::clock_palette;

#[cfg(target_arch = "wasm32")]
const STATS_REFRESH_MS: u32 = 5_000;
// One-shot constraints re-poll delay on an empty reply.
#[cfg(target_arch = "wasm32")]
const RETRY_MS: u32 = 10_000;
// The miner lives on the local network, so an unreachable one should fail
// fast instead of holding the SDK-default 10s timeout.
#[cfg(target_arch = "wasm32")]
const MINER_FETCH_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
enum MinerSource {
    Stats,
    Constraints,
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct State {
    miner: MinerData,
    auth: AuthState,
}

#[cfg(target_arch = "wasm32")]
struct Handles {
    login: PollHandle,
    stats: PollHandle,
    constraints: PollHandle,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static HANDLES: RefCell<Option<Handles>> = const { RefCell::new(None) };
    // Seed the gauge transitions from empty: the first render draws zero-sweep
    // rings so the host always animates the fill in, even when miner data is
    // already available on the first frame.
    static FIRST_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let login = register_poll(build_login, on_login_reply, PollConfig::default());
    let stats = register_poll(
        build_miner,
        on_miner_reply,
        PollConfig {
            interval_ms: Some(STATS_REFRESH_MS),
            ..Default::default()
        },
    );
    // Tuner constraints anchor both gauge rings. Fetched once per login
    // (constraints change only on a re-tune): one-shot, invalidated on login.
    let constraints = register_poll(build_miner, on_miner_reply, PollConfig::default());
    HANDLES.with(|handles| {
        *handles.borrow_mut() = Some(Handles {
            login,
            stats,
            constraints,
        });
    });
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn miner_source(handle: PollHandle) -> MinerSource {
    HANDLES.with(|handles| {
        let handles = handles.borrow();
        let handles = handles
            .as_ref()
            .expect("BUG: mining-clock poll handle used before init");
        if handle == handles.stats {
            MinerSource::Stats
        } else if handle == handles.constraints {
            MinerSource::Constraints
        } else {
            panic!("BUG: mining-clock unknown miner poll handle");
        }
    })
}

#[cfg(target_arch = "wasm32")]
fn build_login(_handle: PollHandle) -> Option<FetchSpec> {
    let params = Params::current();
    if params.miner_password.is_empty() {
        return None;
    }
    let url = miner::endpoint(&params.miner_url, mining::bos::LOGIN_PATH);
    let body = mining::bos::login_body(&params.miner_password);
    Some(
        FetchSpec::post(url)
            .headers("Content-Type: application/json")
            .body(body.as_bytes())
            .timeout(MINER_FETCH_TIMEOUT),
    )
}

#[cfg(target_arch = "wasm32")]
fn build_miner(handle: PollHandle) -> Option<FetchSpec> {
    let header = STATE.with(|state| state.borrow().auth.auth_header())?;
    let path = match miner_source(handle) {
        MinerSource::Stats => "/miner/stats",
        MinerSource::Constraints => "/configuration/constraints",
    };
    Some(
        FetchSpec::get(miner::endpoint(&Params::current().miner_url, path))
            .headers(header)
            .timeout(MINER_FETCH_TIMEOUT),
    )
}

// Deliberately requests no frame: the clock paints once per second
// (`request_frame_after(1000)` in `render`), and refreshed auth state surfaces
// on the next tick. Forcing a frame here would paint at a sub-second offset and
// reset the 1s cadence, so the second hand stops advancing in even steps.
#[cfg(target_arch = "wasm32")]
fn on_login_reply(_handle: PollHandle, response: &FetchResponse) {
    if response.ok()
        && let Some(token) = mining::bos::token(&response.json())
    {
        STATE.with(|state| state.borrow_mut().auth = AuthState::Authenticated(token));
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                handles.stats.invalidate();
                handles.constraints.invalidate();
            }
        });
    } else {
        log_warn!("mining-clock: login failed with status {}", response.status);
        STATE.with(|state| state.borrow_mut().auth = AuthState::Failed);
    }
}

// Requests no frame for the same reason as `on_login_reply`: fresh stats and
// constraints land on the next 1s render tick. Painting on every fetch reply
// would knock the second hand off its even one-second steps.
#[cfg(target_arch = "wasm32")]
fn on_miner_reply(handle: PollHandle, response: &FetchResponse) {
    if response.status == 401 {
        STATE.with(|state| state.borrow_mut().auth = AuthState::LoggingIn);
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                handles.login.invalidate();
            }
        });
        return;
    }

    let source = miner_source(handle);
    if response.ok() {
        let stored = STATE.with(|state| {
            let mut state = state.borrow_mut();
            match source {
                MinerSource::Stats => miner::parse_stats(&response.json(), &mut state.miner),
                // Constraints are slow-changing config: parse on success,
                // but keep the last good values on failure without a stale banner.
                MinerSource::Constraints => {
                    miner::parse_constraints(&response.json(), &mut state.miner)
                }
            }
        });
        // Empty 2xx (reachable, no data yet): flag stale,
        // but re-poll at the source's cadence, not the failure back-off.
        if !stored {
            log_warn!("mining-clock: miner endpoint returned no usable data");
            handle.retry_after(match source {
                MinerSource::Stats => STATS_REFRESH_MS,
                MinerSource::Constraints => RETRY_MS,
            });
        }
    } else {
        log_warn!(
            "mining-clock: miner endpoint failed with status {}",
            response.status
        );
        // Keep the last good data; the poll engine tracks staleness now.
    }
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OverlaySelect {
    Auth,
    Stale,
    None,
}

// Auth error outranks stale; stale needs loaded data (a never-connected miner
// reads as N/A, not stale). The render fills the stale anchor from the poll.
#[cfg(any(target_arch = "wasm32", test))]
fn select_overlay(auth_failed: bool, stale: bool, miner: &MinerData) -> OverlaySelect {
    let has_data = miner.hashrate_ths.is_some() || miner.power_w.is_some();
    if auth_failed {
        OverlaySelect::Auth
    } else if stale && has_data {
        OverlaySelect::Stale
    } else {
        OverlaySelect::None
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        variant,
    } = widget_size();
    let now = SystemTime::now();
    let params = Params::current();
    let effective_tz = params.timezone_override.as_deref().map(Tz::from_runtime);
    let palette = clock_palette(system::current().night_mode().unwrap_or(false));
    let (miner, auth_failed) = STATE.with(|state| {
        let state = state.borrow();
        (state.miner.clone(), matches!(state.auth, AuthState::Failed))
    });
    let (stale, anchor) = HANDLES.with(|handles| {
        handles.borrow().as_ref().map_or((false, None), |handles| {
            (handles.stats.is_stale(), handles.stats.last_success_time())
        })
    });
    let overlay = match select_overlay(auth_failed, stale, &miner) {
        OverlaySelect::Auth => Some(mining::overlay::OverlayKind::Auth),
        OverlaySelect::Stale => anchor.map(mining::overlay::OverlayKind::Stale),
        OverlaySelect::None => None,
    };

    let first_frame = FIRST_FRAME.replace(false);
    let root = analog::round::render(
        now,
        &params,
        variant,
        w,
        h,
        effective_tz.as_ref(),
        &palette,
        &miner,
        first_frame,
        overlay,
    );

    let _ = render_ui(w, h, root);
    // Re-render once per second so the displayed time advances.
    request_frame_after(1000);
    // The seeded first frame shows empty rings; schedule the real values now so
    // the host transition animates them in on the next tick.
    if first_frame {
        request_frame();
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}

/// Re-authenticates when the miner URL or password changes.
///
/// Deliberately requests no frame: the next 1 s tick shows the new values,
/// and an off-cadence repaint makes the second hand skip its even steps.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let prev = Params::previous();
    let changed = prev.as_ref().map_or_else(
        || vec!["miner_url", "miner_password"],
        |prev| Params::current().changed_keys(prev),
    );
    if changed.contains(&"miner_url") || changed.contains(&"miner_password") {
        let password_empty = Params::current().miner_password.is_empty();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.auth = if password_empty {
                AuthState::NoToken
            } else {
                AuthState::LoggingIn
            };
        });
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                handles.login.invalidate();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{OverlaySelect, miner::MinerData, select_overlay};

    #[test]
    fn auth_overlay_takes_precedence_over_stale_data() {
        let miner = MinerData {
            hashrate_ths: Some(122.48),
            ..MinerData::default()
        };

        assert_eq!(select_overlay(true, true, &miner), OverlaySelect::Auth);
    }

    #[test]
    fn stale_overlay_requires_loaded_miner_data() {
        assert_eq!(
            select_overlay(false, true, &MinerData::default()),
            OverlaySelect::None
        );

        let miner = MinerData {
            power_w: Some(41.0),
            ..MinerData::default()
        };

        assert_eq!(select_overlay(false, true, &miner), OverlaySelect::Stale);
    }
}
