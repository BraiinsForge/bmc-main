// Copyright (C) 2026  Braiins Systems s.r.o.

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
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
use manifest_params::Params;
#[cfg(target_arch = "wasm32")]
use miner::{AuthState, MinerData};
#[cfg(target_arch = "wasm32")]
use shared::clock_palette;

#[cfg(any(target_arch = "wasm32", test))]
const STATS_REFRESH_MS: u32 = 5_000;
#[cfg(any(target_arch = "wasm32", test))]
const HASHBOARDS_REFRESH_MS: u32 = 10_000;
#[cfg(target_arch = "wasm32")]
const RETRY_MS: u32 = 10_000;
#[cfg(target_arch = "wasm32")]
const DEBOUNCE_MS: u32 = 300;

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
enum MinerSource {
    Stats,
    Hashboards,
    Constraints,
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct State {
    miner: MinerData,
    auth: AuthState,
    stats_age_ms: Option<u32>,
    hashboards_age_ms: Option<u32>,
    stats_stale: bool,
    hashboards_stale: bool,
}

#[cfg(target_arch = "wasm32")]
struct Handles {
    login: PollHandle,
    stats: PollHandle,
    hashboards: PollHandle,
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
    let login = register_poll(
        build_login,
        on_login_reply,
        PollConfig {
            interval_ms: None,
            retry_ms: RETRY_MS,
            debounce_ms: DEBOUNCE_MS,
            enabled: true,
        },
    );
    let stats = register_poll(
        build_miner,
        on_miner_reply,
        PollConfig {
            interval_ms: Some(STATS_REFRESH_MS),
            retry_ms: RETRY_MS,
            debounce_ms: DEBOUNCE_MS,
            enabled: true,
        },
    );
    let hashboards = register_poll(
        build_miner,
        on_miner_reply,
        PollConfig {
            interval_ms: Some(HASHBOARDS_REFRESH_MS),
            retry_ms: RETRY_MS,
            debounce_ms: DEBOUNCE_MS,
            enabled: true,
        },
    );
    // Tuner constraints anchor both gauge rings. Fetched once per login
    // (constraints change only on a re-tune): one-shot, invalidated on login.
    let constraints = register_poll(
        build_miner,
        on_miner_reply,
        PollConfig {
            interval_ms: None,
            retry_ms: RETRY_MS,
            debounce_ms: DEBOUNCE_MS,
            enabled: true,
        },
    );
    HANDLES.with(|handles| {
        *handles.borrow_mut() = Some(Handles {
            login,
            stats,
            hashboards,
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
        } else if handle == handles.hashboards {
            MinerSource::Hashboards
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
    let url = miner::endpoint(&params.miner_url, "/auth/login");
    let body = fmt!(
        r#"{{"username":"root","password":"{}"}}"#,
        JsonStr(&params.miner_password)
    );
    Some(
        FetchSpec::post(url)
            .headers("Content-Type: application/json")
            .body(body.as_bytes()),
    )
}

#[cfg(target_arch = "wasm32")]
fn build_miner(handle: PollHandle) -> Option<FetchSpec> {
    let header = STATE.with(|state| state.borrow().auth.auth_header())?;
    let path = match miner_source(handle) {
        MinerSource::Stats => "/miner/stats",
        MinerSource::Hashboards => "/miner/hw/hashboards",
        MinerSource::Constraints => "/configuration/constraints",
    };
    Some(FetchSpec::get(miner::endpoint(&Params::current().miner_url, path)).headers(header))
}

// Deliberately requests no frame: the clock paints once per second
// (`request_frame_after(1000)` in `render`), and refreshed auth state surfaces
// on the next tick. Forcing a frame here would paint at a sub-second offset and
// reset the 1s cadence, so the second hand stops advancing in even steps.
#[cfg(target_arch = "wasm32")]
fn on_login_reply(_handle: PollHandle, response: &FetchResponse) {
    if response.ok()
        && let Some(token) = response.json().str("/token")
    {
        STATE.with(|state| state.borrow_mut().auth = AuthState::Authenticated(token));
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                handles.stats.invalidate();
                handles.hashboards.invalidate();
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
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            match source {
                MinerSource::Stats => {
                    miner::parse_stats(&response.json(), &mut state.miner);
                    state.stats_age_ms = Some(0);
                    state.stats_stale = false;
                }
                MinerSource::Hashboards => {
                    miner::parse_hashboards(&response.json(), &mut state.miner);
                    state.hashboards_age_ms = Some(0);
                    state.hashboards_stale = false;
                }
                // Constraints are slow-changing config: parse on success, but
                // keep the last good values on failure without a stale banner.
                MinerSource::Constraints => {
                    miner::parse_constraints(&response.json(), &mut state.miner);
                }
            }
        });
    } else {
        log_warn!(
            "mining-clock: miner endpoint failed with status {}",
            response.status
        );
        // Keep the last good data and flag the source stale so the render path
        // can surface a "stale data" banner; the flag clears on the next success.
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            match source {
                MinerSource::Stats => {
                    state.stats_age_ms = None;
                    state.stats_stale = true;
                }
                MinerSource::Hashboards => {
                    state.hashboards_age_ms = None;
                    state.hashboards_stale = true;
                }
                // Keep the last good constraints; a re-tune is rare and a failed
                // refresh should not blank the gauge scale or raise a banner.
                MinerSource::Constraints => {}
            }
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn advance_freshness(state: &mut State, delta_ms: u32) {
    if let Some(age) = state.stats_age_ms {
        let age = age.saturating_add(delta_ms);
        if miner::is_stale(age) {
            state.stats_age_ms = None;
            state.stats_stale = true;
        } else {
            state.stats_age_ms = Some(age);
        }
    }
    if let Some(age) = state.hashboards_age_ms {
        let age = age.saturating_add(delta_ms);
        if miner::is_stale(age) {
            state.hashboards_age_ms = None;
            state.hashboards_stale = true;
        } else {
            state.hashboards_age_ms = Some(age);
        }
    }
}

// The auth-error banner takes precedence over stale data, matching mining-info.
// Stale data only surfaces once some data has loaded, so a never-connected miner
// reads as N/A on the gauge rather than raising a stale banner.
#[cfg(target_arch = "wasm32")]
fn overlay_message(auth_failed: bool, stale: bool, miner: &MinerData) -> Option<&'static str> {
    let has_data = miner.hashrate_ths.is_some()
        || miner.power_w.is_some()
        || miner.nominal_hashrate_ths.is_some();
    if auth_failed {
        Some(mining::overlay::AUTH_ERROR_TEXT)
    } else if stale && has_data {
        Some(mining::overlay::STALE_DATA_TEXT)
    } else {
        None
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        variant,
    } = widget_size();
    let now = SystemTime::now();
    let params = Params::current();
    let effective_tz = params.timezone_override.as_deref().map(Tz::from_runtime);
    let palette = clock_palette(system::current().night_mode().unwrap_or(false));
    let (miner, auth_failed, stale) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        advance_freshness(&mut state, delta_ms);
        (
            state.miner.clone(),
            matches!(state.auth, AuthState::Failed),
            state.stats_stale || state.hashboards_stale,
        )
    });
    let overlay = overlay_message(auth_failed, stale, &miner);

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

/// Fires after every per-widget params delivery (operator change).
/// Re-authenticates on a credential change; the new values surface on the
/// next 1s render tick, so no frame is requested here. As in the fetch-reply
/// handlers, forcing a frame would paint off the 1s cadence and make the second
/// hand skip its even steps.
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
    use super::*;

    const _: () = {
        assert!(STATS_REFRESH_MS < miner::STALE_AFTER_MS);
        assert!(HASHBOARDS_REFRESH_MS < miner::STALE_AFTER_MS);
    };
}
