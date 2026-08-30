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

//! Ticker — Single widget. Shows one instrument: a header (icon +
//! symbol + period + signed change badge), a tile-centered price, and a
//! bottom-anchored sparkline.

pub mod chart_layout;
pub mod display;
mod manifest_params;
pub mod model;
#[cfg(any(target_arch = "wasm32", test, feature = "scene"))]
pub mod render;

#[cfg(any(target_arch = "wasm32", test))]
use prices::fetch::PriceMiss;
#[cfg(any(target_arch = "wasm32", test))]
use prices::reference::ReferenceOutcome;
#[cfg(any(target_arch = "wasm32", test))]
use prices::transition::{Transition, placeholder};

/// Whether a system-snapshot change affects the current view's output. Number
/// format drives the price text in both views; timezone, time format, and
/// date format drive only the candlestick x-axis labels.
#[cfg(any(target_arch = "wasm32", test))]
fn system_affects_render(
    number_format_changed: bool,
    tz_or_datetime_changed: bool,
    view: manifest_params::View,
) -> bool {
    number_format_changed
        || (matches!(view, manifest_params::View::Candlestick) && tz_or_datetime_changed)
}

#[cfg(any(target_arch = "wasm32", test))]
fn effective_pair(params: &manifest_params::Params) -> &str {
    params.pair.as_deref().unwrap_or("").trim()
}

/// How much of the tile a parameter update invalidates.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParamChange {
    /// Nothing that reaches the request or the drawing.
    None,
    /// The same instrument drawn over a different window or in a different
    /// view: the series reloads, but everything the reference resource said
    /// about the instrument still holds.
    Presentation,
    /// A different instrument, so nothing learned about the old one carries.
    Instrument,
}

#[cfg(any(target_arch = "wasm32", test))]
fn param_change(
    prev: Option<&manifest_params::Params>,
    cur: &manifest_params::Params,
) -> ParamChange {
    let Some(prev) = prev else {
        return ParamChange::Instrument;
    };
    if effective_pair(prev) != effective_pair(cur) {
        ParamChange::Instrument
    } else if prev.period != cur.period || prev.view != cur.view {
        ParamChange::Presentation
    } else {
        ParamChange::None
    }
}

/// The tile's screen state.
#[cfg(any(target_arch = "wasm32", test))]
enum State {
    Loading,
    /// The held series stays drawn across failed refreshes; the SDK poll
    /// staleness (`stale_anchor`) drives the "last refresh" pill.
    Loaded {
        series: model::Series,
    },
    /// Nothing to draw, remembering why so that a later reference verdict
    /// may overturn a missing resource but never a refused request.
    InputError {
        miss: PriceMiss,
    },
    /// The instrument resolved but the window has no candles.
    NoData,
    Error,
}

/// Fold a settled reference verdict into the screen state. A held series only
/// refreshes its market flag; a placeholder born from a price miss takes
/// whichever message the verdict implies via [`placeholder`], the same mapping
/// the price path reads, so the two paths cannot contradict each other.
#[cfg(any(target_arch = "wasm32", test))]
fn apply_reference(state: &mut State, settled: ReferenceOutcome, is_market_open: Option<bool>) {
    match state {
        State::Loaded { series } => series.set_market_open(is_market_open),
        State::InputError { miss } => {
            if placeholder(*miss, settled) == Transition::NoData {
                *state = State::NoData;
            }
        }
        State::NoData => {
            if let Transition::InputError(miss) = placeholder(PriceMiss::NotFound, settled) {
                *state = State::InputError { miss };
            }
        }
        State::Loading | State::Error => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_pair_ignores_surrounding_whitespace_and_missing_values() {
        let mut params = manifest_params::Params {
            pair: Some("  BTC-USD  ".to_owned()),
            period: manifest_params::Period::_7d,
            view: manifest_params::View::Sparkline,
        };
        assert_eq!(effective_pair(&params), "BTC-USD");

        params.pair = None;
        assert_eq!(effective_pair(&params), "");
    }

    fn params(
        pair: &str,
        period: manifest_params::Period,
        view: manifest_params::View,
    ) -> manifest_params::Params {
        manifest_params::Params {
            pair: Some(pair.to_owned()),
            period,
            view,
        }
    }

    #[test]
    fn changing_the_window_or_the_view_keeps_the_instrument() {
        // The reference resource describes the instrument, not the window.
        // A period switch that dropped it would leave the next price 404
        // unexplained, and the tile would call a known symbol unknown.
        use manifest_params::{Period, View};
        let before = params("AAPL", Period::_7d, View::Sparkline);
        assert_eq!(
            param_change(Some(&before), &params("AAPL", Period::_1d, View::Sparkline)),
            ParamChange::Presentation
        );
        assert_eq!(
            param_change(
                Some(&before),
                &params("AAPL", Period::_7d, View::Candlestick)
            ),
            ParamChange::Presentation
        );
    }

    #[test]
    fn changing_the_symbol_voids_what_was_learned_about_the_old_one() {
        use manifest_params::{Period, View};
        let before = params("AAPL", Period::_7d, View::Sparkline);
        assert_eq!(
            param_change(Some(&before), &params("MSFT", Period::_7d, View::Sparkline)),
            ParamChange::Instrument
        );
        // Whitespace is not a new instrument.
        assert_eq!(
            param_change(
                Some(&before),
                &params("  AAPL ", Period::_7d, View::Sparkline)
            ),
            ParamChange::None
        );
    }

    #[test]
    fn the_first_update_has_nothing_to_carry_over() {
        use manifest_params::{Period, View};
        assert_eq!(
            param_change(None, &params("AAPL", Period::_7d, View::Sparkline)),
            ParamChange::Instrument
        );
    }

    #[test]
    fn an_unchanged_update_reloads_nothing() {
        use manifest_params::{Period, View};
        let before = params("AAPL", Period::_7d, View::Sparkline);
        assert_eq!(
            param_change(Some(&before), &params("AAPL", Period::_7d, View::Sparkline)),
            ParamChange::None
        );
    }

    #[test]
    fn a_reference_that_resolves_late_retracts_not_found() {
        let mut state = State::InputError {
            miss: PriceMiss::NotFound,
        };
        apply_reference(&mut state, ReferenceOutcome::Resolved, Some(false));
        assert!(matches!(state, State::NoData));
    }

    #[test]
    fn a_reference_that_stops_resolving_retracts_no_data() {
        let mut state = State::NoData;
        apply_reference(&mut state, ReferenceOutcome::NotFound, None);
        assert!(matches!(
            state,
            State::InputError {
                miss: PriceMiss::NotFound
            }
        ));
    }

    #[test]
    fn a_late_reference_never_reinterprets_a_refused_request() {
        // The reference explains an empty window; it cannot explain a request
        // Nexus refused, so resolving must not repaint "not found" as closed.
        let mut state = State::InputError {
            miss: PriceMiss::Rejected,
        };
        apply_reference(&mut state, ReferenceOutcome::Resolved, Some(false));
        assert!(matches!(
            state,
            State::InputError {
                miss: PriceMiss::Rejected
            }
        ));
    }

    #[test]
    fn a_tile_still_waiting_for_its_price_ignores_reference_verdicts() {
        // Before the first price reply there is nothing to reinterpret:
        // the price path picks the message once its own reply lands.
        let mut loading = State::Loading;
        apply_reference(&mut loading, ReferenceOutcome::NotFound, Some(false));
        assert!(matches!(loading, State::Loading));

        let mut error = State::Error;
        apply_reference(&mut error, ReferenceOutcome::Resolved, Some(false));
        assert!(matches!(error, State::Error));
    }

    #[test]
    fn a_reference_reply_refreshes_the_held_series_market_state() {
        use prices::candle::{CandleBar, Candles};
        let candles = Candles {
            bars: vec![CandleBar {
                t_secs: 0,
                open: 1.0,
                high: 2.0,
                low: 1.0,
                close: 2.0,
                volume: None,
            }],
            quote_currency: None,
        };
        let series =
            model::Series::from_candles(candles).expect("BUG: one candle is a valid series");
        let mut state = State::Loaded { series };
        apply_reference(&mut state, ReferenceOutcome::Resolved, Some(false));
        let State::Loaded { series } = &state else {
            panic!("a market update must not evict the held series");
        };
        assert!(!series.market_open);
    }

    #[test]
    fn number_format_change_renders_in_both_views() {
        for view in [
            manifest_params::View::Sparkline,
            manifest_params::View::Candlestick,
        ] {
            assert!(system_affects_render(true, false, view));
        }
    }

    #[test]
    fn every_manifest_period_token_is_a_nexus_window() {
        // `period_of` maps the generated manifest enum onto `prices::period`
        // with an expect; this locks the two hand-maintained token tables
        // together so a drift fails here instead of panicking on-device.
        for period in manifest_params::Period::ALL {
            assert!(
                prices::period::Period::parse(period.as_manifest_value()).is_some(),
                "manifest period token {:?} has no Nexus window",
                period.as_manifest_value()
            );
        }
        // Distinct tokens all parsing + equal counts makes the mapping a
        // bijection: every Nexus window stays reachable from the manifest.
        assert_eq!(
            manifest_params::Period::ALL.len(),
            prices::period::Period::ALL.len(),
            "the manifest ladder must expose every Nexus window"
        );
    }

    #[test]
    fn tz_or_datetime_change_renders_only_in_candlestick() {
        assert!(system_affects_render(
            false,
            true,
            manifest_params::View::Candlestick
        ));
        assert!(!system_affects_render(
            false,
            true,
            manifest_params::View::Sparkline
        ));
    }

    #[test]
    fn no_relevant_change_requests_no_frame() {
        for view in [
            manifest_params::View::Sparkline,
            manifest_params::View::Candlestick,
        ] {
            assert!(!system_affects_render(false, false, view));
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{
        ParamChange, State, Transition, apply_reference, effective_pair, manifest_params, model,
        param_change, render, system_affects_render,
    };
    use std::cell::{Cell, RefCell};

    #[expect(clippy::wildcard_imports, reason = "widget glue uses many SDK exports")]
    use bmc_wasm_sdk::*;
    use prices::candle;
    use prices::fetch::{self, FetchClass};
    use prices::period::Period;
    use prices::reference::{self, ReferenceOutcome};
    use prices::transition;

    const REFRESH_MS: u32 = 60_000;

    fn reschedule_failure(handle: PollHandle, class: FetchClass) {
        if transition::should_retry(class) {
            handle.retry();
        } else if class.uses_poll_interval() {
            handle.retry_after(REFRESH_MS);
        }
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
        static MARKET_OPEN: Cell<Option<bool>> = const { Cell::new(None) };
        static REFERENCE: Cell<ReferenceOutcome> =
            const { Cell::new(ReferenceOutcome::Unknown) };
        static PRICE_POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        static REFERENCE_POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
    }

    fn period_of(params: &manifest_params::Params) -> Period {
        Period::parse(params.period.as_manifest_value())
            .expect("BUG: every manifest period token is a Nexus window")
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let handle = register_poll(
            build_request,
            on_reply,
            PollConfig {
                interval_ms: Some(REFRESH_MS),
                ..PollConfig::default()
            },
        );
        PRICE_POLL.with(|p| p.set(Some(handle)));
        let reference_handle = register_poll(
            build_reference_request,
            on_reference_reply,
            PollConfig {
                interval_ms: Some(reference::REFERENCE_REFRESH_MS),
                ..PollConfig::default()
            },
        );
        REFERENCE_POLL.with(|poll| poll.set(Some(reference_handle)));
    }

    fn build_reference_request(_handle: PollHandle) -> Option<FetchSpec> {
        let params = manifest_params::Params::current();
        let pair = effective_pair(&params);
        if pair.is_empty() {
            return None;
        }
        Some(
            FetchSpec::get(reference::reference_url(prices::NEXUS_BASE, pair))
                .timeout(prices::FETCH_TIMEOUT),
        )
    }

    fn on_reference_reply(handle: PollHandle, response: &FetchResponse) {
        let class = fetch::classify(response.status);
        if let Some(delay_ms) = reference::reference_reschedule(class) {
            handle.retry_after(delay_ms);
        }
        let Some(settled) = reference::reference_outcome(class) else {
            return;
        };
        REFERENCE.with(|reference| reference.set(settled));
        let instrument_reference = if class == FetchClass::Ok {
            reference::parse_reference(&response.json())
        } else {
            reference::InstrumentReference::default()
        };
        MARKET_OPEN.with(|market_open| market_open.set(instrument_reference.is_market_open));
        STATE.with(|state| {
            apply_reference(
                &mut state.borrow_mut(),
                settled,
                instrument_reference.is_market_open,
            );
        });
        request_frame();
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        let params = manifest_params::Params::current();
        let pair = effective_pair(&params);
        if pair.is_empty() {
            return None;
        }
        let period = period_of(&params);
        Some(
            FetchSpec::get(fetch::prices_url(
                prices::NEXUS_BASE,
                pair,
                period,
                period.candle(),
            ))
            .timeout(prices::FETCH_TIMEOUT),
        )
    }

    fn on_reply(handle: PollHandle, response: &FetchResponse) {
        let class = fetch::classify(response.status);
        let parsed = if class == FetchClass::Ok {
            let series =
                candle::parse_candles(&response.json()).and_then(model::Series::from_candles);
            if series.is_none() {
                // The one failure that means a bug (Nexus schema drift), not
                // weather — it must not retry in silence.
                log_warn!("ticker-single: unusable 2xx payload");
            }
            series
        } else {
            if class == FetchClass::Transient {
                log_warn!("ticker-single: fetch failed (status {})", response.status);
            }
            None
        };
        let has_data = STATE.with(|s| matches!(&*s.borrow(), State::Loaded { .. }));
        match transition::from_reply(class, parsed.is_some(), has_data, REFERENCE.with(Cell::get)) {
            Transition::Store => {
                let mut series = parsed.expect("BUG: Store outcome implies a parsed series");
                series.set_market_open(MARKET_OPEN.with(Cell::get));
                STATE.with(|s| {
                    *s.borrow_mut() = State::Loaded { series };
                });
            }
            Transition::Keep => {
                // A failed refresh keeps the held series on screen; the poll
                // engine ages the stale pill from the last good load. `retry`
                // also rejects a 2xx-bad-parse reply as a refresh, so it
                // cannot bank a fresh staleness anchor.
                reschedule_failure(handle, class);
            }
            Transition::InputError(miss) => {
                STATE.with(|s| *s.borrow_mut() = State::InputError { miss });
                reschedule_failure(handle, class);
            }
            Transition::NoData => {
                STATE.with(|s| *s.borrow_mut() = State::NoData);
                reschedule_failure(handle, class);
            }
            Transition::Fail => {
                STATE.with(|s| *s.borrow_mut() = State::Error);
                reschedule_failure(handle, class);
            }
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        let change = param_change(prev.as_ref(), &cur);
        if change == ParamChange::None {
            return;
        }
        if change == ParamChange::Instrument {
            MARKET_OPEN.with(|market_open| market_open.set(None));
            REFERENCE.with(|reference| reference.set(ReferenceOutcome::Unknown));
            REFERENCE_POLL.with(|poll| {
                if let Some(handle) = poll.get() {
                    handle.invalidate();
                }
            });
        }
        STATE.with(|s| *s.borrow_mut() = State::Loading);
        PRICE_POLL.with(|p| {
            if let Some(handle) = p.get() {
                // Blanked data drops its staleness — the new instrument
                // must not inherit the old one's refresh anchor.
                handle.reset_staleness();
                handle.set_enabled(true);
                handle.invalidate();
            }
        });
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        let prev = system::previous();
        let cur = system::current();
        let number_format_changed = prev.number_format() != cur.number_format();
        let tz_or_datetime_changed = prev.timezone() != cur.timezone()
            || prev.time_format() != cur.time_format()
            || prev.date_format() != cur.date_format();
        let view = manifest_params::Params::current().view;
        if system_affects_render(number_format_changed, tz_or_datetime_changed, view) {
            request_frame();
        }
    }

    // The stale overlay's anchor: the last good load, but only while stale.
    fn stale_anchor() -> Option<SystemTime> {
        let handle = PRICE_POLL.with(Cell::get)?;
        if handle.is_stale() {
            handle.last_success_time()
        } else {
            None
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let params = manifest_params::Params::current();
        let symbol = effective_pair(&params);
        let node = if symbol.is_empty() {
            render::message_view("Enter symbol", ws)
        } else {
            STATE.with(|s| match &*s.borrow() {
                State::Loaded { series } => {
                    let view = match params.view {
                        manifest_params::View::Sparkline => render::sparkline::series_view(
                            series,
                            symbol,
                            params.period.as_manifest_value(),
                            ws,
                        ),
                        manifest_params::View::Candlestick => render::candlestick::series_view(
                            series,
                            symbol,
                            period_of(&params),
                            params.period.as_manifest_value(),
                            ws,
                        ),
                    };
                    match stale_anchor() {
                        Some(anchor) => with_stale_overlay(view, anchor, widget_viewport().shape),
                        None => view,
                    }
                }
                State::Loading => render::message_view("Loading\u{2026}", ws),
                State::InputError { .. } => {
                    render::message_view(&fmt!("Symbol {symbol} not found"), ws)
                }
                State::NoData => {
                    let message = if MARKET_OPEN.with(Cell::get) == Some(false) {
                        fmt!("{symbol} \u{2014} market closed")
                    } else {
                        fmt!("No data for this period")
                    };
                    render::message_view(&message, ws)
                }
                State::Error => render::message_view(&fmt!("{symbol} unavailable"), ws),
            })
        };
        let _ = render_ui(ws.width, ws.height, node);
    }
}
