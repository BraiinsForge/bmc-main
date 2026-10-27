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
//! Blank rows are skipped and the list compacts.

use crate::manifest_params::SymbolsItem;

/// Maximum symbols the list renders (and the most we ever fetch);
/// matches the manifest's `max_items`.
pub const MAX_SYMBOLS: usize = 8;

#[must_use]
pub fn collect_symbols(rows: &[SymbolsItem]) -> Vec<String> {
    rows.iter()
        .take(MAX_SYMBOLS)
        .filter_map(|row| {
            let base = row.base.trim();
            if base.is_empty() {
                return None;
            }
            let quote = row
                .quote
                .as_deref()
                .map(str::trim)
                .filter(|quote| !quote.is_empty());
            Some(match quote {
                Some(quote) => [base, "-", quote].concat(),
                None => base.to_owned(),
            })
        })
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
        }
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
