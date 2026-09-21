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

#[cfg(not(target_arch = "wasm32"))]
use bmc_wasm_sdk::{Draw, Easing};

#[cfg(target_arch = "wasm32")]
use manifest_params::Params;
#[cfg(target_arch = "wasm32")]
use manifest_params::credentials as slots;
#[cfg(any(target_arch = "wasm32", test))]
use miner::MinerData;
#[cfg(target_arch = "wasm32")]
use mining::bos;
#[cfg(any(target_arch = "wasm32", test))]
use mining::bos::AuthMode;
#[cfg(target_arch = "wasm32")]
use mining::bos::{AuthState, Placeholders, ReplyAction};
#[cfg(target_arch = "wasm32")]
use shared::clock_palette;

#[cfg(target_arch = "wasm32")]
const STATS_REFRESH_MS: u32 = 5_000;

#[cfg(target_arch = "wasm32")]
const STATS_POLL: usize = 1;
#[cfg(target_arch = "wasm32")]
const CONSTRAINTS_POLL: usize = 2;

/// Widgets get no entering/visible lifecycle hook, so a render gap this long
/// stands in for one: the hands snap to the current time
/// instead of sweeping from where the page left them.
/// Keyed on the render delta rather than wall-clock time,
/// so a DST or NTP step still sweeps the hands.
const MAX_ANIMATED_RENDER_GAP_MS: u32 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClockHandTransition {
    Animate,
    Snap,
}

impl ClockHandTransition {
    fn for_render_gap(delta_ms: u32) -> Self {
        if delta_ms > MAX_ANIMATED_RENDER_GAP_MS {
            Self::Snap
        } else {
            Self::Animate
        }
    }

    fn apply(self, draw: Draw, id: &str, duration_ms: u32) -> Draw {
        let duration_ms = match self {
            Self::Animate => duration_ms,
            Self::Snap => 0,
        };
        draw.transition(id, duration_ms, Easing::EaseOut)
    }
}

// Re-poll delay for the one-shot polls: constraints on
// an empty reply, and the login after one it could not use.
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
    assert_eq!(
        (stats.index(), constraints.index()),
        (STATS_POLL, CONSTRAINTS_POLL),
        "BUG: poll order drifted from miner_source"
    );
    HANDLES.with(|handles| {
        *handles.borrow_mut() = Some(Handles {
            login,
            stats,
            constraints,
        });
    });
    request_frame();
}

// Mapped by registration index, not through `HANDLES`: `register_poll`
// runs the builder before `init` can store the handle it returns.
#[cfg(target_arch = "wasm32")]
fn miner_source(handle: PollHandle) -> MinerSource {
    match handle.index() {
        STATS_POLL => MinerSource::Stats,
        CONSTRAINTS_POLL => MinerSource::Constraints,
        _ => panic!("BUG: mining-clock unknown miner poll handle"),
    }
}

#[cfg(target_arch = "wasm32")]
fn auth_mode() -> AuthMode {
    let bound = credentials::current();
    let local_bound = bound.is_bound("bos_local");
    let remote_bound = bound.is_bound("bos_remote");
    AuthMode::derive(
        local_bound,
        remote_bound,
        &Params::current().miner_url,
        Placeholders {
            token: slots::bos_local::TOKEN,
            username: slots::bos_remote::USERNAME,
            password: slots::bos_remote::PASSWORD,
        },
    )
}

#[cfg(target_arch = "wasm32")]
fn build_login(_handle: PollHandle) -> Option<FetchSpec> {
    let (url, body) = auth_mode().login()?;
    Some(
        FetchSpec::post(url)
            .headers("Content-Type: application/json")
            .body(body.as_bytes())
            .timeout(MINER_FETCH_TIMEOUT),
    )
}

#[cfg(target_arch = "wasm32")]
fn build_miner(handle: PollHandle) -> Option<FetchSpec> {
    let path = match miner_source(handle) {
        MinerSource::Stats => bos::STATS_PATH,
        MinerSource::Constraints => bos::CONSTRAINTS_PATH,
    };
    let auth = STATE.with(|state| state.borrow().auth.clone());
    let (url, header) = auth_mode().miner_request(&auth, path)?;
    Some(
        FetchSpec::get(url)
            .headers(header)
            .timeout(MINER_FETCH_TIMEOUT),
    )
}

// Deliberately requests no frame: the clock paints once per second
// (`request_frame_after(1000)` in `render`), and refreshed
// auth state surfaces on the next tick.
//
// Forcing a frame here would paint at a sub-second offset and reset
// the 1s cadence, so the second hand stops advancing in even steps.
#[cfg(target_arch = "wasm32")]
fn on_login_reply(handle: PollHandle, response: &FetchResponse) {
    if response.ok()
        && let Some(token) = bos::parse_token(&response.json())
    {
        STATE.with(|state| state.borrow_mut().auth = AuthState::Authenticated(token));
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                handles.stats.invalidate();
                handles.constraints.invalidate();
            }
        });
    } else if bos::login_refused(response.outcome()) {
        log_warn!(
            "mining-clock: login refused with status {}",
            response.status
        );
        STATE.with(|state| state.borrow_mut().auth = AuthState::Failed);
        // The queued requests carry the token this reply just rejected,
        // so they are dropped rather than left to 401.
        //
        // That makes the retry below the only thing re-arming
        // this one-shot login, where those 401s used to.
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                handles.stats.invalidate();
                handles.constraints.invalidate();
            }
        });
        handle.retry_after(RETRY_MS);
    } else {
        log_warn!(
            "mining-clock: login got no answer, status {}",
            response.status
        );
        STATE.with(|state| state.borrow_mut().auth = AuthState::Unreachable);
        handle.retry_after(RETRY_MS);
    }
}

// Requests no frame for the same reason as `on_login_reply`:
// fresh stats and constraints land on the next 1s render tick.
//
// Painting on every fetch reply would knock
// the second hand off its even one-second steps.
#[cfg(target_arch = "wasm32")]
fn on_miner_reply(handle: PollHandle, response: &FetchResponse) {
    let current = STATE.with(|state| state.borrow().auth.clone());
    match auth_mode().reply_action(&current, response.status) {
        // Local mode has no login to re-arm; the next tick paints the banner.
        ReplyAction::SetAuth(auth) => STATE.with(|state| state.borrow_mut().auth = auth),
        ReplyAction::Relogin => {
            STATE.with(|state| state.borrow_mut().auth = AuthState::LoggingIn);
            HANDLES.with(|handles| {
                if let Some(handles) = handles.borrow().as_ref() {
                    handles.login.invalidate();
                }
            });
        }
        ReplyAction::Keep => {}
    }
    if response.status == 401 {
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
    Unbound,
    Ambiguous,
    Auth,
    Stale,
    None,
}

// A binding prompt outranks everything,
// since nothing is fetched without a usable slot.
// Auth error outranks stale; stale needs loaded data
// (a never-connected miner reads as N/A, not stale).
// The render fills the stale anchor from the poll.
#[cfg(any(target_arch = "wasm32", test))]
fn select_overlay(
    mode: &AuthMode,
    auth_failed: bool,
    stale: bool,
    miner: &MinerData,
) -> OverlaySelect {
    let has_data = miner.hashrate_ths.is_some() || miner.power_w.is_some();
    match mode {
        AuthMode::Unbound => OverlaySelect::Unbound,
        AuthMode::Ambiguous => OverlaySelect::Ambiguous,
        AuthMode::Local { .. } | AuthMode::Remote { .. } if auth_failed => OverlaySelect::Auth,
        AuthMode::Local { .. } | AuthMode::Remote { .. } if stale && has_data => {
            OverlaySelect::Stale
        }
        AuthMode::Local { .. } | AuthMode::Remote { .. } => OverlaySelect::None,
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
    let hand_transition = ClockHandTransition::for_render_gap(delta_ms);
    let (miner, auth_failed) = STATE.with(|state| {
        let state = state.borrow();
        (state.miner.clone(), matches!(state.auth, AuthState::Failed))
    });
    let (stale, anchor) = HANDLES.with(|handles| {
        handles.borrow().as_ref().map_or((false, None), |handles| {
            (handles.stats.is_stale(), handles.stats.last_success_time())
        })
    });
    let overlay = match select_overlay(&auth_mode(), auth_failed, stale, &miner) {
        OverlaySelect::Unbound => Some(mining::overlay::OverlayKind::Unbound),
        OverlaySelect::Ambiguous => Some(mining::overlay::OverlayKind::Ambiguous),
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
        hand_transition,
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

// Blanks the readings while installing the freshly derived mode,
// so one miner's figures are never shown under another's address.
#[cfg(target_arch = "wasm32")]
fn reauthenticate() {
    let auth = auth_mode().initial_auth();
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.miner = MinerData::default();
        state.auth = auth;
    });
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            handles.login.invalidate();
            handles.stats.invalidate();
            handles.constraints.invalidate();
        }
    });
}

/// Deliberately requests no frame: the next 1 s tick shows the new values,
/// and an off-cadence repaint makes the second hand skip its even steps.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let prev = Params::previous();
    let url_changed = prev
        .as_ref()
        .is_none_or(|prev| Params::current().changed_keys(prev).contains(&"miner_url"));
    if url_changed && matches!(auth_mode(), AuthMode::Remote { .. }) {
        reauthenticate();
    }
}

/// Same cadence rule as the other callbacks: no frame request.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_credentials_update() {
    reauthenticate();
}

#[cfg(test)]
mod tests {
    use super::{ClockHandTransition, OverlaySelect, miner::MinerData, select_overlay};
    use bmc_wasm_sdk::{Draw, Easing, WHITE};
    use mining::bos::{AuthMode, Placeholders};

    const PLACEHOLDERS: Placeholders = Placeholders {
        token: "t",
        username: "u",
        password: "p",
    };

    fn mode(local: bool, remote: bool) -> AuthMode {
        AuthMode::derive(local, remote, "http://10.0.0.5/api/v1", PLACEHOLDERS)
    }

    #[test]
    fn hand_transitions_animate_through_five_second_render_gaps() {
        for delta_ms in [0, 1_000, 4_999, 5_000] {
            assert_eq!(
                ClockHandTransition::for_render_gap(delta_ms),
                ClockHandTransition::Animate
            );
        }
    }

    #[test]
    fn hand_transitions_snap_after_longer_render_gaps() {
        for delta_ms in [5_001, 30_000, u32::MAX] {
            assert_eq!(
                ClockHandTransition::for_render_gap(delta_ms),
                ClockHandTransition::Snap
            );
        }
    }

    #[test]
    fn snapping_preserves_hand_identity_and_resumes_normal_duration() {
        for (id, duration_ms) in [
            ("hour-hand", 500),
            ("minute-hand", 500),
            ("second-hand", 200),
        ] {
            let mut identity = None;
            for (delta_ms, expected_duration) in
                [(1_000, duration_ms), (5_001, 0), (1_000, duration_ms)]
            {
                let draw = ClockHandTransition::for_render_gap(delta_ms).apply(
                    Draw::rect(0.0, 0.0, 1.0, 1.0, WHITE),
                    id,
                    duration_ms,
                );
                let Draw::Modified {
                    transition: Some(transition),
                    ..
                } = draw
                else {
                    panic!("each hand must retain its transition across a render gap");
                };
                assert_eq!(transition.duration_ms, expected_duration);
                assert_eq!(transition.easing, Easing::EaseOut);
                assert_eq!(
                    *identity.get_or_insert(transition.id_hash),
                    transition.id_hash
                );
            }
        }
    }

    #[test]
    fn auth_overlay_takes_precedence_over_stale_data() {
        let miner = MinerData {
            hashrate_ths: Some(122.48),
            ..MinerData::default()
        };

        assert_eq!(
            select_overlay(&mode(true, false), true, true, &miner),
            OverlaySelect::Auth
        );
    }

    #[test]
    fn stale_overlay_requires_loaded_miner_data() {
        assert_eq!(
            select_overlay(&mode(true, false), false, true, &MinerData::default()),
            OverlaySelect::None
        );

        let miner = MinerData {
            power_w: Some(41.0),
            ..MinerData::default()
        };

        assert_eq!(
            select_overlay(&mode(true, false), false, true, &miner),
            OverlaySelect::Stale
        );
    }

    /// With no usable slot nothing is fetched, so a stale pill
    /// or an auth banner would describe requests that never happen.
    #[test]
    fn a_binding_prompt_outranks_auth_and_stale() {
        let miner = MinerData {
            hashrate_ths: Some(122.48),
            ..MinerData::default()
        };
        assert_eq!(
            select_overlay(&mode(false, false), true, true, &miner),
            OverlaySelect::Unbound
        );
        assert_eq!(
            select_overlay(&mode(true, true), true, true, &miner),
            OverlaySelect::Ambiguous
        );
        assert_eq!(
            select_overlay(&mode(false, true), true, true, &miner),
            OverlaySelect::Auth
        );
    }
}
