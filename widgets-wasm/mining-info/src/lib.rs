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
// exists, so each runs its own refresh loop rather than chaining. `views` lists
// the views whose render path reads a field this endpoint produces; an endpoint
// is only fetched while one of those views is selected.
#[cfg(target_arch = "wasm32")]
struct MinerEndpoint {
    path: &'static str,
    parse: MinerParser,
    views: &'static [View],
}

#[cfg(target_arch = "wasm32")]
const MINER_ENDPOINTS: [MinerEndpoint; 5] = [
    MinerEndpoint {
        path: "/miner/details",
        parse: miner_details,
        views: &[View::Geek, View::InfoOverload],
    },
    MinerEndpoint {
        path: "/miner/stats",
        parse: miner_stats,
        views: &[View::Mining, View::Geek, View::InfoOverload],
    },
    MinerEndpoint {
        path: "/miner/hw/hashboards",
        parse: miner_hashboards,
        views: &[View::Mining, View::Geek],
    },
    MinerEndpoint {
        path: "/cooling/state",
        parse: miner_cooling,
        views: &[View::Mining],
    },
    MinerEndpoint {
        path: "/network/",
        parse: miner_network,
        views: &[View::Mining, View::Geek],
    },
];

// Public Bitcoin endpoints are unauthenticated and fully independent; each builds
// its URL from the selected currency and parses into `PublicData`. `views` gates
// fetching to the views that render the endpoint's fields. `currency_dependent`
// marks endpoints whose used fields change with the currency (the fiat price and
// hashprice); the others return currency-independent data, so a currency change
// leaves them untouched instead of refetching.
#[cfg(target_arch = "wasm32")]
struct PublicEndpoint {
    url: PublicUrl,
    parse: PublicParser,
    views: &'static [View],
    currency_dependent: bool,
}

#[cfg(target_arch = "wasm32")]
const PUBLIC_ENDPOINTS: [PublicEndpoint; 4] = [
    PublicEndpoint {
        url: public_api::price_stats_url,
        parse: public_price,
        views: &[View::Geek, View::Network, View::InfoOverload],
        currency_dependent: true,
    },
    PublicEndpoint {
        url: public_api::block_url,
        parse: public_block,
        views: &[View::Network, View::InfoOverload],
        currency_dependent: false,
    },
    PublicEndpoint {
        url: public_api::difficulty_url,
        parse: public_difficulty,
        views: &[View::Network, View::InfoOverload],
        currency_dependent: false,
    },
    PublicEndpoint {
        url: public_api::hashrate_url,
        parse: public_hashrate,
        views: &[View::Network, View::InfoOverload],
        currency_dependent: true,
    },
];

#[cfg(target_arch = "wasm32")]
fn miner_endpoint_needed(idx: usize, view: View) -> bool {
    MINER_ENDPOINTS[idx].views.contains(&view)
}

#[cfg(target_arch = "wasm32")]
fn public_endpoint_needed(idx: usize, view: View) -> bool {
    PUBLIC_ENDPOINTS[idx].views.contains(&view)
}

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

// Whether any miner endpoint feeds the current view. The login serves every miner
// endpoint, so it is driven at this source granularity, while the individual
// endpoints are gated per-view by `miner_endpoint_needed`.
#[cfg(target_arch = "wasm32")]
fn view_needs_miner(view: View) -> bool {
    MINER_ENDPOINTS
        .iter()
        .any(|endpoint| endpoint.views.contains(&view))
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
    let prev = Params::previous();
    let changed = prev.as_ref().map_or_else(
        || vec!["miner_url", "miner_password", "currency"],
        |prev| Params::current().changed_keys(prev),
    );
    let miner_creds_changed = changed.contains(&"miner_url") || changed.contains(&"miner_password");
    let currency_changed = changed.contains(&"currency");

    let view = Params::current().view;
    let prev_view = prev.as_ref().map(|prev| prev.view);

    reconcile_miner(view, prev_view, miner_creds_changed);
    reconcile_public(view, prev_view, currency_changed);
    request_frame();
}

// Drive the miner loops toward the current view: re-authenticate on a credential
// change, start the endpoints a newly-selected view reads, stop those it no
// longer reads, and tear everything down once no view needs the miner. Endpoints
// only fetch once authenticated, so while a login is still pending the loops are
// left for `on_login_response` to start with the right set.
#[cfg(target_arch = "wasm32")]
fn reconcile_miner(view: View, prev_view: Option<View>, creds_changed: bool) {
    if !view_needs_miner(view) {
        stop_miner();
        return;
    }
    if creds_changed {
        relogin(Some(DEBOUNCE_MS));
        return;
    }
    if STATE.with(|state| state.borrow().auth.token().is_none()) {
        ensure_login();
        return;
    }
    for (idx, endpoint) in MINER_ENDPOINTS.iter().enumerate() {
        let need_now = endpoint.views.contains(&view);
        let need_before = prev_view.is_some_and(|prev| endpoint.views.contains(&prev));
        if need_now && !need_before {
            rekick_miner(idx, Some(DEBOUNCE_MS));
        } else if !need_now && need_before {
            STATE.with(|state| {
                supersede(&mut state.borrow_mut().loops.miner[idx]);
            });
        }
    }
}

// Drive the public loops toward the current view: start the endpoints a
// newly-selected view reads, stop those it no longer reads, and on a currency
// change refresh only the currency-dependent endpoints — the rest return
// currency-independent data, so their existing responses stay valid.
#[cfg(target_arch = "wasm32")]
fn reconcile_public(view: View, prev_view: Option<View>, currency_changed: bool) {
    for (idx, endpoint) in PUBLIC_ENDPOINTS.iter().enumerate() {
        let need_now = endpoint.views.contains(&view);
        let need_before = prev_view.is_some_and(|prev| endpoint.views.contains(&prev));
        if need_now {
            if !need_before || (currency_changed && endpoint.currency_dependent) {
                rekick_public(idx, Some(DEBOUNCE_MS));
            }
        } else if need_before {
            STATE.with(|state| {
                supersede(&mut state.borrow_mut().loops.public[idx]);
            });
        }
    }
}

// Discard any token (it was issued for the old URL/password or a then-hidden
// miner view) and re-authenticate. Cancel the miner loops without reissuing —
// they restart once login succeeds, so reissuing now would only fetch
// unauthenticated and bounce off a 401.
#[cfg(target_arch = "wasm32")]
fn relogin(delay_ms: Option<u32>) {
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
        rekick_login(delay_ms);
    }
}

// Cancel the login and miner loops and drop the token; the miner source is
// hidden, so leaving them running would only burn fetch slots. In-flight
// requests cannot be cancelled, but their callbacks see the hidden view and stop
// rather than rescheduling.
#[cfg(target_arch = "wasm32")]
fn stop_miner() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.auth = AuthState::NoToken;
        supersede(&mut state.loops.login);
        for slot in &mut state.loops.miner {
            supersede(slot);
        }
    });
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
    let params = Params::current();
    if !view_needs_miner(params.view) || params.miner_password.is_empty() {
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
    let need_miner = view_needs_miner(Params::current().view);
    if pending {
        // Credentials changed (or the miner view was hidden) while this login was
        // in flight; its result is for the old state, so discard it and retry
        // with the current params only while the miner source is still shown.
        if need_miner {
            rekick_login(None);
        }
        return;
    }

    if response.ok()
        && let Some(token) = response.json().str("/token")
    {
        STATE.with(|state| state.borrow_mut().auth = AuthState::Authenticated(token));
        if need_miner {
            kick_miner();
        }
        request_frame();
        return;
    }
    log_warn!("mining-info: login failed with status {}", response.status);
    STATE.with(|state| state.borrow_mut().auth = AuthState::LoggingIn);
    if need_miner {
        schedule_login(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn kick_miner() {
    let view = Params::current().view;
    for idx in 0..MINER_ENDPOINTS.len() {
        if miner_endpoint_needed(idx, view) {
            rekick_miner(idx, None);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn kick_public(delay_ms: Option<u32>) {
    let view = Params::current().view;
    for idx in 0..PUBLIC_ENDPOINTS.len() {
        if public_endpoint_needed(idx, view) {
            rekick_public(idx, delay_ms);
        }
    }
}

// Refresh one miner loop with current params, single-flight: a queued request is
// replaced now, one already in flight is left pending for its callback to reissue.
#[cfg(target_arch = "wasm32")]
fn rekick_miner(idx: usize, delay_ms: Option<u32>) {
    if STATE.with(|state| supersede(&mut state.borrow_mut().loops.miner[idx])) {
        schedule_miner_endpoint(idx, delay_ms);
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
    let params = Params::current();
    let url = endpoint(&params.miner_url, MINER_ENDPOINTS[idx].path);
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
    let url = (PUBLIC_ENDPOINTS[idx].url)(selected_currency());
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
    let needed = miner_endpoint_needed(idx, Params::current().view);
    if pending {
        // A param change superseded this request, so its body is for stale
        // params. Refresh with current params, but only while this endpoint is
        // still shown and authenticated — otherwise the login flow re-kicks it
        // once a token is available, or the view no longer needs it at all.
        let authenticated = STATE.with(|state| state.borrow().auth.token().is_some());
        if needed && authenticated {
            schedule_miner_endpoint(idx, None);
        }
        return;
    }

    if response.status == 401 {
        // A rejected token means re-auth; `ensure_login` collapses the parallel
        // 401s into a single login and itself no-ops once the view hides the miner.
        ensure_login();
        return;
    }
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            (MINER_ENDPOINTS[idx].parse)(&response.json(), &mut state.miner);
        });
        let authenticated = STATE.with(|state| state.borrow().auth.token().is_some());
        if needed && authenticated {
            schedule_miner_endpoint(idx, Some(MINER_REFRESH_MS));
        }
    } else if needed {
        log_warn!(
            "mining-info: miner endpoint {} failed with status {}",
            MINER_ENDPOINTS[idx].path,
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
    let needed = public_endpoint_needed(idx, Params::current().view);
    if pending {
        // A currency or view change superseded this request, so its body is stale.
        // Refresh with current params only while this endpoint is still shown;
        // otherwise the view dropped it and the loop stops here.
        if needed {
            schedule_public_endpoint(idx, None);
        }
        return;
    }

    if response.ok() {
        let currency = selected_currency();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            (PUBLIC_ENDPOINTS[idx].parse)(&response.json(), currency, &mut state.public);
        });
        if needed {
            schedule_public_endpoint(idx, Some(PUBLIC_REFRESH_MS));
        }
    } else if needed {
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
