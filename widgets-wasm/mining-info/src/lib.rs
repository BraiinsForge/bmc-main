// Copyright (C) 2026  Braiins Systems s.r.o.

mod format;
mod layout;
mod manifest_params;
mod miner_api;
mod model;
mod public_api;
#[cfg(target_arch = "wasm32")]
mod render;

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;
#[cfg(target_arch = "wasm32")]
use manifest_params::{Currency as ParamCurrency, Params, View};
#[cfg(target_arch = "wasm32")]
use miner_api::{AuthState, endpoint};
#[cfg(target_arch = "wasm32")]
use model::{Currency, MinerData, PublicData};
#[cfg(target_arch = "wasm32")]
use render::RenderSize;

#[cfg(target_arch = "wasm32")]
const MINER_REFRESH_MS: u32 = 5_000;
#[cfg(target_arch = "wasm32")]
const PUBLIC_REFRESH_MS: u32 = 60_000;
#[cfg(target_arch = "wasm32")]
const RETRY_MS: u32 = 10_000;
// Settle a burst of rapid parameter changes before refetching: a change schedules
// the request after this delay rather than firing immediately, and the next
// change cancels the still-queued timer and restarts it, so dragging a control or
// typing a URL issues one request when the value settles instead of one per step.
#[cfg(target_arch = "wasm32")]
const DEBOUNCE_MS: u32 = 300;

#[cfg(target_arch = "wasm32")]
type MinerParser = fn(&JsonDoc, &mut MinerData);
#[cfg(target_arch = "wasm32")]
type PublicUrl = fn(Currency) -> String;
#[cfg(target_arch = "wasm32")]
type PublicParser = fn(&JsonDoc, Currency, &mut PublicData);

// Each miner endpoint is an authenticated GET paired with the parser that folds
// its response into `MinerData`. They are mutually independent once a token
// exists, so each runs its own refresh loop rather than chaining.
#[cfg(target_arch = "wasm32")]
const MINER_ENDPOINTS: [(&str, MinerParser); 5] = [
    ("/miner/details", miner_details),
    ("/miner/stats", miner_stats),
    ("/miner/hw/hashboards", miner_hashboards),
    ("/cooling/state", miner_cooling),
    ("/network/", miner_network),
];

// Public Bitcoin endpoints are unauthenticated and fully independent; each
// builds its URL from the selected currency and parses into `PublicData`.
#[cfg(target_arch = "wasm32")]
const PUBLIC_ENDPOINTS: [(PublicUrl, PublicParser); 4] = [
    (public_api::price_stats_url, public_price),
    (public_api::block_url, public_block),
    (public_api::difficulty_url, public_difficulty),
    (public_api::hashrate_url, public_hashrate),
];

#[cfg(target_arch = "wasm32")]
fn miner_details(json: &JsonDoc, data: &mut MinerData) {
    miner_api::parse_details(json, data);
}
#[cfg(target_arch = "wasm32")]
fn miner_stats(json: &JsonDoc, data: &mut MinerData) {
    miner_api::parse_stats(json, data);
}
#[cfg(target_arch = "wasm32")]
fn miner_hashboards(json: &JsonDoc, data: &mut MinerData) {
    miner_api::parse_hashboards(json, data);
}
#[cfg(target_arch = "wasm32")]
fn miner_cooling(json: &JsonDoc, data: &mut MinerData) {
    miner_api::parse_cooling(json, data);
}
#[cfg(target_arch = "wasm32")]
fn miner_network(json: &JsonDoc, data: &mut MinerData) {
    miner_api::parse_network(json, data);
}
#[cfg(target_arch = "wasm32")]
fn public_price(json: &JsonDoc, currency: Currency, data: &mut PublicData) {
    public_api::parse_price_stats(json, currency, data);
}
#[cfg(target_arch = "wasm32")]
fn public_block(json: &JsonDoc, _currency: Currency, data: &mut PublicData) {
    public_api::parse_block(json, data);
}
#[cfg(target_arch = "wasm32")]
fn public_difficulty(json: &JsonDoc, _currency: Currency, data: &mut PublicData) {
    public_api::parse_difficulty_stats(json, data);
}
#[cfg(target_arch = "wasm32")]
fn public_hashrate(json: &JsonDoc, currency: Currency, data: &mut PublicData) {
    public_api::parse_hashrate_stats(json, currency, data);
}

// One outstanding request per endpoint loop, kept single-flight. `live` is the
// request id currently in flight or queued; it routes the response back to its
// slot. A param change that supersedes a request the host can still cancel
// removes it outright, while one already in flight cannot be stopped, so
// `pending` marks it for its callback to discard the stale body and reissue
// with current params rather than racing a second request into a fetch slot.
#[cfg(target_arch = "wasm32")]
#[derive(Default, Clone, Copy)]
struct Slot {
    live: Option<FetchRequestId>,
    pending: bool,
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct Loops {
    login: Slot,
    miner: [Slot; 5],
    public: [Slot; 4],
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct State {
    miner: MinerData,
    public: PublicData,
    auth: AuthState,
    loops: Loops,
}

// Supersede a slot's outstanding request and report whether the slot is now free
// to receive a fresh one. A still-queued request is cancelled (freeing the slot
// immediately); one already in flight cannot be stopped, so the slot is marked
// pending and left for its callback to reissue instead of orphaning the slot.
#[cfg(target_arch = "wasm32")]
fn supersede(slot: &mut Slot) -> bool {
    match slot.live {
        Some(id) if !cancel(id) => {
            slot.pending = true;
            false
        }
        _ => {
            slot.live = None;
            true
        }
    }
}

// Clear a slot whose response just arrived and report whether a param change had
// marked it pending — in which case the caller discards the now-stale body and
// reissues with current params instead of treating the response as fresh data.
#[cfg(target_arch = "wasm32")]
fn take_pending(slot: &mut Slot) -> bool {
    slot.live = None;
    let pending = slot.pending;
    slot.pending = false;
    pending
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[cfg(target_arch = "wasm32")]
fn selected_currency() -> Currency {
    match Params::current().currency {
        ParamCurrency::Usd => Currency::Usd,
        ParamCurrency::Eur => Currency::Eur,
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    ensure_login();
    kick_public(None);
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let changed = Params::previous().map_or_else(
        || vec!["miner_url", "miner_password", "currency"],
        |prev| Params::current().changed_keys(&prev),
    );
    let miner_changed = changed.contains(&"miner_url") || changed.contains(&"miner_password");
    let currency_changed = changed.contains(&"currency");

    if miner_changed {
        // The token (if any) was issued for the old URL/password, so discard it
        // and re-authenticate. Cancel the miner loops without reissuing here;
        // they restart once login succeeds, so reissuing now would only fetch
        // unauthenticated and bounce off a 401.
        let password_empty = Params::current().miner_password.is_empty();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.auth = if password_empty {
                AuthState::NoToken
            } else {
                AuthState::LoggingIn
            };
            for slot in &mut state.loops.miner {
                supersede(slot);
            }
        });
        if password_empty {
            STATE.with(|state| {
                supersede(&mut state.borrow_mut().loops.login);
            });
        } else {
            rekick_login(Some(DEBOUNCE_MS));
        }
    }
    if currency_changed {
        kick_public(Some(DEBOUNCE_MS));
    }
    request_frame();
}

// Numbers are formatted from raw state on every render against the live
// `number_format` setting, so a frame request is enough to reflect a changed
// system setting promptly instead of waiting for the next data refresh.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}

// Begin a login unless one is already under way; the `LoggingIn` state collapses
// a 401 storm from the parallel miner endpoints into a single login. An empty
// password leaves the miner loops dormant while public data keeps rendering.
#[cfg(target_arch = "wasm32")]
fn ensure_login() {
    if Params::current().miner_password.is_empty() {
        return;
    }
    let transition = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if matches!(state.auth, AuthState::LoggingIn) {
            false
        } else {
            state.auth = AuthState::LoggingIn;
            true
        }
    });
    if transition {
        rekick_login(None);
    }
}

// Force a fresh login with the current credentials, single-flight: a queued
// login is replaced immediately, while one already in flight is marked pending
// so its callback reissues rather than leaving a second login to race it.
#[cfg(target_arch = "wasm32")]
fn rekick_login(delay_ms: Option<u32>) {
    if STATE.with(|state| supersede(&mut state.borrow_mut().loops.login)) {
        schedule_login(delay_ms);
    }
}

#[cfg(target_arch = "wasm32")]
fn schedule_login(delay_ms: Option<u32>) {
    let params = Params::current();
    if params.miner_password.is_empty() {
        STATE.with(|state| state.borrow_mut().loops.login.live = None);
        return;
    }
    let url = endpoint(&params.miner_url, "/auth/login");
    let body = fmt!(
        r#"{{"username":"root","password":"{}"}}"#,
        JsonStr(&params.miner_password)
    );
    let req = FetchRequest::post(&url)
        .headers("Content-Type: application/json")
        .body(body.as_bytes());
    let queued = match delay_ms {
        Some(delay) => req.send_after(delay, on_login_response),
        None => req.send(on_login_response),
    };
    let Some(id) = queued else {
        log_warn!("mining-info: login not queued, retrying");
        if delay_ms != Some(RETRY_MS) {
            schedule_login(Some(RETRY_MS));
        }
        return;
    };
    STATE.with(|state| state.borrow_mut().loops.login.live = Some(id));
}

#[cfg(target_arch = "wasm32")]
fn on_login_response(response: &FetchResponse) {
    let routed = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let slot = &mut state.loops.login;
        (slot.live == Some(response.request_id)).then(|| take_pending(slot))
    });
    let Some(pending) = routed else {
        return;
    };
    if pending {
        // Credentials changed while this login was in flight; its result is for
        // the old credentials, so discard it and retry with the current ones.
        rekick_login(None);
        return;
    }

    if response.ok()
        && let Some(token) = response.json().str("/token")
    {
        STATE.with(|state| state.borrow_mut().auth = AuthState::Authenticated(token));
        kick_miner();
        request_frame();
        return;
    }
    log_warn!("mining-info: login failed with status {}", response.status);
    STATE.with(|state| state.borrow_mut().auth = AuthState::LoggingIn);
    schedule_login(Some(RETRY_MS));
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn kick_miner() {
    for idx in 0..MINER_ENDPOINTS.len() {
        rekick_miner(idx);
    }
}

#[cfg(target_arch = "wasm32")]
fn kick_public(delay_ms: Option<u32>) {
    for idx in 0..PUBLIC_ENDPOINTS.len() {
        rekick_public(idx, delay_ms);
    }
}

// Refresh one miner loop with current params, single-flight: a queued request is
// replaced now, one already in flight is left pending for its callback to reissue.
#[cfg(target_arch = "wasm32")]
fn rekick_miner(idx: usize) {
    if STATE.with(|state| supersede(&mut state.borrow_mut().loops.miner[idx])) {
        schedule_miner_endpoint(idx, None);
    }
}

#[cfg(target_arch = "wasm32")]
fn rekick_public(idx: usize, delay_ms: Option<u32>) {
    if STATE.with(|state| supersede(&mut state.borrow_mut().loops.public[idx])) {
        schedule_public_endpoint(idx, delay_ms);
    }
}

#[cfg(target_arch = "wasm32")]
fn schedule_miner_endpoint(idx: usize, delay_ms: Option<u32>) {
    let path = MINER_ENDPOINTS[idx].0;
    let params = Params::current();
    let url = endpoint(&params.miner_url, path);
    let header = STATE.with(|state| state.borrow().auth.auth_header());
    let queued = match delay_ms {
        Some(delay) => fetch_after(delay, &url, header.as_deref(), on_miner_response),
        None => fetch(&url, header.as_deref(), on_miner_response),
    };
    let Some(id) = queued else {
        log_warn!("mining-info: miner request not queued, retrying");
        STATE.with(|state| state.borrow_mut().loops.miner[idx].live = None);
        if delay_ms != Some(RETRY_MS) {
            schedule_miner_endpoint(idx, Some(RETRY_MS));
        }
        return;
    };
    STATE.with(|state| state.borrow_mut().loops.miner[idx].live = Some(id));
}

#[cfg(target_arch = "wasm32")]
fn schedule_public_endpoint(idx: usize, delay_ms: Option<u32>) {
    let url = (PUBLIC_ENDPOINTS[idx].0)(selected_currency());
    let queued = match delay_ms {
        Some(delay) => fetch_after(delay, &url, None, on_public_response),
        None => fetch(&url, None, on_public_response),
    };
    let Some(id) = queued else {
        log_warn!("mining-info: public request not queued, retrying");
        STATE.with(|state| state.borrow_mut().loops.public[idx].live = None);
        if delay_ms != Some(RETRY_MS) {
            schedule_public_endpoint(idx, Some(RETRY_MS));
        }
        return;
    };
    STATE.with(|state| state.borrow_mut().loops.public[idx].live = Some(id));
}

#[cfg(target_arch = "wasm32")]
fn on_miner_response(response: &FetchResponse) {
    let routed = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state
            .loops
            .miner
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.live == Some(response.request_id))
            .map(|(idx, slot)| (idx, take_pending(slot)))
    });
    let Some((idx, pending)) = routed else {
        return;
    };
    if pending {
        // A param change superseded this request, so its body is for stale
        // params. Refresh with current params, but only while authenticated —
        // otherwise the login flow re-kicks this loop once a token is available.
        if STATE.with(|state| state.borrow().auth.token().is_some()) {
            schedule_miner_endpoint(idx, None);
        }
        return;
    }

    if response.status == 401 {
        // A rejected token means re-auth; `ensure_login` collapses the parallel
        // 401s into a single login by acting only when not already logging in.
        ensure_login();
        return;
    }
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            (MINER_ENDPOINTS[idx].1)(&response.json(), &mut state.miner);
        });
        let authenticated = STATE.with(|state| state.borrow().auth.token().is_some());
        if authenticated {
            schedule_miner_endpoint(idx, Some(MINER_REFRESH_MS));
        }
    } else {
        log_warn!(
            "mining-info: miner endpoint {} failed with status {}",
            MINER_ENDPOINTS[idx].0,
            response.status
        );
        schedule_miner_endpoint(idx, Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_public_response(response: &FetchResponse) {
    let routed = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state
            .loops
            .public
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.live == Some(response.request_id))
            .map(|(idx, slot)| (idx, take_pending(slot)))
    });
    let Some((idx, pending)) = routed else {
        return;
    };
    if pending {
        // Currency changed while this request was in flight; its body is for the
        // old currency, so discard it and refresh with the current one.
        schedule_public_endpoint(idx, None);
        return;
    }

    if response.ok() {
        let currency = selected_currency();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            (PUBLIC_ENDPOINTS[idx].1)(&response.json(), currency, &mut state.public);
        });
        schedule_public_endpoint(idx, Some(PUBLIC_REFRESH_MS));
    } else {
        log_warn!(
            "mining-info: public endpoint {} failed with status {}",
            idx,
            response.status
        );
        schedule_public_endpoint(idx, Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let viewport = widget_viewport();
    let params = Params::current();
    let size = RenderSize {
        width: viewport.width,
        height: viewport.height,
    };
    let (miner, public) = STATE.with(|state| {
        let state = state.borrow();
        (state.miner.clone(), state.public.clone())
    });
    let root = match params.view {
        View::Mining => render::mining(size, &miner),
        View::Geek => render::geek(size, &miner, &public),
        View::Network => render::network(size, &public),
        View::InfoOverload => render::info_overload(size, &miner, &public),
    };
    let _ = render_ui(viewport.width, viewport.height, root);
}
