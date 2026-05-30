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

// The live request id for each in-flight fetch. A response whose id no longer
// matches its slot was superseded by a re-kick (params change or re-login) and
// is ignored, which keeps every loop single-flight across re-kicks.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct LiveIds {
    login: Option<FetchRequestId>,
    miner: [Option<FetchRequestId>; 5],
    public: [Option<FetchRequestId>; 4],
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct State {
    miner: MinerData,
    public: PublicData,
    auth: AuthState,
    live: LiveIds,
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
    kick_public();
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
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.auth.clear();
            state.live.login = None;
            state.live.miner = Default::default();
        });
        ensure_login();
    }
    if currency_changed {
        kick_public();
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

// Authenticate only from `NoToken`; the `LoggingIn` state dedupes a 401 storm
// from the parallel miner endpoints into a single login. An empty password
// leaves the miner loops dormant while public data keeps rendering.
#[cfg(target_arch = "wasm32")]
fn ensure_login() {
    if Params::current().miner_password.is_empty() {
        return;
    }
    let transition = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if matches!(state.auth, AuthState::NoToken) {
            state.auth = AuthState::LoggingIn;
            true
        } else {
            false
        }
    });
    if transition {
        schedule_login(None);
    }
}

#[cfg(target_arch = "wasm32")]
fn schedule_login(delay_ms: Option<u32>) {
    let params = Params::current();
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
    STATE.with(|state| state.borrow_mut().live.login = Some(id));
}

#[cfg(target_arch = "wasm32")]
fn on_login_response(response: &FetchResponse) {
    let is_live = STATE.with(|state| state.borrow().live.login == Some(response.request_id));
    if !is_live {
        return;
    }
    STATE.with(|state| state.borrow_mut().live.login = None);

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
        schedule_miner_endpoint(idx, None);
    }
}

#[cfg(target_arch = "wasm32")]
fn kick_public() {
    for idx in 0..PUBLIC_ENDPOINTS.len() {
        schedule_public_endpoint(idx, None);
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
        STATE.with(|state| state.borrow_mut().live.miner[idx] = None);
        if delay_ms != Some(RETRY_MS) {
            schedule_miner_endpoint(idx, Some(RETRY_MS));
        }
        return;
    };
    STATE.with(|state| state.borrow_mut().live.miner[idx] = Some(id));
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
        STATE.with(|state| state.borrow_mut().live.public[idx] = None);
        if delay_ms != Some(RETRY_MS) {
            schedule_public_endpoint(idx, Some(RETRY_MS));
        }
        return;
    };
    STATE.with(|state| state.borrow_mut().live.public[idx] = Some(id));
}

#[cfg(target_arch = "wasm32")]
fn on_miner_response(response: &FetchResponse) {
    let idx = STATE.with(|state| {
        state
            .borrow()
            .live
            .miner
            .iter()
            .position(|id| *id == Some(response.request_id))
    });
    let Some(idx) = idx else {
        return;
    };
    STATE.with(|state| state.borrow_mut().live.miner[idx] = None);

    if response.status == 401 {
        STATE.with(|state| state.borrow_mut().auth.clear());
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
    let idx = STATE.with(|state| {
        state
            .borrow()
            .live
            .public
            .iter()
            .position(|id| *id == Some(response.request_id))
    });
    let Some(idx) = idx else {
        return;
    };
    STATE.with(|state| state.borrow_mut().live.public[idx] = None);

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
