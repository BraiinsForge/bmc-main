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

//! Symbol pair detection and icon-base extraction.

/// Split a configured symbol into base and quote when it names a pair:
/// `BTC-USD` → `(BTC, USD)`. Only the first hyphen followed by a 3-or-more-char
/// tail is a pair separator, so a class share (`BRK-B`) does not split.
#[must_use]
pub fn split_pair(symbol: &str) -> Option<(&str, &str)> {
    let (base, quote) = symbol.split_once('-')?;
    (quote.len() >= 3).then_some((base, quote))
}

/// Base symbol used to pick the display icon: `BTC-USD` → `BTC`,
/// `EURUSD=X` → `EURUSD`. Split on `-`/`=`, uppercased.
#[must_use]
pub fn base_symbol(symbol: &str) -> String {
    symbol
        .split(['-', '='])
        .next()
        .unwrap_or(symbol)
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_pair_follows_the_currency_tail_rule() {
        assert_eq!(split_pair("BTC-USD"), Some(("BTC", "USD")));
        assert_eq!(split_pair("BTC"), None);
        assert_eq!(split_pair("BRK-B"), None);
        assert_eq!(split_pair("^GSPC"), None);
    }

    #[test]
    fn base_symbol_takes_the_leading_segment_uppercased() {
        assert_eq!(base_symbol("BTC-USD"), "BTC");
        assert_eq!(base_symbol("EURUSD=X"), "EURUSD");
        assert_eq!(base_symbol("goog"), "GOOG");
        assert_eq!(base_symbol("^GSPC"), "^GSPC");
    }
}
