// Copyright (C) 2026  Braiins Systems s.r.o.

mod chart;
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
// Ceiling for the login backoff: a persistently-wrong login stops retrying more
// often than this rather than hammering `/auth/login` every `RETRY_MS` forever.
#[cfg(target_arch = "wasm32")]
const MAX_LOGIN_RETRY_MS: u32 = 300_000;
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
#[cfg(target_arch = "wasm32")]
type PublicReset = fn(&mut PublicData);

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
    reset: PublicReset,
    views: &'static [View],
    currency_dependent: bool,
}

#[cfg(target_arch = "wasm32")]
const PUBLIC_ENDPOINTS: [PublicEndpoint; 5] = [
    PublicEndpoint {
        url: public_api::price_stats_url,
        parse: public_price,
        reset: public_api::reset_price_stats,
        views: &[View::Geek, View::Network, View::InfoOverload],
        currency_dependent: true,
    },
    PublicEndpoint {
        url: public_api::block_url,
        parse: public_block,
        reset: public_api::reset_block,
        views: &[View::Network, View::InfoOverload],
        currency_dependent: false,
    },
    PublicEndpoint {
        url: public_api::difficulty_url,
        parse: public_difficulty,
        reset: public_api::reset_difficulty_stats,
        views: &[View::Network, View::InfoOverload],
        currency_dependent: false,
    },
    PublicEndpoint {
        url: public_api::hashrate_url,
        parse: public_hashrate,
        reset: public_api::reset_hashrate_stats,
        views: &[View::Network, View::InfoOverload],
        currency_dependent: true,
    },
    PublicEndpoint {
        url: public_api::price_history_url,
        parse: public_history,
        reset: public_api::reset_price_history,
        views: &[View::InfoOverload],
        currency_dependent: false,
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
#[cfg(target_arch = "wasm32")]
fn public_history(json: &JsonDoc, _currency: Currency, data: &mut PublicData) {
    public_api::parse_price_history(json, data);
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct State {
    miner: MinerData,
    public: PublicData,
    auth: AuthState,
    // Per-endpoint flags set when a poll's last reply was a failure. The failed
    // data is kept on screen (last-good) and the render path raises a "stale
    // data" banner while any endpoint feeding the current view stays in this
    // state.
    miner_stale: [bool; MINER_ENDPOINTS.len()],
    public_stale: [bool; PUBLIC_ENDPOINTS.len()],
    // Consecutive failed login replies, driving the retry backoff. Reset on a
    // successful login and on a credential change.
    login_failures: u32,
}

// Exponential backoff for repeated login failures: the delay doubles per
// consecutive failure off `base_ms`, capped at `cap_ms`, so a persistently
// wrong login (e.g. a 2xx with no token) stops hammering `/auth/login`.
fn login_retry_delay(failures: u32, base_ms: u32, cap_ms: u32) -> u32 {
    match 1_u32.checked_shl(failures) {
        Some(multiplier) => base_ms.saturating_mul(multiplier).min(cap_ms),
        None => cap_ms,
    }
}

// Handles for the registered polls. They are registered in the order login,
// the miner endpoints, then the public endpoints, so each group occupies a
// contiguous index range and a global handle index maps to a table index by
// subtracting the group's base.
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
    // Seed the gauge transition from empty: the first render draws a single lit
    // segment so the host always animates the fill in, even when miner data is
    // already available on the first frame.
    static FIRST_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
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

// Whether the current view renders any public Bitcoin field. Gates the
// "stale data" banner so it never shows on the miner-only Mining view.
#[cfg(target_arch = "wasm32")]
fn view_needs_public(view: View) -> bool {
    PUBLIC_ENDPOINTS
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
    let view = Params::current().view;
    let login = register_poll(
        build_login,
        on_login_reply,
        PollConfig {
            interval_ms: None,
            retry_ms: RETRY_MS,
            debounce_ms: DEBOUNCE_MS,
            enabled: view_needs_miner(view),
        },
    );
    let miner = std::array::from_fn(|idx| {
        register_poll(
            build_miner,
            on_miner_reply,
            PollConfig {
                interval_ms: Some(MINER_REFRESH_MS),
                retry_ms: RETRY_MS,
                debounce_ms: DEBOUNCE_MS,
                enabled: miner_endpoint_needed(idx, view),
            },
        )
    });
    let public = std::array::from_fn(|idx| {
        register_poll(
            build_public,
            on_public_reply,
            PollConfig {
                interval_ms: Some(PUBLIC_REFRESH_MS),
                retry_ms: RETRY_MS,
                debounce_ms: DEBOUNCE_MS,
                enabled: public_endpoint_needed(idx, view),
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

// Reconcile the registered polls with the current view and params: enable the
// endpoints the view reads and disable the rest, re-authenticate on a credential
// change, and on a currency change refresh only the currency-dependent public
// endpoints. The login serves every miner endpoint, so it is gated on whether any
// miner endpoint feeds the view rather than per-endpoint.
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

    HANDLES.with(|handles| {
        let handles = handles.borrow();
        let Some(handles) = handles.as_ref() else {
            return;
        };
        if view_needs_miner(view) {
            handles.login.set_enabled(true);
            if miner_creds_changed {
                let password_empty = Params::current().miner_password.is_empty();
                STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    miner_api::reset_all(&mut state.miner);
                    state.miner_stale = [false; MINER_ENDPOINTS.len()];
                    state.login_failures = 0;
                    state.auth = if password_empty {
                        AuthState::NoToken
                    } else {
                        AuthState::LoggingIn
                    };
                });
                handles.login.invalidate();
            }
            for (idx, miner) in handles.miner.iter().enumerate() {
                miner.set_enabled(miner_endpoint_needed(idx, view));
            }
        } else {
            STATE.with(|state| state.borrow_mut().auth = AuthState::NoToken);
            handles.login.set_enabled(false);
            for miner in &handles.miner {
                miner.set_enabled(false);
            }
        }
        for (idx, public) in handles.public.iter().enumerate() {
            let needed = public_endpoint_needed(idx, view);
            public.set_enabled(needed);
            if needed && currency_changed && PUBLIC_ENDPOINTS[idx].currency_dependent {
                STATE.with(|state| {
                    (PUBLIC_ENDPOINTS[idx].reset)(&mut state.borrow_mut().public);
                });
                public.invalidate();
            }
        }
    });
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

// Build the login request, or `None` to stay dormant when the miner source is
// hidden or no password is set. A successful reply yields a token the miner
// builders read; the poll is one-shot, so re-auth happens by invalidating the
// login handle rather than looping.
#[cfg(target_arch = "wasm32")]
fn build_login(_handle: PollHandle) -> Option<FetchSpec> {
    let params = Params::current();
    if !view_needs_miner(params.view) || params.miner_password.is_empty() {
        return None;
    }
    let url = endpoint(&params.miner_url, "/auth/login");
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

// Build an authenticated miner request, or `None` while no token is held so the
// poll stays dormant until login succeeds and invalidates it.
#[cfg(target_arch = "wasm32")]
fn build_miner(handle: PollHandle) -> Option<FetchSpec> {
    let header = STATE.with(|state| state.borrow().auth.auth_header())?;
    let url = endpoint(
        &Params::current().miner_url,
        MINER_ENDPOINTS[miner_index(handle)].path,
    );
    Some(FetchSpec::get(url).headers(header))
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

// Store the token and invalidate the miner endpoints so the ones the view needs
// refetch with it, and clear the failure backoff. Any other outcome surfaces the
// auth-error overlay and re-arms the one-shot login with `retry_after`, backing
// off exponentially per consecutive failure so a persistently-wrong login (e.g.
// a 2xx without a token) stops hammering the endpoint instead of wedging.
#[cfg(target_arch = "wasm32")]
fn on_login_reply(handle: PollHandle, response: &FetchResponse) {
    if response.ok()
        && let Some(token) = response.json().str("/token")
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
    } else {
        log_warn!("mining-info: login failed with status {}", response.status);
        let delay = STATE.with(|state| {
            let mut state = state.borrow_mut();
            miner_api::reset_all(&mut state.miner);
            state.miner_stale = [false; MINER_ENDPOINTS.len()];
            state.auth = AuthState::Failed;
            let delay = login_retry_delay(state.login_failures, RETRY_MS, MAX_LOGIN_RETRY_MS);
            state.login_failures = state.login_failures.saturating_add(1);
            delay
        });
        handle.retry_after(delay);
    }
    request_frame();
}

// Fold a miner response into state. A 401 means the token was rejected: drop it
// and invalidate the login handle so a single re-auth covers every endpoint that
// hit the same wall, then let this poll go dormant (its builder yields `None`
// without a token) until the fresh login reinvalidates it. Any other failure
// keeps the last good data and flags the endpoint stale so the render path can
// surface a "stale data" banner; the flag clears on the next success.
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
    let idx = miner_index(handle);
    if response.ok() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            (MINER_ENDPOINTS[idx].parse)(&response.json(), &mut state.miner);
            state.miner_stale[idx] = false;
        });
    } else {
        log_warn!(
            "mining-info: miner endpoint {} failed with status {}",
            MINER_ENDPOINTS[idx].path,
            response.status
        );
        STATE.with(|state| state.borrow_mut().miner_stale[idx] = true);
    }
    request_frame();
}

// Fold a public response into state. A failed refresh keeps the last good data
// rather than blanking the fields (matching the miner path); it only flags the
// endpoint stale so the render path can surface a "stale data" banner. The flag
// clears on the next success. The "this data is now wrong" case (a currency
// change) is handled by the deliberate `reset` in `on_params_update`.
#[cfg(target_arch = "wasm32")]
fn on_public_reply(handle: PollHandle, response: &FetchResponse) {
    let idx = public_index(handle);
    if response.ok() {
        let currency = selected_currency();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            (PUBLIC_ENDPOINTS[idx].parse)(&response.json(), currency, &mut state.public);
            state.public_stale[idx] = false;
        });
    } else {
        log_warn!(
            "mining-info: public endpoint {} failed with status {}",
            idx,
            response.status
        );
        STATE.with(|state| state.borrow_mut().public_stale[idx] = true);
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
    let (miner, public, auth_failed, miner_stale, public_stale) = STATE.with(|state| {
        let state = state.borrow();
        (
            state.miner.clone(),
            state.public.clone(),
            state.auth == AuthState::Failed,
            state.miner_stale.iter().any(|&stale| stale),
            state.public_stale.iter().any(|&stale| stale),
        )
    });
    let first_frame = FIRST_FRAME.replace(false);
    let mut root = match viewport.shape {
        ViewportShape::Round => match params.view {
            View::Mining => render::round::mining(size, &miner, first_frame),
            View::Geek => render::round::geek(size, &miner, &public, first_frame),
            View::InfoOverload => render::round::info_overload(&miner, &public),
            View::Network => render::round::network(&public),
        },
        ViewportShape::Rectangular => match params.view {
            View::Mining => render::mining(size, &miner),
            View::Geek => render::geek(size, &miner, &public),
            View::Network => render::network(size, &public),
            View::InfoOverload => render::info_overload(size, &miner, &public),
        },
    };
    // The auth-error overlay takes precedence: an unreachable miner is the larger
    // problem, and both banners share the same corner. Either source going stale
    // raises one shared "stale data" banner.
    let stale = (miner_stale && view_needs_miner(params.view))
        || (public_stale && view_needs_public(params.view));
    if auth_failed && view_needs_miner(params.view) {
        root = render::with_overlay(root, render::AUTH_ERROR_TEXT, viewport.shape);
    } else if stale {
        root = render::with_overlay(root, render::STALE_DATA_TEXT, viewport.shape);
    }
    let _ = render_ui(viewport.width, viewport.height, root);
    // The seeded first frame shows one segment; schedule the real value so the
    // transition animates from it on the next tick.
    if first_frame {
        request_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::login_retry_delay;

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
