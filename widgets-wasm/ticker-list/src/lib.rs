// Copyright (C) 2026  Braiins Systems s.r.o.

//! Financial Ticker List widget. Up to 8 rows, each an independently-fetched
//! symbol: symbol + best-effort company name + sparkline + price + signed
//! change. One bad symbol degrades only its own row. Ported from deckfeeder's
//! `ticker-list`.

pub mod layout;
mod manifest_params;
pub mod model;
#[cfg(target_arch = "wasm32")]
pub mod render;
pub mod symbols;

#[cfg(any(target_arch = "wasm32", test))]
use prices::fetch::FetchClass;

/// What a price reply does to one row's state.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowTransition {
    /// Replace the row with a freshly parsed series.
    Store,
    /// Leave the row unchanged, keeping the last-good data on screen.
    Keep,
    /// 400/404 — disable this row's poll; render a "Not found" placeholder.
    InputError,
    /// Transient failure with nothing loaded for this row yet.
    Fail,
}

/// Fold an HTTP-status class + parse result into a per-row transition. Each row
/// is independent, so one symbol's outcome never touches another's slot.
#[cfg(any(target_arch = "wasm32", test))]
fn row_transition(class: FetchClass, parsed_ok: bool, has_data: bool) -> RowTransition {
    match class {
        FetchClass::Ok if parsed_ok => RowTransition::Store,
        FetchClass::InputError => RowTransition::InputError,
        // 503 also covers "no data for this symbol", so without held data it
        // degrades the row like any failure; the poll stays on, so a backend
        // that was merely warming up recovers on a later reply.
        FetchClass::Ok | FetchClass::Transient | FetchClass::Warming => {
            if has_data {
                RowTransition::Keep
            } else {
                RowTransition::Fail
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parsed_payload_stores_the_row() {
        assert_eq!(
            row_transition(FetchClass::Ok, true, false),
            RowTransition::Store
        );
    }

    #[test]
    fn input_error_is_isolated_to_the_row() {
        assert_eq!(
            row_transition(FetchClass::InputError, false, true),
            RowTransition::InputError
        );
        assert_eq!(
            row_transition(FetchClass::InputError, false, false),
            RowTransition::InputError
        );
    }

    #[test]
    fn any_failure_keeps_held_data_else_fails() {
        // A 503 row with nothing loaded must surface as unavailable rather
        // than sit invisibly in Loading — the server also answers 503 for
        // symbols it has no data for, not just while warming up.
        for class in [FetchClass::Ok, FetchClass::Transient, FetchClass::Warming] {
            assert_eq!(row_transition(class, false, true), RowTransition::Keep);
            assert_eq!(row_transition(class, false, false), RowTransition::Fail);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{RowTransition, layout, manifest_params, model, render, row_transition, symbols};
    use std::cell::{Cell, RefCell};

    #[expect(clippy::wildcard_imports, reason = "widget glue uses many SDK exports")]
    use bmc_wasm_sdk::*;
    use prices::fetch::{self, FetchClass};
    use prices::period::Period;
    use prices::{candle, reference};

    const PRICE_INTERVAL_MS: u32 = 300_000;
    const RETRY_MS: u32 = 10_000;
    const DEBOUNCE_MS: u32 = 300;
    const NEXUS_BASE: &str = "https://nexus.bit4u.cz/api/v1/data/";

    thread_local! {
        static SYMBOLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static STATES: RefCell<Vec<model::RowState>> = const { RefCell::new(Vec::new()) };
        static NAMES: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
        static PRICE_HANDLES: RefCell<Vec<PollHandle>> = const { RefCell::new(Vec::new()) };
        static REF_HANDLES: RefCell<Vec<PollHandle>> = const { RefCell::new(Vec::new()) };
        static PRICE_BASE: Cell<usize> = const { Cell::new(0) };
        static REF_BASE: Cell<usize> = const { Cell::new(0) };
        static FATAL: Cell<Option<symbols::SymbolsError>> = const { Cell::new(None) };
    }

    fn period_of() -> Period {
        Period::parse(
            manifest_params::Params::current()
                .period
                .as_manifest_value(),
        )
        .unwrap_or(Period::D7)
    }

    /// Decode the symbols param, resetting the row slots and the names cache.
    /// Stores a fatal error for the whole-widget message path.
    fn reload_symbols() {
        let raw = manifest_params::Params::current().symbols;
        let doc = JsonDoc::parse(raw.as_bytes());
        let decoded = if doc.is_valid() {
            symbols::decode_symbols(&doc)
        } else {
            Err(symbols::SymbolsError::Invalid)
        };
        match decoded {
            Ok(list) => {
                let n = list.len();
                SYMBOLS.with(|s| *s.borrow_mut() = list);
                STATES.with(|s| {
                    let mut st = s.borrow_mut();
                    st.clear();
                    st.resize_with(n, || model::RowState::Loading);
                });
                NAMES.with(|s| {
                    let mut nm = s.borrow_mut();
                    nm.clear();
                    nm.resize(n, None);
                });
                FATAL.with(|f| f.set(None));
            }
            Err(err) => {
                SYMBOLS.with(|s| s.borrow_mut().clear());
                STATES.with(|s| s.borrow_mut().clear());
                NAMES.with(|s| s.borrow_mut().clear());
                FATAL.with(|f| f.set(Some(err)));
            }
        }
    }

    /// Rows that are both configured and within the current size's capacity.
    fn enabled_rows() -> usize {
        let cap = layout::size_capacity(widget_size().variant);
        SYMBOLS.with(|s| s.borrow().len()).min(cap)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        reload_symbols();
        let mut price = Vec::with_capacity(symbols::MAX_SYMBOLS);
        for _ in 0..symbols::MAX_SYMBOLS {
            price.push(register_poll(
                build_price,
                on_price_reply,
                PollConfig {
                    interval_ms: Some(PRICE_INTERVAL_MS),
                    retry_ms: RETRY_MS,
                    debounce_ms: DEBOUNCE_MS,
                    enabled: true,
                },
            ));
        }
        let mut reference_handles = Vec::with_capacity(symbols::MAX_SYMBOLS);
        for _ in 0..symbols::MAX_SYMBOLS {
            reference_handles.push(register_poll(
                build_reference,
                on_reference_reply,
                PollConfig {
                    // One-shot, best-effort; enabled lazily once a row resolves.
                    interval_ms: None,
                    retry_ms: RETRY_MS,
                    debounce_ms: DEBOUNCE_MS,
                    enabled: false,
                },
            ));
        }
        PRICE_BASE.with(|b| b.set(price[0].index()));
        REF_BASE.with(|b| b.set(reference_handles[0].index()));
        PRICE_HANDLES.with(|h| *h.borrow_mut() = price);
        REF_HANDLES.with(|h| *h.borrow_mut() = reference_handles);
        request_frame();
    }

    fn price_row(handle: PollHandle) -> usize {
        handle.index() - PRICE_BASE.with(Cell::get)
    }

    fn reference_row(handle: PollHandle) -> usize {
        handle.index() - REF_BASE.with(Cell::get)
    }

    fn build_price(handle: PollHandle) -> Option<FetchSpec> {
        let row = price_row(handle);
        if row >= enabled_rows() {
            return None;
        }
        let symbol = SYMBOLS.with(|s| s.borrow().get(row).cloned())?;
        Some(FetchSpec::get(fetch::prices_url(
            NEXUS_BASE,
            &symbol,
            period_of(),
        )))
    }

    fn build_reference(handle: PollHandle) -> Option<FetchSpec> {
        let row = reference_row(handle);
        let symbol = SYMBOLS.with(|s| s.borrow().get(row).cloned())?;
        Some(FetchSpec::get(reference::reference_url(
            NEXUS_BASE, &symbol,
        )))
    }

    fn on_price_reply(handle: PollHandle, response: &FetchResponse) {
        let row = price_row(handle);
        let Some(symbol) = SYMBOLS.with(|s| s.borrow().get(row).cloned()) else {
            return;
        };
        let class = fetch::classify(response.status);
        let parsed = if class == FetchClass::Ok {
            let now = SystemTime::now().unix_secs;
            candle::parse_candles(&response.json()).and_then(|c| {
                model::TickerRow::from_candles(&symbol, &c, period_of().liveness(), now)
            })
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
        match row_transition(class, parsed.is_some(), has_data) {
            RowTransition::Store => {
                let row_data = parsed.expect("BUG: Store implies a parsed row");
                STATES.with(|s| {
                    s.borrow_mut()[row] = model::RowState::Resolved {
                        data: row_data,
                        stale: false,
                    };
                });
                // Lazily fetch the company name once the row first resolves and
                // we don't already have it — staggering names behind prices.
                let need_name = NAMES.with(|n| n.borrow().get(row).is_none_or(Option::is_none));
                if need_name {
                    REF_HANDLES.with(|h| {
                        if let Some(handle) = h.borrow().get(row) {
                            handle.set_enabled(true);
                        }
                    });
                }
            }
            RowTransition::InputError => {
                STATES.with(|s| s.borrow_mut()[row] = model::RowState::InputError);
                handle.set_enabled(false);
            }
            RowTransition::Fail => {
                STATES.with(|s| s.borrow_mut()[row] = model::RowState::Failed);
            }
            RowTransition::Keep => {
                // Keep is only reached on a failed refresh over held data —
                // mark the row stale so the render says so.
                STATES.with(|s| {
                    if let Some(model::RowState::Resolved { stale, .. }) =
                        s.borrow_mut().get_mut(row)
                    {
                        *stale = true;
                    }
                });
                if class == FetchClass::Ok {
                    handle.retry();
                }
            }
        }
        request_frame();
    }

    fn on_reference_reply(handle: PollHandle, response: &FetchResponse) {
        let row = reference_row(handle);
        if fetch::classify(response.status) == FetchClass::Ok
            && let Some(name) = reference::parse_name(&response.json())
        {
            NAMES.with(|n| {
                if let Some(slot) = n.borrow_mut().get_mut(row) {
                    *slot = Some(name);
                }
            });
            request_frame();
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        let symbols_changed = prev.as_ref().is_none_or(|p| p.symbols != cur.symbols);
        let period_changed = prev.as_ref().is_none_or(|p| p.period != cur.period);

        if symbols_changed {
            // New symbol set: rebuild rows + names, disable reference polls (they
            // re-enable lazily as rows resolve), and refetch prices.
            reload_symbols();
            REF_HANDLES.with(|h| {
                for handle in h.borrow().iter() {
                    handle.set_enabled(false);
                }
            });
            PRICE_HANDLES.with(|h| {
                for handle in h.borrow().iter() {
                    handle.set_enabled(true);
                    handle.invalidate();
                }
            });
        } else if period_changed {
            // Same symbols, different window: every row's series is stale, but
            // names are period-independent — keep the NAMES cache.
            STATES.with(|s| {
                for slot in s.borrow_mut().iter_mut() {
                    *slot = model::RowState::Loading;
                }
            });
            PRICE_HANDLES.with(|h| {
                for handle in h.borrow().iter() {
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
        let node = if let Some(err) = FATAL.with(Cell::get) {
            let msg = match err {
                symbols::SymbolsError::Invalid => "Invalid symbols list",
                symbols::SymbolsError::Empty => "No symbols provided",
            };
            render::message_view(msg, ws)
        } else {
            let visible = enabled_rows();
            let all_failed = visible > 0
                && STATES.with(|s| {
                    s.borrow().iter().take(visible).all(|st| {
                        matches!(st, model::RowState::InputError | model::RowState::Failed)
                    })
                });
            if all_failed {
                let any_transient = STATES.with(|s| {
                    s.borrow()
                        .iter()
                        .take(visible)
                        .any(|st| matches!(st, model::RowState::Failed))
                });
                let msg = if any_transient {
                    "Failed to fetch data"
                } else {
                    "No symbols returned data"
                };
                render::message_view(msg, ws)
            } else {
                SYMBOLS.with(|sym| {
                    STATES.with(|st| {
                        NAMES.with(|nm| render::view(&sym.borrow(), &st.borrow(), &nm.borrow(), ws))
                    })
                })
            }
        };
        let _ = render_ui(ws.width, ws.height, node);
    }
}
