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

//! Financial Ticker List widget. Up to 8 rows, each an independently-fetched
//! symbol: symbol + best-effort company name + sparkline + price + signed
//! change. One bad symbol degrades only its own row.

pub mod layout;
mod manifest_params;
pub mod model;
#[cfg(any(target_arch = "wasm32", feature = "scene", test))]
pub mod render;
#[cfg(any(target_arch = "wasm32", test))]
mod symbols;

#[cfg(any(target_arch = "wasm32", test))]
use prices::fetch::PriceMiss;
#[cfg(any(target_arch = "wasm32", test))]
use prices::reference::ReferenceOutcome;
#[cfg(any(target_arch = "wasm32", test))]
use prices::transition::{Transition, placeholder};

/// Whether this price reply leaves the row needing its reference resource.
/// A stored row wants the company name it has not got yet; an unexplained 404
/// wants the instrument looked up, and keeps wanting that until it resolves.
#[cfg(any(target_arch = "wasm32", test))]
fn wants_reference(transition: Transition, has_name: bool) -> bool {
    match transition {
        Transition::Store => !has_name,
        Transition::InputError(PriceMiss::NotFound) => true,
        Transition::InputError(PriceMiss::Rejected)
        | Transition::Keep
        | Transition::NoData
        | Transition::Fail => false,
    }
}

/// Fold a settled reference verdict into one row's state. A held row only
/// refreshes its market flag; a placeholder born from a price miss takes
/// whichever message the verdict implies via [`placeholder`], the same
/// mapping the price path reads, so the two paths cannot contradict.
#[cfg(any(target_arch = "wasm32", test))]
fn apply_row_reference(
    slot: &mut model::RowState,
    settled: ReferenceOutcome,
    is_market_open: Option<bool>,
) {
    let market_closed = is_market_open == Some(false);
    match slot {
        model::RowState::Resolved { data } => data.set_market_open(is_market_open),
        model::RowState::InputError { miss } => {
            if placeholder(*miss, settled) == Transition::NoData {
                *slot = model::RowState::NoData { market_closed };
            }
        }
        model::RowState::NoData { .. } => {
            *slot = match placeholder(PriceMiss::NotFound, settled) {
                Transition::InputError(miss) => model::RowState::InputError { miss },
                Transition::Store | Transition::Keep | Transition::NoData | Transition::Fail => {
                    model::RowState::NoData { market_closed }
                }
            };
        }
        model::RowState::Loading | model::RowState::Failed => {}
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn changed_symbol_rows(previous: &[String], current: &[String]) -> [bool; symbols::MAX_SYMBOLS] {
    std::array::from_fn(|index| previous.get(index) != current.get(index))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn changing_one_symbol_preserves_every_other_row() {
        let previous = ["A", "B", "C"].map(str::to_owned);
        let current = ["A", "X", "C"].map(str::to_owned);
        assert_eq!(
            changed_symbol_rows(&previous, &current),
            [false, true, false, false, false, false, false, false]
        );
    }

    #[test]
    fn full_layout_capacity_matches_the_registered_row_polls() {
        assert_eq!(
            layout::size_capacity(bmc_wasm_sdk::SizeVariant::Full),
            symbols::MAX_SYMBOLS
        );
    }

    #[test]
    fn a_row_404_asks_for_the_reference_that_would_explain_it() {
        // The reference poll is off until something asks for it. If a 404
        // did not ask, a row that never once loaded could never learn
        // its market is merely closed, and would read "Not found" forever.
        assert!(wants_reference(
            Transition::InputError(PriceMiss::NotFound),
            true
        ));
    }

    #[test]
    fn a_row_404_keeps_asking_until_the_instrument_resolves() {
        // A reference 404 is not the last word — Nexus can start carrying
        // instruments it did not know before, and the row must notice.
        // A no-data row already resolved, which ends the asking;
        // the price cadence bounds how often the lookup repeats.
        assert!(wants_reference(
            Transition::InputError(PriceMiss::NotFound),
            true
        ));
        assert!(!wants_reference(Transition::NoData, true));
    }

    #[test]
    fn only_a_stored_row_still_missing_its_name_asks_on_success() {
        assert!(wants_reference(Transition::Store, false));
        assert!(!wants_reference(Transition::Store, true));
    }

    #[test]
    fn an_unusable_payload_does_not_chase_the_name_on_the_fast_retry() {
        // A 2xx that fails to parse retries far faster than the metadata
        // cadence. Asking on each of those would either repeat the lookup
        // or keep cancelling a reply still in flight.
        for transition in [Transition::Keep, Transition::Fail] {
            assert!(!wants_reference(transition, false));
        }
    }

    #[test]
    fn a_refused_request_never_asks_for_the_reference() {
        // No reference verdict may repaint a rejected request, so a lookup
        // on its behalf would waste the fetch.
        assert!(!wants_reference(
            Transition::InputError(PriceMiss::Rejected),
            false
        ));
    }

    #[test]
    fn a_reference_that_resolves_late_retracts_a_row_not_found() {
        let mut slot = model::RowState::InputError {
            miss: PriceMiss::NotFound,
        };
        apply_row_reference(&mut slot, ReferenceOutcome::Resolved, Some(false));
        assert!(matches!(
            slot,
            model::RowState::NoData {
                market_closed: true
            }
        ));
    }

    #[test]
    fn a_reference_that_stops_resolving_retracts_a_row_no_data() {
        let mut slot = model::RowState::NoData {
            market_closed: true,
        };
        apply_row_reference(&mut slot, ReferenceOutcome::NotFound, None);
        assert!(matches!(
            slot,
            model::RowState::InputError {
                miss: PriceMiss::NotFound
            }
        ));
    }

    #[test]
    fn a_late_reference_never_reinterprets_a_refused_row_request() {
        let mut slot = model::RowState::InputError {
            miss: PriceMiss::Rejected,
        };
        apply_row_reference(&mut slot, ReferenceOutcome::Resolved, Some(false));
        assert!(matches!(
            slot,
            model::RowState::InputError {
                miss: PriceMiss::Rejected
            }
        ));
    }

    #[test]
    fn a_reference_reply_refreshes_a_no_data_market_flag() {
        // The market can open while the window stays empty; the row
        // must not keep reading "Closed" off a stale flag.
        let mut slot = model::RowState::NoData {
            market_closed: true,
        };
        apply_row_reference(&mut slot, ReferenceOutcome::Resolved, Some(true));
        assert!(matches!(
            slot,
            model::RowState::NoData {
                market_closed: false
            }
        ));
    }

    #[test]
    fn a_reference_reply_refreshes_a_resolved_row_market_state() {
        let candles = candles_fixture();
        let data = model::TickerRow::from_candles("AAPL", &candles)
            .expect("BUG: one candle is a valid row");
        let mut slot = model::RowState::Resolved { data };
        apply_row_reference(&mut slot, ReferenceOutcome::Resolved, Some(false));
        let model::RowState::Resolved { data } = &slot else {
            panic!("a market update must not evict the held row");
        };
        assert!(!data.market_open);
    }

    fn candles_fixture() -> prices::candle::Candles {
        prices::candle::Candles {
            bars: vec![prices::candle::CandleBar {
                t_secs: 0,
                open: 1.0,
                high: 2.0,
                low: 1.0,
                close: 2.0,
                volume: None,
            }],
            quote_currency: None,
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{
        Transition, apply_row_reference, changed_symbol_rows, layout, manifest_params, model,
        render, symbols, wants_reference,
    };
    use std::cell::{Cell, RefCell};

    #[expect(clippy::wildcard_imports, reason = "widget glue uses many SDK exports")]
    use bmc_wasm_sdk::*;
    use prices::fetch::{self, FetchClass};
    use prices::period::Period;
    use prices::reference::ReferenceOutcome;
    use prices::transition;
    use prices::{candle, reference};

    const PRICE_INTERVAL_MS: u32 = 300_000;

    fn reschedule_failure(handle: PollHandle, class: FetchClass) {
        if transition::should_retry(class) {
            handle.retry();
        } else if class.uses_poll_interval() {
            handle.retry_after(PRICE_INTERVAL_MS);
        }
    }

    thread_local! {
        static SYMBOLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static STATES: RefCell<Vec<model::RowState>> = const { RefCell::new(Vec::new()) };
        static NAMES: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
        static MARKET_OPEN: RefCell<Vec<Option<bool>>> = const { RefCell::new(Vec::new()) };
        static REFERENCE: RefCell<Vec<ReferenceOutcome>> = const { RefCell::new(Vec::new()) };
        static PRICE_HANDLES: RefCell<Vec<PollHandle>> = const { RefCell::new(Vec::new()) };
        static REF_HANDLES: RefCell<Vec<PollHandle>> = const { RefCell::new(Vec::new()) };
        static PRICE_BASE: Cell<Option<usize>> = const { Cell::new(None) };
        static REF_BASE: Cell<Option<usize>> = const { Cell::new(None) };
    }

    fn period_of() -> Period {
        Period::parse(
            manifest_params::Params::current()
                .period
                .as_manifest_value(),
        )
        .expect("BUG: every manifest period token is a Nexus window")
    }

    /// Collect the `symbol_N` slot params and reset only rows whose effective
    /// symbol changed. An all-empty configuration leaves the list empty.
    fn reload_symbols() -> [bool; symbols::MAX_SYMBOLS] {
        let params = manifest_params::Params::current();
        let list = symbols::collect_symbols(&symbols::slots(&params));
        let n = list.len();
        let changed = SYMBOLS.with(|symbols| {
            let mut symbols = symbols.borrow_mut();
            let changed = changed_symbol_rows(&symbols, &list);
            *symbols = list;
            changed
        });
        STATES.with(|s| {
            let mut st = s.borrow_mut();
            st.resize_with(n, || model::RowState::Loading);
            for (index, slot) in st.iter_mut().enumerate() {
                if changed[index] {
                    *slot = model::RowState::Loading;
                }
            }
        });
        NAMES.with(|s| {
            let mut nm = s.borrow_mut();
            nm.resize(n, None);
            for (index, slot) in nm.iter_mut().enumerate() {
                if changed[index] {
                    *slot = None;
                }
            }
        });
        MARKET_OPEN.with(|states| {
            let mut states = states.borrow_mut();
            states.resize(n, None);
            for (index, state) in states.iter_mut().enumerate() {
                if changed[index] {
                    *state = None;
                }
            }
        });
        REFERENCE.with(|states| {
            let mut states = states.borrow_mut();
            states.resize(n, ReferenceOutcome::Unknown);
            for (index, state) in states.iter_mut().enumerate() {
                if changed[index] {
                    *state = ReferenceOutcome::Unknown;
                }
            }
        });
        changed
    }

    /// Rows that are both configured and within the current size's capacity.
    fn enabled_rows() -> usize {
        let cap = layout::size_capacity(widget_size().variant);
        SYMBOLS.with(|s| s.borrow().len()).min(cap)
    }

    // A poll waiting out its interval still holds its fetch slot, so eight symbols
    // at two polls each sit exactly on `RuntimeResourceLimits::max_fetches`.
    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let _ = reload_symbols();
        let mut price = Vec::with_capacity(symbols::MAX_SYMBOLS);
        price.push(register_poll(
            build_price,
            on_price_reply,
            PollConfig {
                interval_ms: Some(PRICE_INTERVAL_MS),
                ..PollConfig::default()
            },
        ));
        PRICE_BASE.with(|b| b.set(Some(price[0].index())));
        // Registration builds synchronously; retry row 0 after its base exists.
        price[0].invalidate();
        for _ in 1..symbols::MAX_SYMBOLS {
            price.push(register_poll(
                build_price,
                on_price_reply,
                PollConfig {
                    interval_ms: Some(PRICE_INTERVAL_MS),
                    ..PollConfig::default()
                },
            ));
        }
        let mut reference_handles = Vec::with_capacity(symbols::MAX_SYMBOLS);
        reference_handles.push(register_poll(
            build_reference,
            on_reference_reply,
            PollConfig {
                interval_ms: Some(reference::REFERENCE_REFRESH_MS),
                enabled: false,
                ..PollConfig::default()
            },
        ));
        REF_BASE.with(|b| b.set(Some(reference_handles[0].index())));
        for _ in 1..symbols::MAX_SYMBOLS {
            reference_handles.push(register_poll(
                build_reference,
                on_reference_reply,
                PollConfig {
                    // Armed lazily by the first price reply that wants it;
                    // from then on the interval keeps the row's name
                    // and market state fresh.
                    interval_ms: Some(reference::REFERENCE_REFRESH_MS),
                    enabled: false,
                    ..PollConfig::default()
                },
            ));
        }
        PRICE_HANDLES.with(|h| *h.borrow_mut() = price);
        REF_HANDLES.with(|h| *h.borrow_mut() = reference_handles);
        request_frame();
    }

    fn price_row(handle: PollHandle) -> Option<usize> {
        handle.index().checked_sub(PRICE_BASE.with(Cell::get)?)
    }

    fn reference_row(handle: PollHandle) -> Option<usize> {
        handle.index().checked_sub(REF_BASE.with(Cell::get)?)
    }

    fn build_price(handle: PollHandle) -> Option<FetchSpec> {
        let row = price_row(handle)?;
        if row >= enabled_rows() {
            return None;
        }
        let symbol = SYMBOLS.with(|s| s.borrow().get(row).cloned())?;
        let period = period_of();
        Some(
            FetchSpec::get(fetch::prices_url(
                prices::NEXUS_BASE,
                &symbol,
                period,
                period.candle(),
            ))
            .timeout(prices::FETCH_TIMEOUT),
        )
    }

    fn build_reference(handle: PollHandle) -> Option<FetchSpec> {
        let row = reference_row(handle)?;
        let symbol = SYMBOLS.with(|s| s.borrow().get(row).cloned())?;
        Some(
            FetchSpec::get(reference::reference_url(prices::NEXUS_BASE, &symbol))
                .timeout(prices::FETCH_TIMEOUT),
        )
    }

    fn on_price_reply(handle: PollHandle, response: &FetchResponse) {
        let Some(row) = price_row(handle) else {
            return;
        };
        let Some(symbol) = SYMBOLS.with(|s| s.borrow().get(row).cloned()) else {
            return;
        };
        let class = fetch::classify(response.status);
        let parsed = if class == FetchClass::Ok {
            let row_data = candle::parse_candles(&response.json())
                .and_then(|c| model::TickerRow::from_candles(&symbol, &c));
            if row_data.is_none() {
                // The one failure that means a bug (Nexus schema drift), not
                // weather — it must not retry in silence.
                log_warn!("ticker-list: '{}' returned an unusable 2xx payload", symbol);
            }
            row_data
        } else {
            if class == FetchClass::Transient {
                log_warn!(
                    "ticker-list: '{}' fetch failed (status {})",
                    symbol,
                    response.status
                );
            }
            None
        };
        let has_data =
            STATES.with(|s| matches!(s.borrow().get(row), Some(model::RowState::Resolved { .. })));
        let reference = REFERENCE.with(|r| {
            r.borrow()
                .get(row)
                .copied()
                .unwrap_or(ReferenceOutcome::Unknown)
        });
        let transition = transition::from_reply(class, parsed.is_some(), has_data, reference);
        match transition {
            Transition::Store => {
                let mut row_data = parsed.expect("BUG: Store implies a parsed row");
                row_data.set_market_open(
                    MARKET_OPEN.with(|states| states.borrow().get(row).copied().flatten()),
                );
                STATES.with(|s| {
                    s.borrow_mut()[row] = model::RowState::Resolved { data: row_data };
                });
            }
            Transition::InputError(miss) => {
                STATES.with(|s| s.borrow_mut()[row] = model::RowState::InputError { miss });
                reschedule_failure(handle, class);
            }
            Transition::NoData => {
                let market_closed = MARKET_OPEN
                    .with(|states| states.borrow().get(row).copied().flatten())
                    == Some(false);
                STATES.with(|s| s.borrow_mut()[row] = model::RowState::NoData { market_closed });
                reschedule_failure(handle, class);
            }
            Transition::Fail => {
                STATES.with(|s| s.borrow_mut()[row] = model::RowState::Failed);
                reschedule_failure(handle, class);
            }
            Transition::Keep => {
                // A failed refresh keeps the held series on screen; the stale
                // badge rides the poll engine's `is_stale` grace, and `retry`
                // rejects a 2xx-bad-parse reply as a staleness anchor.
                reschedule_failure(handle, class);
            }
        }
        let has_name = NAMES.with(|names| names.borrow().get(row).is_some_and(Option::is_some));
        if wants_reference(transition, has_name) {
            REF_HANDLES.with(|handles| {
                if let Some(handle) = handles.borrow().get(row) {
                    handle.set_enabled(true);
                    // A disable can fail to cancel a reply already in flight
                    // from the previous symbol; invalidate so it is reissued
                    // instead of delivered as this row's.
                    handle.invalidate();
                }
            });
        }
        request_frame();
    }

    fn on_reference_reply(handle: PollHandle, response: &FetchResponse) {
        let Some(row) = reference_row(handle) else {
            return;
        };
        let class = fetch::classify(response.status);
        if let Some(delay_ms) = reference::reference_reschedule(class) {
            handle.retry_after(delay_ms);
        }
        let Some(settled) = reference::reference_outcome(class) else {
            return;
        };
        let instrument_reference = if class == FetchClass::Ok {
            reference::parse_reference(&response.json())
        } else {
            reference::InstrumentReference::default()
        };
        REFERENCE.with(|states| {
            if let Some(state) = states.borrow_mut().get_mut(row) {
                *state = settled;
            }
        });
        MARKET_OPEN.with(|states| {
            if let Some(state) = states.borrow_mut().get_mut(row) {
                *state = instrument_reference.is_market_open;
            }
        });
        NAMES.with(|names| {
            if let Some(slot) = names.borrow_mut().get_mut(row) {
                *slot = instrument_reference.name;
            }
        });
        STATES.with(|states| {
            if let Some(slot) = states.borrow_mut().get_mut(row) {
                apply_row_reference(slot, settled, instrument_reference.is_market_open);
            }
        });
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        // Compare the effective lists, not the raw slots: a cosmetic edit
        // (added whitespace, a blank slot moved) must not blank every row
        // into Loading and refetch.
        let symbols_changed = prev.as_ref().is_none_or(|p| {
            symbols::collect_symbols(&symbols::slots(p))
                != symbols::collect_symbols(&symbols::slots(&cur))
        });
        let period_changed = prev.as_ref().is_none_or(|p| p.period != cur.period);
        let changed = symbols_changed || period_changed;

        if changed {
            let changed_rows = if symbols_changed {
                reload_symbols()
            } else {
                [false; symbols::MAX_SYMBOLS]
            };
            REF_HANDLES.with(|h| {
                for (index, handle) in h.borrow().iter().enumerate() {
                    if changed_rows[index] {
                        handle.set_enabled(false);
                    }
                }
            });
            if period_changed {
                STATES.with(|s| {
                    for slot in s.borrow_mut().iter_mut() {
                        *slot = model::RowState::Loading;
                    }
                });
            }
            PRICE_HANDLES.with(|h| {
                for (index, handle) in h.borrow().iter().enumerate() {
                    if changed_rows[index] || period_changed {
                        handle.reset_staleness();
                        handle.set_enabled(true);
                        handle.invalidate();
                    }
                }
            });
        }
        if changed {
            request_frame();
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        if system::previous().number_format() != system::current().number_format() {
            request_frame();
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let node = if SYMBOLS.with(|s| s.borrow().is_empty()) {
            render::message_view("No symbols provided", ws)
        } else {
            // Even when every row failed, keep the per-row placeholders: each
            // one names its symbol and why it is missing, which a collapsed
            // whole-widget message would hide.

            // Each badge ages from its row's last good load, but only while
            // that row is stale — the same anchor rule ticker-single applies.
            let stale: Vec<Option<SystemTime>> = PRICE_HANDLES.with(|h| {
                h.borrow()
                    .iter()
                    .map(|h| {
                        if h.is_stale() {
                            h.last_success_time()
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            SYMBOLS.with(|sym| {
                STATES.with(|st| {
                    NAMES.with(|nm| {
                        render::view(&sym.borrow(), &st.borrow(), &nm.borrow(), &stale, ws)
                    })
                })
            })
        };
        let _ = render_ui(ws.width, ws.height, node);
    }
}
