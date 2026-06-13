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

//! Collect the per-slot `symbol_N` params into the row list. The manifest has
//! no array type, so each row is its own string param; empty slots are skipped
//! and the list compacts.

use crate::manifest_params::Params;

/// Maximum symbols the list renders (and the most we ever fetch).
pub const MAX_SYMBOLS: usize = 8;

/// The slot params in row order. The array length ties the manifest's
/// `symbol_N` param count to [`MAX_SYMBOLS`] at compile time.
#[must_use]
pub fn slots(params: &Params) -> [&str; MAX_SYMBOLS] {
    [
        params.symbol_1.as_deref().unwrap_or(""),
        params.symbol_2.as_deref().unwrap_or(""),
        params.symbol_3.as_deref().unwrap_or(""),
        params.symbol_4.as_deref().unwrap_or(""),
        params.symbol_5.as_deref().unwrap_or(""),
        params.symbol_6.as_deref().unwrap_or(""),
        params.symbol_7.as_deref().unwrap_or(""),
        params.symbol_8.as_deref().unwrap_or(""),
    ]
}

/// Collect the configured slots in order: trimmed, empty slots skipped. An
/// all-empty configuration yields an empty list; the widget renders its
/// "No symbols provided" message off that.
#[must_use]
pub fn collect_symbols(slots: &[&str; MAX_SYMBOLS]) -> Vec<String> {
    slots
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest_params::Period;

    fn params(slot_values: [&str; MAX_SYMBOLS]) -> Params {
        let [s1, s2, s3, s4, s5, s6, s7, s8] = slot_values.map(|s| Some(s.to_owned()));
        Params {
            period: Period::_7d,
            symbol_1: s1,
            symbol_2: s2,
            symbol_3: s3,
            symbol_4: s4,
            symbol_5: s5,
            symbol_6: s6,
            symbol_7: s7,
            symbol_8: s8,
        }
    }

    #[test]
    fn slots_preserve_row_order() {
        let p = params(["A", "B", "C", "D", "E", "F", "G", "H"]);
        assert_eq!(slots(&p), ["A", "B", "C", "D", "E", "F", "G", "H"]);
    }

    #[test]
    fn collect_trims_and_compacts_empty_slots() {
        let p = params(["NVDA", "  AAPL  ", "", "  ", "TSLA", "", "", ""]);
        assert_eq!(
            collect_symbols(&slots(&p)),
            vec!["NVDA".to_owned(), "AAPL".to_owned(), "TSLA".to_owned()]
        );
    }

    #[test]
    fn all_empty_slots_collect_to_an_empty_list() {
        let p = params(["", " ", "", "", "", "", "", ""]);
        assert!(collect_symbols(&slots(&p)).is_empty());
    }
}
