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

//! Polling runtime shared by the miner widgets: the endpoint tables,
//! the BOS login and refresh loops, and the overlay a face draws over its data.
//!
//! The SDK's poll callbacks are bare `fn` pointers with no user data,
//! so the runtime cannot capture a widget's configuration.
//! Each widget installs a [`ConfigFn`] at [`init`] instead.

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget runtime code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
use crate::layout;
use crate::layout::Panel;
#[cfg(target_arch = "wasm32")]
use crate::model::{Currency, MinerData, PublicData, Verdict};
#[cfg(target_arch = "wasm32")]
use crate::{api as miner_api, model, public as public_api};
#[cfg(target_arch = "wasm32")]
use mining::bos;
use mining::bos::AuthMode;
#[cfg(target_arch = "wasm32")]
use mining::bos::{AuthState, ReplyAction};

/// Which face a widget draws, and with it which endpoints are worth fetching.
///
/// The endpoint tables are keyed on this type.
/// A widget's generated `manifest_params::View` cannot serve,
/// since a shared table cannot name a type that each crate regenerates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
    Mining,
    Geek,
    InfoOverload,
}

impl View {
    /// Whether the face is the hashrate gauge on this panel.
    #[must_use]
    pub const fn draws_gauge(self, panel: Panel) -> bool {
        matches!(self, Self::Mining | Self::Geek) && matches!(panel, Panel::Round)
    }
}

/// What the runtime needs from the widget on every callback.
#[derive(Clone, Debug)]
pub struct Config {
    pub auth: AuthMode,
    pub view: View,
}

/// Reads the hosting widget's current parameters.
/// Installed once by [`init`].
pub type ConfigFn = fn() -> Config;

/// Which inputs a parameter update moved.
/// The widget derives this from its own generated `Params`,
/// whose key names the runtime has no way to know.
#[derive(Clone, Copy, Debug, Default)]
pub struct Changed {
    pub miner_url: bool,
    pub currency: bool,
}

#[cfg(target_arch = "wasm32")]
const MINER_REFRESH_MS: u32 = 5_000;
#[cfg(target_arch = "wasm32")]
const PUBLIC_REFRESH_MS: u32 = 60_000;
#[cfg(target_arch = "wasm32")]
const RETRY_MS: u32 = 10_000;
// The miner lives on the local network,
// so an unreachable one should fail fast
// instead of holding the SDK-default 10s timeout.
// Public API fetches keep the default.
#[cfg(target_arch = "wasm32")]
const MINER_FETCH_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(target_arch = "wasm32")]
const MAX_LOGIN_RETRY_MS: u32 = 300_000;

#[cfg(target_arch = "wasm32")]
type MinerParser = fn(&JsonDoc, &mut MinerData) -> Verdict;
#[cfg(target_arch = "wasm32")]
type PublicUrl = fn(Currency) -> String;
#[cfg(target_arch = "wasm32")]
type PublicParser = fn(&JsonDoc, Currency, &mut PublicData) -> Verdict;
#[cfg(target_arch = "wasm32")]
type PublicReset = fn(&mut PublicData);

/// Which faces read an endpoint.
#[cfg(any(target_arch = "wasm32", test))]
type Reads = fn(View, Panel) -> bool;

// Host-pure, unlike the tables, so the tests can pin the matrix.
#[cfg(any(target_arch = "wasm32", test))]
mod reads {
    use super::{Panel, View};
    use crate::layout::{InfoOverloadFields, info_overload_fields};

    // An endpoint the grid hides on a panel is not read there: a failure
    // on it would otherwise raise a banner over a face missing nothing.
    fn overload_shows(view: View, panel: Panel, field: fn(InfoOverloadFields) -> bool) -> bool {
        view == View::InfoOverload && field(info_overload_fields(panel))
    }

    pub(super) fn details(view: View, _: Panel) -> bool {
        matches!(view, View::Geek | View::InfoOverload)
    }

    pub(super) fn stats(_: View, _: Panel) -> bool {
        true
    }

    pub(super) fn hashboards(view: View, _: Panel) -> bool {
        matches!(view, View::Mining | View::Geek)
    }

    pub(super) fn cooling(view: View, _: Panel) -> bool {
        view == View::Mining
    }

    pub(super) fn network(view: View, _: Panel) -> bool {
        matches!(view, View::Mining | View::Geek)
    }

    // The tuner target anchors the ring, so only a gauge face reads it.
    pub(super) fn constraints(view: View, panel: Panel) -> bool {
        view.draws_gauge(panel)
    }

    pub(super) fn price_stats(view: View, _: Panel) -> bool {
        matches!(view, View::Geek | View::InfoOverload)
    }

    // Block height is on every Overload grid.
    pub(super) fn block(view: View, _: Panel) -> bool {
        view == View::InfoOverload
    }

    pub(super) fn difficulty_stats(view: View, panel: Panel) -> bool {
        overload_shows(view, panel, |fields| fields.show_difficulty_row)
    }

    pub(super) fn hashrate_stats(view: View, panel: Panel) -> bool {
        overload_shows(view, panel, |fields| {
            fields.show_hashvalue || fields.show_fee_percent
        })
    }

    pub(super) fn price_history(view: View, panel: Panel) -> bool {
        overload_shows(view, panel, |fields| fields.show_price_graph)
    }
}

// An authenticated GET paired with the parser folding its response
// into `MinerData`. Endpoints are independent once a token exists,
// so each runs its own refresh loop rather than chaining.
//
// A `None` interval means one-shot, refetched
// only when the login invalidates it.
#[cfg(target_arch = "wasm32")]
struct MinerEndpoint {
    path: &'static str,
    parse: MinerParser,
    needed: Reads,
    interval_ms: Option<u32>,
}

#[cfg(target_arch = "wasm32")]
const MINER_ENDPOINTS: [MinerEndpoint; 6] = [
    MinerEndpoint {
        path: bos::DETAILS_PATH,
        parse: miner_details,
        needed: reads::details,
        interval_ms: Some(MINER_REFRESH_MS),
    },
    MinerEndpoint {
        path: bos::STATS_PATH,
        parse: miner_stats,
        needed: reads::stats,
        interval_ms: Some(MINER_REFRESH_MS),
    },
    MinerEndpoint {
        path: bos::HASHBOARDS_PATH,
        parse: miner_hashboards,
        needed: reads::hashboards,
        interval_ms: Some(MINER_REFRESH_MS),
    },
    MinerEndpoint {
        path: bos::COOLING_PATH,
        parse: miner_cooling,
        needed: reads::cooling,
        interval_ms: Some(MINER_REFRESH_MS),
    },
    MinerEndpoint {
        path: bos::NETWORK_PATH,
        parse: miner_network,
        needed: reads::network,
        interval_ms: Some(MINER_REFRESH_MS),
    },
    // One-shot because constraints change only on a re-tune.
    MinerEndpoint {
        path: bos::CONSTRAINTS_PATH,
        parse: miner_constraints,
        needed: reads::constraints,
        interval_ms: None,
    },
];

// Unauthenticated and fully independent;
// each builds its URL from the selected currency and parses into `PublicData`.
// Only the fiat price and hashprice move with the currency,
// so a currency change refetches those alone.
#[cfg(target_arch = "wasm32")]
struct PublicEndpoint {
    url: PublicUrl,
    parse: PublicParser,
    reset: PublicReset,
    needed: Reads,
    currency_dependent: bool,
}

#[cfg(target_arch = "wasm32")]
const PUBLIC_ENDPOINTS: [PublicEndpoint; 5] = [
    PublicEndpoint {
        url: public_api::price_stats_url,
        parse: public_price,
        reset: public_api::reset_price_stats,
        needed: reads::price_stats,
        currency_dependent: true,
    },
    PublicEndpoint {
        url: public_api::block_url,
        parse: public_block,
        reset: public_api::reset_block,
        needed: reads::block,
        currency_dependent: false,
    },
    PublicEndpoint {
        url: public_api::difficulty_url,
        parse: public_difficulty,
        reset: public_api::reset_difficulty_stats,
        needed: reads::difficulty_stats,
        currency_dependent: false,
    },
    PublicEndpoint {
        url: public_api::hashrate_url,
        parse: public_hashrate,
        reset: public_api::reset_hashrate_stats,
        needed: reads::hashrate_stats,
        currency_dependent: true,
    },
    PublicEndpoint {
        url: public_api::price_history_url,
        parse: public_history,
        reset: public_api::reset_price_history,
        needed: reads::price_history,
        currency_dependent: false,
    },
];

#[cfg(any(target_arch = "wasm32", test))]
fn offline_label(miner: bool, public: bool) -> Option<&'static str> {
    match (miner, public) {
        (true, true) => Some("Failed to load: Miner, Network"),
        (true, false) => Some("Failed to load: Miner"),
        (false, true) => Some("Failed to load: Network"),
        (false, false) => None,
    }
}

#[cfg(target_arch = "wasm32")]
fn miner_endpoint_needed(idx: usize, view: View, panel: Panel) -> bool {
    (MINER_ENDPOINTS[idx].needed)(view, panel)
}

#[cfg(target_arch = "wasm32")]
fn public_endpoint_needed(idx: usize, view: View, panel: Panel) -> bool {
    (PUBLIC_ENDPOINTS[idx].needed)(view, panel)
}

#[cfg(target_arch = "wasm32")]
fn panel() -> Panel {
    layout::classify(widget_viewport())
}

#[cfg(target_arch = "wasm32")]
fn miner_details(json: &JsonDoc, data: &mut MinerData) -> Verdict {
    miner_api::parse_details(json).stored(|details| data.uptime = details.uptime.into())
}
#[cfg(target_arch = "wasm32")]
fn miner_stats(json: &JsonDoc, data: &mut MinerData) -> Verdict {
    miner_api::parse_stats(json).stored(|stats| {
        data.hashrate = stats.hashrate.into();
        data.power = stats.power.into();
        data.efficiency = stats.efficiency.into();
    })
}
#[cfg(target_arch = "wasm32")]
fn miner_hashboards(json: &JsonDoc, data: &mut MinerData) -> Verdict {
    miner_api::parse_hashboards(json).stored(|boards| {
        data.temperature = boards.temperature.into();
        data.mcr = boards.mcr.into();
        data.chip_type = boards.chip_type.into();
        data.chip_count = boards.chip_count.into();
    })
}
#[cfg(target_arch = "wasm32")]
fn miner_cooling(json: &JsonDoc, data: &mut MinerData) -> Verdict {
    miner_api::parse_cooling(json).stored(|cooling| data.fan_speed = cooling.fan_speed.into())
}
#[cfg(target_arch = "wasm32")]
fn miner_network(json: &JsonDoc, data: &mut MinerData) -> Verdict {
    miner_api::parse_network(json).stored(|network| data.ip_address = network.ip_address.into())
}
#[cfg(target_arch = "wasm32")]
fn miner_constraints(json: &JsonDoc, data: &mut MinerData) -> Verdict {
    miner_api::parse_constraints(json).stored(|constraints| data.constraints = constraints)
}
#[cfg(target_arch = "wasm32")]
fn public_price(json: &JsonDoc, currency: Currency, data: &mut PublicData) -> Verdict {
    public_api::parse_price_stats(json, currency).stored(|price| {
        data.btc_price = price.btc_price.into();
        data.btc_change_24h = price.btc_change_24h.into();
    })
}
#[cfg(target_arch = "wasm32")]
fn public_block(json: &JsonDoc, _currency: Currency, data: &mut PublicData) -> Verdict {
    public_api::parse_block(json).stored(|block| data.block_height = block.block_height.into())
}
#[cfg(target_arch = "wasm32")]
fn public_difficulty(json: &JsonDoc, _currency: Currency, data: &mut PublicData) -> Verdict {
    public_api::parse_difficulty_stats(json).stored(|difficulty| {
        data.prev_diff_adjust = difficulty.prev_diff_adjust.into();
        data.est_diff_adjust = difficulty.est_diff_adjust.into();
        data.epoch_progress = difficulty.epoch_progress.into();
    })
}
#[cfg(target_arch = "wasm32")]
fn public_hashrate(json: &JsonDoc, _currency: Currency, data: &mut PublicData) -> Verdict {
    public_api::parse_hashrate_stats(json).stored(|hashrate| {
        data.avg_fee_share = hashrate.avg_fee_share.into();
        data.hashvalue = hashrate.hashvalue.into();
    })
}
#[cfg(target_arch = "wasm32")]
fn public_history(json: &JsonDoc, _currency: Currency, data: &mut PublicData) -> Verdict {
    public_api::parse_price_history(json).stored(|history| data.btc_price_history = history.points)
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct State {
    miner: MinerData,
    public: PublicData,
    auth: AuthState,
    // Drives the retry backoff; reset by a successful login or a credential change.
    login_failures: u32,
}

// Backs off a persistently wrong login — e.g. a 2xx carrying no token —
// which would otherwise hammer `/auth/login` at `base_ms` forever.
#[cfg(any(target_arch = "wasm32", test))]
fn login_retry_delay(failures: u32, base_ms: u32, cap_ms: u32) -> u32 {
    match 1_u32.checked_shl(failures) {
        Some(multiplier) => base_ms.saturating_mul(multiplier).min(cap_ms),
        None => cap_ms,
    }
}

// Registered in the order login, miner, then public,
// so each group holds a contiguous index range
// and a handle index maps to a table index by subtracting the group's base.
#[cfg(target_arch = "wasm32")]
struct Handles {
    login: PollHandle,
    miner: [PollHandle; MINER_ENDPOINTS.len()],
    public: [PollHandle; PUBLIC_ENDPOINTS.len()],
}

#[cfg(target_arch = "wasm32")]
const MINER_BASE: usize = 1;
#[cfg(target_arch = "wasm32")]
const PUBLIC_BASE: usize = MINER_BASE + MINER_ENDPOINTS.len();

#[cfg(target_arch = "wasm32")]
fn miner_index(handle: PollHandle) -> usize {
    handle.index() - MINER_BASE
}

#[cfg(target_arch = "wasm32")]
fn public_index(handle: PollHandle) -> usize {
    handle.index() - PUBLIC_BASE
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static HANDLES: RefCell<Option<Handles>> = const { RefCell::new(None) };
    static CONFIG: std::cell::Cell<Option<ConfigFn>> = const { std::cell::Cell::new(None) };
    static FIRST_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(target_arch = "wasm32")]
fn config() -> Config {
    let read = CONFIG.with(std::cell::Cell::get);
    read.expect("BUG: the runtime was driven before `init` installed a ConfigFn")()
}

/// Whether this is the widget's first frame, consuming the flag.
///
/// The gauge seeds from empty on that frame,
/// so the host animates the fill in even when miner data already arrived.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn take_first_frame() -> bool {
    FIRST_FRAME.replace(false)
}

/// The data a face draws, plus where the login stands.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn frame() -> (MinerData, PublicData, AuthState) {
    STATE.with(|state| {
        let state = state.borrow();
        (
            state.miner.clone(),
            state.public.clone(),
            state.auth.clone(),
        )
    })
}

// The login serves every miner endpoint, so it is driven at source granularity
// and gates the auth banner; the endpoints themselves gate per face.
#[cfg(target_arch = "wasm32")]
fn view_needs_miner(view: View, panel: Panel) -> bool {
    MINER_ENDPOINTS
        .iter()
        .any(|endpoint| (endpoint.needed)(view, panel))
}

// Blank the miner data and start over under the current mode.
// Blanked data drops its staleness — no pill over the new miner's N/A —
// and its in-flight requests, whose replies would otherwise
// refill the fields just cleared.
#[cfg(target_arch = "wasm32")]
fn reauthenticate(handles: &Handles) {
    let auth = config().auth.initial_auth();
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        miner_api::reset_all(&mut state.miner);
        state.login_failures = 0;
        state.auth = auth;
    });
    for miner in &handles.miner {
        miner.reset_staleness();
        miner.invalidate();
    }
    handles.login.invalidate();
}

#[cfg(target_arch = "wasm32")]
const fn selected_currency() -> Currency {
    model::CURRENCY
}

/// Register the login, miner and public polls,
/// and install the widget's parameter reader.
/// Call once from the widget's `init` export.
#[cfg(target_arch = "wasm32")]
pub fn init(read_config: ConfigFn) {
    CONFIG.with(|slot| slot.set(Some(read_config)));
    let view = config().view;
    let panel = panel();
    let login = register_poll(
        build_login,
        on_login_reply,
        PollConfig {
            enabled: view_needs_miner(view, panel),
            ..Default::default()
        },
    );
    let miner = std::array::from_fn(|idx| {
        register_poll(
            build_miner,
            on_miner_reply,
            PollConfig {
                interval_ms: MINER_ENDPOINTS[idx].interval_ms,
                enabled: miner_endpoint_needed(idx, view, panel),
                ..Default::default()
            },
        )
    });
    let public = std::array::from_fn(|idx| {
        register_poll(
            build_public,
            on_public_reply,
            PollConfig {
                interval_ms: Some(PUBLIC_REFRESH_MS),
                enabled: public_endpoint_needed(idx, view, panel),
                ..Default::default()
            },
        )
    });
    HANDLES.with(|handles| {
        *handles.borrow_mut() = Some(Handles {
            login,
            miner,
            public,
        });
    });
    request_frame();
}

// Enable the endpoints the view reads and disable the rest,
// re-authenticating when remote mode's URL moves.
// The login serves every miner endpoint,
// so it is gated on whether any of them feeds the view rather than per-endpoint.
#[cfg(target_arch = "wasm32")]
pub fn on_params_update(changed: Changed) {
    let view = config().view;
    let panel = panel();

    HANDLES.with(|handles| {
        let handles = handles.borrow();
        let Some(handles) = handles.as_ref() else {
            return;
        };
        if view_needs_miner(view, panel) {
            handles.login.set_enabled(true);
            // Only remote mode dials the URL param; local mode ignores it.
            if changed.miner_url && matches!(config().auth, AuthMode::Remote { .. }) {
                reauthenticate(handles);
            }
            for (idx, miner) in handles.miner.iter().enumerate() {
                miner.set_enabled(miner_endpoint_needed(idx, view, panel));
            }
        } else {
            STATE.with(|state| state.borrow_mut().auth = AuthState::NoToken);
            handles.login.set_enabled(false);
            for miner in &handles.miner {
                miner.set_enabled(false);
            }
        }
        for (idx, public) in handles.public.iter().enumerate() {
            let needed = public_endpoint_needed(idx, view, panel);
            public.set_enabled(needed);
            if needed && changed.currency && PUBLIC_ENDPOINTS[idx].currency_dependent {
                STATE.with(|state| {
                    (PUBLIC_ENDPOINTS[idx].reset)(&mut state.borrow_mut().public);
                });
                public.invalidate();
            }
        }
    });
    request_frame();
}

/// A slot was bound, unbound or rotated:
/// derive the mode again and refetch everything under it.
/// A face reading no miner endpoint polls nothing;
/// it picks the binding up when `on_params_update` re-enables the polls.
#[cfg(target_arch = "wasm32")]
pub fn on_credentials_update() {
    if !view_needs_miner(config().view, panel()) {
        return;
    }
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            reauthenticate(handles);
        }
    });
    request_frame();
}

// `None` keeps the poll dormant while the miner source is hidden
// or the mode has no login. The poll is one-shot,
// so re-auth happens by invalidating the handle rather than looping.
#[cfg(target_arch = "wasm32")]
fn build_login(_handle: PollHandle) -> Option<FetchSpec> {
    let params = config();
    if !view_needs_miner(params.view, panel()) {
        return None;
    }
    let (url, body) = params.auth.login()?;
    Some(
        FetchSpec::post(url)
            .headers("Content-Type: application/json")
            .body(body.as_bytes())
            .timeout(MINER_FETCH_TIMEOUT),
    )
}

// `None` while the mode has nothing to send —
// remote mode holds no token yet, or no slot picks a mode —
// so the poll stays dormant until a login succeeds and invalidates it.
#[cfg(target_arch = "wasm32")]
fn build_miner(handle: PollHandle) -> Option<FetchSpec> {
    let auth = STATE.with(|state| state.borrow().auth.clone());
    let (url, header) = config()
        .auth
        .miner_request(&auth, MINER_ENDPOINTS[miner_index(handle)].path)?;
    Some(
        FetchSpec::get(url)
            .headers(header)
            .timeout(MINER_FETCH_TIMEOUT),
    )
}

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "signature must match the poll Build fn pointer, which returns Option"
)]
fn build_public(handle: PollHandle) -> Option<FetchSpec> {
    let url = (PUBLIC_ENDPOINTS[public_index(handle)].url)(selected_currency());
    Some(FetchSpec::get(url))
}

// A token invalidates the miner endpoints,
// so the ones the view needs refetch with it.
// A refusal raises the auth overlay, anything else the offline one,
// and either way the one-shot login re-arms rather than wedging.
#[cfg(target_arch = "wasm32")]
fn on_login_reply(handle: PollHandle, response: &FetchResponse) {
    if response.ok()
        && let Some(token) = bos::parse_token(&response.json())
    {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.auth = AuthState::Authenticated(token);
            state.login_failures = 0;
        });
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                for miner in &handles.miner {
                    miner.invalidate();
                }
            }
        });
    } else if bos::login_refused(response.outcome()) {
        log_warn!("login refused with status {}", response.status);
        let delay = STATE.with(|state| {
            let mut state = state.borrow_mut();
            miner_api::reset_all(&mut state.miner);
            state.auth = AuthState::Failed;
            let delay = login_retry_delay(state.login_failures, RETRY_MS, MAX_LOGIN_RETRY_MS);
            state.login_failures = state.login_failures.saturating_add(1);
            delay
        });
        // Blanked data drops its staleness — auth banner over N/A, not a stale pill.
        // Invalidating drops the queued requests with it: each carries the token
        // this reply just rejected, and its 401 would re-fire the backed-off login.
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                for miner in &handles.miner {
                    miner.reset_staleness();
                    miner.invalidate();
                }
            }
        });
        handle.retry_after(delay);
    } else {
        log_warn!("login got no answer, status {}", response.status);
        STATE.with(|state| state.borrow_mut().auth = AuthState::Unreachable);
        handle.retry_after(RETRY_MS);
    }
    request_frame();
}

// A 401 in remote mode means the session was rejected:
// drop it and invalidate the login handle,
// so one re-auth covers every endpoint that hit the same wall;
// this poll then goes dormant until that login reinvalidates it.
//
// In local mode the reply alone raises the auth banner
// and a later success lowers it; polling continues.
//
// Any other failure keeps the last good data, flagging it stale.
#[cfg(target_arch = "wasm32")]
fn on_miner_reply(handle: PollHandle, response: &FetchResponse) {
    let current = STATE.with(|state| state.borrow().auth.clone());
    let action = config().auth.reply_action(&current, response.status);
    match &action {
        ReplyAction::SetAuth(auth) => {
            STATE.with(|state| state.borrow_mut().auth = auth.clone());
        }
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
        // A banner the reply itself raised needs a repaint now;
        // a re-login paints when it answers.
        if matches!(action, ReplyAction::SetAuth(_)) {
            request_frame();
        }
        return;
    }
    let idx = miner_index(handle);
    if response.ok() {
        // A captive portal answers 200 with HTML, which no parser can judge.
        let json = response.json();
        let verdict = if json.is_valid() {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                (MINER_ENDPOINTS[idx].parse)(&json, &mut state.miner)
            })
        } else {
            Verdict::Unusable
        };
        // No answer is no refresh: the reading goes stale instead of banking,
        // and the next attempt keeps the endpoint's own cadence.
        if verdict == Verdict::Unusable {
            log_warn!(
                "miner endpoint {} returned no usable data",
                MINER_ENDPOINTS[idx].path
            );
            handle.retry_after(MINER_ENDPOINTS[idx].interval_ms.unwrap_or(RETRY_MS));
        }
    } else {
        log_warn!(
            "miner endpoint {} failed with status {}",
            MINER_ENDPOINTS[idx].path,
            response.status
        );
    }
    request_frame();
}

// A failed refresh keeps the last good data rather than blanking the fields,
// flagging the endpoint stale instead.
// The "this data is now wrong" case is a currency change,
// which `on_params_update` resets deliberately.
#[cfg(target_arch = "wasm32")]
fn on_public_reply(handle: PollHandle, response: &FetchResponse) {
    let idx = public_index(handle);
    if response.ok() {
        let currency = selected_currency();
        let json = response.json();
        let verdict = if json.is_valid() {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                (PUBLIC_ENDPOINTS[idx].parse)(&json, currency, &mut state.public)
            })
        } else {
            Verdict::Unusable
        };
        if verdict == Verdict::Unusable {
            log_warn!("public endpoint {} returned no usable data", idx);
            handle.retry();
        }
    } else {
        log_warn!(
            "public endpoint {} failed with status {}",
            idx,
            response.status
        );
    }
    request_frame();
}

/// Which overlay, if any, belongs over the frame `auth_failed` describes.
///
/// A binding prompt outranks everything:
/// without a usable slot no miner request is made,
/// so nothing else it could say is true.
/// Auth error outranks stale, since both share the corner.
/// Both scans consider only endpoints enabled for the current view,
/// because a disabled endpoint keeps its stale/offline history.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn overlay(view: View, panel: Panel, auth: &AuthState) -> Option<mining::overlay::OverlayKind> {
    let needs_miner = view_needs_miner(view, panel);
    if needs_miner && let Some(binding) = mining::overlay::binding_overlay(&config().auth) {
        Some(binding)
    } else if needs_miner && *auth == AuthState::Failed {
        Some(mining::overlay::OverlayKind::Auth)
    } else {
        HANDLES.with(|handles| {
            let handles = handles.borrow();
            let handles = handles.as_ref()?;
            // Age-stale pill: the oldest anchor among enabled endpoints
            // that loaded once and are now failing.
            let stale = handles
                .miner
                .iter()
                .chain(handles.public.iter())
                .filter(|handle| handle.enabled())
                .filter(|handle| handle.is_stale())
                .filter_map(|handle| handle.last_success_time())
                .min_by_key(|anchor| anchor.unix_secs)
                .map(mining::overlay::OverlayKind::Stale);
            if stale.is_some() {
                return stale;
            }
            // Offline banner: an enabled source that never loaded and is failing.
            // A login with no answer counts as the miner: without a token
            // its endpoints never fire, so they never report failing themselves.
            let miner_offline = (needs_miner && *auth == AuthState::Unreachable)
                || handles
                    .miner
                    .iter()
                    .copied()
                    .filter(|handle| handle.enabled())
                    .any(PollHandle::is_offline);
            let public_offline = handles
                .public
                .iter()
                .copied()
                .filter(|handle| handle.enabled())
                .any(PollHandle::is_offline);
            offline_label(miner_offline, public_offline).map(mining::overlay::OverlayKind::Failed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Panel, Reads, View, login_retry_delay, offline_label, reads};

    #[test]
    fn offline_label_names_only_the_failing_groups() {
        assert_eq!(offline_label(false, false), None);
        assert_eq!(offline_label(true, false), Some("Failed to load: Miner"));
        assert_eq!(offline_label(false, true), Some("Failed to load: Network"));
        assert_eq!(
            offline_label(true, true),
            Some("Failed to load: Miner, Network")
        );
    }

    const ENDPOINTS: [(&str, Reads); 11] = [
        ("details", reads::details),
        ("stats", reads::stats),
        ("hashboards", reads::hashboards),
        ("cooling", reads::cooling),
        ("network", reads::network),
        ("constraints", reads::constraints),
        ("price-stats", reads::price_stats),
        ("block", reads::block),
        ("difficulty-stats", reads::difficulty_stats),
        ("hashrate-stats", reads::hashrate_stats),
        ("price-history", reads::price_history),
    ];

    fn expected_reads(view: View, panel: Panel) -> &'static [&'static str] {
        match (view, panel) {
            (View::Mining, Panel::Small | Panel::Bmm101) => {
                &["stats", "hashboards", "cooling", "network"]
            }
            (View::Mining, Panel::Round) => {
                &["stats", "hashboards", "cooling", "network", "constraints"]
            }
            (View::Geek, Panel::Small | Panel::Bmm101) => {
                &["details", "stats", "hashboards", "network", "price-stats"]
            }
            (View::Geek, Panel::Round) => &[
                "details",
                "stats",
                "hashboards",
                "network",
                "constraints",
                "price-stats",
            ],
            // The small grid shows block height but no difficulty row,
            // hashvalue, fee share or sparkline, so it reads none of theirs.
            (View::InfoOverload, Panel::Small) => &["details", "stats", "price-stats", "block"],
            (View::InfoOverload, Panel::Bmm101 | Panel::Round) => &[
                "details",
                "stats",
                "price-stats",
                "block",
                "difficulty-stats",
                "hashrate-stats",
                "price-history",
            ],
        }
    }

    #[test]
    fn each_face_reads_exactly_its_endpoints() {
        for view in [View::Mining, View::Geek, View::InfoOverload] {
            for panel in [Panel::Small, Panel::Bmm101, Panel::Round] {
                let read: Vec<&str> = ENDPOINTS
                    .iter()
                    .filter(|(_, needed)| needed(view, panel))
                    .map(|(name, _)| *name)
                    .collect();
                assert_eq!(read, expected_reads(view, panel), "{view:?} on {panel:?}");
            }
        }
    }

    #[test]
    fn login_retry_delay_doubles_per_failure_then_caps() {
        let base = 10_000;
        let cap = 300_000;
        assert_eq!(login_retry_delay(0, base, cap), 10_000);
        assert_eq!(login_retry_delay(1, base, cap), 20_000);
        assert_eq!(login_retry_delay(2, base, cap), 40_000);
        assert_eq!(login_retry_delay(4, base, cap), 160_000);
        // 10s << 5 = 320s exceeds the cap, and every larger count stays capped.
        assert_eq!(login_retry_delay(5, base, cap), cap);
        assert_eq!(login_retry_delay(99, base, cap), cap);
    }
}
