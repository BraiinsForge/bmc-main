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

//! Turn the `symbols` rows into the instrument list the widget fetches:
//! `base-quote` for a pair, `base` alone otherwise.
//! Blank rows are skipped and the list compacts; each row's chosen price
//! decimals compact alongside, so both lists index the same rows.

use crate::manifest_params::SymbolsItem;

/// Maximum symbols the list renders (and the most we ever fetch);
/// matches the manifest's `max_items`.
pub const MAX_SYMBOLS: usize = 8;

/// The rows the list shows: within the cap and with a non-blank base.
fn shown(rows: &[SymbolsItem]) -> impl Iterator<Item = &SymbolsItem> {
    rows.iter()
        .take(MAX_SYMBOLS)
        .filter(|row| !row.base.trim().is_empty())
}

#[must_use]
pub fn collect_symbols(rows: &[SymbolsItem]) -> Vec<String> {
    shown(rows)
        .map(|row| {
            let base = row.base.trim();
            let quote = row
                .quote
                .as_deref()
                .map(str::trim)
                .filter(|quote| !quote.is_empty());
            match quote {
                Some(quote) => [base, "-", quote].concat(),
                None => base.to_owned(),
            }
        })
        .collect()
}

/// Each shown row's price decimals, in [`collect_symbols`] order;
/// `None` keeps the automatic rule.
#[must_use]
pub fn collect_decimals(rows: &[SymbolsItem]) -> Vec<Option<u32>> {
    shown(rows)
        .map(|row| prices::format::chosen_decimals(row.precision))
        .collect()
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::*;

    fn row(base: &str, quote: Option<&str>) -> SymbolsItem {
        SymbolsItem {
            base: base.to_owned(),
            quote: quote.map(str::to_owned),
            precision: None,
        }
    }

    #[wasm_test]
    fn decimals_follow_their_rows_through_the_compaction() {
        let rows = [
            SymbolsItem {
                precision: Some(2),
                ..row("EUR", Some("CZK"))
            },
            SymbolsItem {
                precision: Some(4),
                ..row("   ", None)
            },
            row("BTC", Some("USD")),
            SymbolsItem {
                precision: Some(0),
                ..row("USD", Some("CZK"))
            },
        ];
        assert_eq!(
            collect_symbols(&rows),
            vec!["EUR-CZK", "BTC-USD", "USD-CZK"]
        );
        assert_eq!(collect_decimals(&rows), vec![Some(2), None, Some(0)]);
    }

    #[wasm_test]
    fn rows_keep_their_order_and_pairs_join_with_a_dash() {
        let rows = [
            row("NVDA", None),
            row("EUR", Some("USD")),
            row("^GSPC", None),
        ];
        assert_eq!(collect_symbols(&rows), vec!["NVDA", "EUR-USD", "^GSPC"]);
    }

    #[wasm_test]
    fn collect_trims_and_compacts_blank_rows() {
        let rows = [
            row("  AAPL ", Some("  ")),
            row("   ", None),
            row("BTC", Some(" USD ")),
        ];
        assert_eq!(collect_symbols(&rows), vec!["AAPL", "BTC-USD"]);
    }

    #[wasm_test]
    fn all_blank_rows_collect_to_an_empty_list() {
        assert!(collect_symbols(&[row(" ", None)]).is_empty());
    }

    #[wasm_test]
    fn rows_past_the_cap_are_ignored() {
        let rows: Vec<_> = (0..10).map(|_| row("S", None)).collect();
        assert_eq!(collect_symbols(&rows).len(), MAX_SYMBOLS);
    }

    #[wasm_test]
    fn the_cap_is_the_manifest_max_items() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../manifest.json"))
            .expect("BUG: the manifest is JSON");
        assert_eq!(
            manifest["params"]["symbols"]["max_items"]
                .as_u64()
                .and_then(|max| usize::try_from(max).ok()),
            Some(MAX_SYMBOLS),
            "a larger manifest cap saves rows the widget never renders or fetches"
        );
    }
}
