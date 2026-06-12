// Copyright (C) 2026  Braiins Systems s.r.o.

//! Ticker — Single Sparkline widget. Shows one instrument: a header (icon +
//! symbol + period + signed change badge), a tile-centered price, and a
//! bottom-anchored sparkline. Ported from deckfeeder's `ticker-single-sparkline`.

pub mod display;
mod manifest_params;
pub mod model;
#[cfg(target_arch = "wasm32")]
pub mod render;

#[cfg(any(target_arch = "wasm32", test))]
use prices::fetch::FetchClass;

/// What a fetch reply does to the on-screen state.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transition {
    /// Replace the series with a freshly parsed one.
    Store,
    /// Keep the last-known-good series on screen (a refresh failed).
    KeepStale,
    /// The symbol/period is not resolvable; stop polling until params change.
    InputError,
    /// Failed reply with nothing loaded yet; the poll keeps running.
    Error,
}

/// Fold an HTTP-status class and the parse result into a state transition.
/// Held data survives any failed refresh; a 400/404 always disables the poll.
#[cfg(any(target_arch = "wasm32", test))]
fn outcome(class: FetchClass, parsed_ok: bool, has_data: bool) -> Transition {
    match class {
        FetchClass::Ok if parsed_ok => Transition::Store,
        FetchClass::InputError => Transition::InputError,
        // A 2xx whose body did not parse, a transient failure, or a 503
        // (which also covers "no data for this symbol"): keep the
        // last-known-good series if we have one, otherwise show unavailable.
        FetchClass::Ok | FetchClass::Transient | FetchClass::Warming => {
            if has_data {
                Transition::KeepStale
            } else {
                Transition::Error
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parsed_payload_replaces_the_series() {
        assert_eq!(outcome(FetchClass::Ok, true, false), Transition::Store);
        assert_eq!(outcome(FetchClass::Ok, true, true), Transition::Store);
    }

    #[test]
    fn input_error_disables_polling_regardless_of_held_data() {
        // 400/404 → InputError whether or not a series is already on screen,
        // so the poll loop stops hammering an unresolvable symbol.
        assert_eq!(
            outcome(FetchClass::InputError, false, false),
            Transition::InputError
        );
        assert_eq!(
            outcome(FetchClass::InputError, false, true),
            Transition::InputError
        );
    }

    #[test]
    fn any_failure_keeps_data_when_present_else_errors() {
        // A 503 with nothing loaded must surface as unavailable rather than
        // an indefinite warming/loading state — the server also answers 503
        // for symbols it has no data for, not just while warming up.
        for class in [FetchClass::Ok, FetchClass::Transient, FetchClass::Warming] {
            // Ok-but-unparsed and any failure keep held data...
            assert_eq!(outcome(class, false, true), Transition::KeepStale);
            // ...but show unavailable on first load with nothing yet.
            assert_eq!(outcome(class, false, false), Transition::Error);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{Transition, manifest_params, model, outcome, render};
    use std::cell::{Cell, RefCell};

    #[expect(clippy::wildcard_imports, reason = "widget glue uses many SDK exports")]
    use bmc_wasm_sdk::*;
    use prices::candle;
    use prices::fetch::{self, FetchClass};
    use prices::period::Period;

    const REFRESH_MS: u32 = 60_000;
    const RETRY_MS: u32 = 10_000;
    const DEBOUNCE_MS: u32 = 300;
    const NEXUS_BASE: &str = "https://nexus.bit4u.cz/api/v1/data/";

    enum State {
        Loading,
        /// `stale` is set when a later refresh fails, so the held series is
        /// still drawn but the tile says it is no longer current.
        Loaded {
            series: model::Series,
            stale: bool,
        },
        InputError,
        Error,
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
        static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
    }

    fn period_of(params: &manifest_params::Params) -> Period {
        Period::parse(params.period.as_manifest_value()).unwrap_or(Period::D7)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let handle = register_poll(
            build_request,
            on_reply,
            PollConfig {
                interval_ms: Some(REFRESH_MS),
                retry_ms: RETRY_MS,
                debounce_ms: DEBOUNCE_MS,
                enabled: true,
            },
        );
        POLL.with(|p| p.set(Some(handle)));
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        let params = manifest_params::Params::current();
        let pair = params.pair.trim();
        if pair.is_empty() {
            return None;
        }
        Some(FetchSpec::get(fetch::prices_url(
            NEXUS_BASE,
            pair,
            period_of(&params),
        )))
    }

    fn on_reply(handle: PollHandle, response: &FetchResponse) {
        let params = manifest_params::Params::current();
        let liveness = period_of(&params).liveness();
        let class = fetch::classify(response.status);
        let parsed = if class == FetchClass::Ok {
            let now = SystemTime::now().unix_secs;
            candle::parse_candles(&response.json())
                .and_then(|c| model::Series::from_candles(&c, liveness, now))
        } else {
            if class == FetchClass::Transient {
                log_warn!(
                    "ticker-single-sparkline: fetch failed (status {})",
                    response.status
                );
            }
            None
        };
        let has_data = STATE.with(|s| matches!(&*s.borrow(), State::Loaded { .. }));
        match outcome(class, parsed.is_some(), has_data) {
            Transition::Store => {
                let series = parsed.expect("BUG: Store outcome implies a parsed series");
                STATE.with(|s| {
                    *s.borrow_mut() = State::Loaded {
                        series,
                        stale: false,
                    };
                });
            }
            Transition::KeepStale => {
                // A failed refresh keeps the held series but marks it stale.
                STATE.with(|s| {
                    if let State::Loaded { stale, .. } = &mut *s.borrow_mut() {
                        *stale = true;
                    }
                });
                // A 2xx whose body failed to parse is worth retrying sooner
                // than the next poll; a transient failure waits for the engine.
                if class == FetchClass::Ok {
                    handle.retry();
                }
            }
            Transition::InputError => {
                STATE.with(|s| *s.borrow_mut() = State::InputError);
                handle.set_enabled(false);
            }
            Transition::Error => {
                STATE.with(|s| *s.borrow_mut() = State::Error);
            }
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        let changed = prev
            .as_ref()
            .is_none_or(|p| p.pair != cur.pair || p.period != cur.period);
        if changed {
            STATE.with(|s| *s.borrow_mut() = State::Loading);
            POLL.with(|p| {
                if let Some(handle) = p.get() {
                    handle.set_enabled(true);
                    handle.invalidate();
                }
            });
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let params = manifest_params::Params::current();
        let symbol = params.pair.trim();
        let node = if symbol.is_empty() {
            render::message_view("Enter symbol", ws)
        } else {
            STATE.with(|s| match &*s.borrow() {
                State::Loaded { series, stale } => render::series_view(
                    series,
                    symbol,
                    params.period.as_manifest_value(),
                    *stale,
                    ws,
                ),
                State::Loading => render::message_view("Loading\u{2026}", ws),
                State::InputError => render::message_view("Symbol not found", ws),
                State::Error => render::message_view(&fmt!("{symbol} unavailable"), ws),
            })
        };
        let _ = render_ui(ws.width, ws.height, node);
    }
}
