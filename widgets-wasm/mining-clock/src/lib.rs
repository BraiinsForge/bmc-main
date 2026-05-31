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
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct State {
    miner: MinerData,
    auth: AuthState,
    stats_age_ms: Option<u32>,
    hashboards_age_ms: Option<u32>,
}

#[cfg(target_arch = "wasm32")]
struct Handles {
    login: PollHandle,
    stats: PollHandle,
    hashboards: PollHandle,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static HANDLES: RefCell<Option<Handles>> = const { RefCell::new(None) };
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
    HANDLES.with(|handles| {
        *handles.borrow_mut() = Some(Handles {
            login,
            stats,
            hashboards,
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
    };
    Some(FetchSpec::get(miner::endpoint(&Params::current().miner_url, path)).headers(header))
}

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
            }
        });
    } else {
        log_warn!("mining-clock: login failed with status {}", response.status);
        STATE.with(|state| state.borrow_mut().auth = AuthState::Failed);
    }
    request_frame();
}

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
                }
                MinerSource::Hashboards => {
                    miner::parse_hashboards(&response.json(), &mut state.miner);
                    state.hashboards_age_ms = Some(0);
                }
            }
        });
    } else {
        log_warn!(
            "mining-clock: miner endpoint failed with status {}",
            response.status
        );
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            match source {
                MinerSource::Stats => {
                    miner::clear_stats(&mut state.miner);
                    state.stats_age_ms = None;
                }
                MinerSource::Hashboards => {
                    miner::clear_hashboards(&mut state.miner);
                    state.hashboards_age_ms = None;
                }
            }
        });
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn advance_freshness(state: &mut State, delta_ms: u32) {
    if let Some(age) = state.stats_age_ms {
        let age = age.saturating_add(delta_ms);
        if miner::is_stale(age) {
            miner::clear_stats(&mut state.miner);
            state.stats_age_ms = None;
        } else {
            state.stats_age_ms = Some(age);
        }
    }
    if let Some(age) = state.hashboards_age_ms {
        let age = age.saturating_add(delta_ms);
        if miner::is_stale(age) {
            miner::clear_hashboards(&mut state.miner);
            state.hashboards_age_ms = None;
        } else {
            state.hashboards_age_ms = Some(age);
        }
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
    let miner = STATE.with(|state| {
        let mut state = state.borrow_mut();
        advance_freshness(&mut state, delta_ms);
        state.miner.clone()
    });

    let root = analog::round::render(
        now,
        &params,
        variant,
        w,
        h,
        effective_tz.as_ref(),
        &palette,
        &miner,
    );

    let _ = render_ui(w, h, root);
    // Re-render once per second so the displayed time advances.
    request_frame_after(1000);
}

/// Fires after every per-widget params delivery (operator change).
/// Trigger an immediate re-render so operator changes don't wait for
/// the next 1s tick.
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
    request_frame();
}

/// Fires after every deck-wide system snapshot delivery
/// (timezone, formats, next-alarm, night-mode, …).
///
/// Same reason for immediate re-render — night-mode flips
/// shouldn't sit on screen for up to a second before
/// the palette swap takes effect.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}

#[cfg(test)]
mod tests {
    use super::*;

    const _: () = {
        assert!(STATS_REFRESH_MS < miner::STALE_AFTER_MS);
        assert!(HASHBOARDS_REFRESH_MS < miner::STALE_AFTER_MS);
    };
}
