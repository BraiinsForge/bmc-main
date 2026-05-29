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
#[derive(Clone, Debug, Default)]
struct State {
    miner: MinerData,
    public: PublicData,
    auth: AuthState,
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
    schedule_miner_refresh(None);
    schedule_public_fetch(None);
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    STATE.with(|state| state.borrow_mut().auth.clear());
    schedule_miner_refresh(None);
    schedule_public_fetch(None);
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

// A dropped queue slot (`None`) is host backpressure, not a bug: the host
// declined to queue another in-flight request. Reschedule on the retry cadence
// rather than panicking the widget.
#[cfg(target_arch = "wasm32")]
fn requeue_miner_on_drop(queued: Option<FetchRequestId>) {
    if queued.is_none() {
        log_warn!("mining-info: miner request not queued, retrying");
        schedule_miner_refresh(Some(RETRY_MS));
    }
}

#[cfg(target_arch = "wasm32")]
fn requeue_public_on_drop(queued: Option<FetchRequestId>) {
    if queued.is_none() {
        log_warn!("mining-info: public request not queued, retrying");
        schedule_public_fetch(Some(RETRY_MS));
    }
}

// Reuse a cached token across refresh cycles; only authenticate when there is
// no token (first run, params change, or after a 401 clears it).
#[cfg(target_arch = "wasm32")]
fn schedule_miner_refresh(delay_ms: Option<u32>) {
    if Params::current().miner_password.is_empty() {
        return;
    }
    let has_token = STATE.with(|state| state.borrow().auth.token().is_some());
    if has_token {
        fetch_miner_endpoint("/miner/details", on_details_response, delay_ms);
    } else {
        schedule_login(delay_ms);
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
    requeue_miner_on_drop(queued);
}

#[cfg(target_arch = "wasm32")]
fn schedule_public_fetch(delay_ms: Option<u32>) {
    let url = public_api::price_stats_url(selected_currency());
    let queued = match delay_ms {
        Some(delay) => fetch_after(delay, &url, None, on_price_response),
        None => fetch(&url, None, on_price_response),
    };
    requeue_public_on_drop(queued);
}

#[cfg(target_arch = "wasm32")]
fn on_login_response(response: &FetchResponse) {
    if response.ok() {
        let json = response.json();
        if let Some(token) = json.str("/token") {
            STATE.with(|state| state.borrow_mut().auth.set_token(token));
            fetch_miner_endpoint("/miner/details", on_details_response, None);
            return;
        }
    }
    log_warn!("mining-info: login failed with status {}", response.status);
    schedule_miner_refresh(Some(RETRY_MS));
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn fetch_miner_endpoint(path: &'static str, callback: fn(&FetchResponse), delay_ms: Option<u32>) {
    let params = Params::current();
    let url = endpoint(&params.miner_url, path);
    let header = STATE.with(|state| state.borrow().auth.auth_header());
    let queued = match delay_ms {
        Some(delay) => fetch_after(delay, &url, header.as_deref(), callback),
        None => fetch(&url, header.as_deref(), callback),
    };
    requeue_miner_on_drop(queued);
}

#[cfg(target_arch = "wasm32")]
fn on_details_response(response: &FetchResponse) {
    if response.status == 401 {
        STATE.with(|state| state.borrow_mut().auth.clear());
        schedule_miner_refresh(Some(RETRY_MS));
        return;
    }
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            miner_api::parse_details(&response.json(), &mut state.miner);
        });
        fetch_miner_endpoint("/miner/stats", on_stats_response, None);
    } else {
        log_warn!(
            "mining-info: miner details failed with status {}",
            response.status
        );
        schedule_miner_refresh(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_stats_response(response: &FetchResponse) {
    if response.status == 401 {
        STATE.with(|state| state.borrow_mut().auth.clear());
        schedule_miner_refresh(Some(RETRY_MS));
        return;
    }
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            miner_api::parse_stats(&response.json(), &mut state.miner);
        });
        fetch_miner_endpoint("/miner/hw/hashboards", on_hashboards_response, None);
    } else {
        log_warn!(
            "mining-info: miner stats failed with status {}",
            response.status
        );
        schedule_miner_refresh(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_hashboards_response(response: &FetchResponse) {
    if response.status == 401 {
        STATE.with(|state| state.borrow_mut().auth.clear());
        schedule_miner_refresh(Some(RETRY_MS));
        return;
    }
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            miner_api::parse_hashboards(&response.json(), &mut state.miner);
        });
        fetch_miner_endpoint("/cooling/state", on_cooling_response, None);
    } else {
        log_warn!(
            "mining-info: hashboards failed with status {}",
            response.status
        );
        schedule_miner_refresh(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_cooling_response(response: &FetchResponse) {
    if response.status == 401 {
        STATE.with(|state| state.borrow_mut().auth.clear());
        schedule_miner_refresh(Some(RETRY_MS));
        return;
    }
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            miner_api::parse_cooling(&response.json(), &mut state.miner);
        });
        fetch_miner_endpoint("/network/", on_network_response, None);
    } else {
        log_warn!(
            "mining-info: cooling failed with status {}",
            response.status
        );
        schedule_miner_refresh(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_network_response(response: &FetchResponse) {
    if response.status == 401 {
        STATE.with(|state| state.borrow_mut().auth.clear());
        schedule_miner_refresh(Some(RETRY_MS));
        return;
    }
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            miner_api::parse_network(&response.json(), &mut state.miner);
        });
        schedule_miner_refresh(Some(MINER_REFRESH_MS));
    } else {
        log_warn!(
            "mining-info: network info failed with status {}",
            response.status
        );
        schedule_miner_refresh(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_price_response(response: &FetchResponse) {
    if response.ok() {
        let currency = selected_currency();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            public_api::parse_price_stats(&response.json(), currency, &mut state.public);
        });
        let url = public_api::block_url(currency);
        requeue_public_on_drop(fetch(&url, None, on_block_response));
    } else {
        log_warn!(
            "mining-info: price stats failed with status {}",
            response.status
        );
        schedule_public_fetch(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_block_response(response: &FetchResponse) {
    let currency = selected_currency();
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            public_api::parse_block(&response.json(), &mut state.public);
        });
        let url = public_api::difficulty_url(currency);
        requeue_public_on_drop(fetch(&url, None, on_difficulty_response));
    } else {
        log_warn!(
            "mining-info: block data failed with status {}",
            response.status
        );
        schedule_public_fetch(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_difficulty_response(response: &FetchResponse) {
    let currency = selected_currency();
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            public_api::parse_difficulty_stats(&response.json(), &mut state.public);
        });
        let url = public_api::hashrate_url(currency);
        requeue_public_on_drop(fetch(&url, None, on_hashrate_response));
    } else {
        log_warn!(
            "mining-info: difficulty stats failed with status {}",
            response.status
        );
        schedule_public_fetch(Some(RETRY_MS));
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn on_hashrate_response(response: &FetchResponse) {
    if response.ok() {
        let currency = selected_currency();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            public_api::parse_hashrate_stats(&response.json(), currency, &mut state.public);
        });
        schedule_public_fetch(Some(PUBLIC_REFRESH_MS));
    } else {
        log_warn!(
            "mining-info: hashrate stats failed with status {}",
            response.status
        );
        schedule_public_fetch(Some(RETRY_MS));
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
