// Copyright (C) 2026  Braiins Systems s.r.o.

//! Symbol → Nexus instrument segment, and symbol → icon base. Mirrors
//! deckfeeder's `toInstrument` and `getCurrencyIcon` base extraction.

/// Map a configured symbol to a Nexus instrument segment: `BTC-USD` → `BTC/USD`;
/// a bare code (`AAPL`), a class-share code (`BRK-B`), or an index (`^GSPC`)
/// passes through unchanged. Only a hyphen followed by a 3-or-more-char tail is
/// a pair separator (so `BRK-B` keeps its hyphen, `BTC-USD` splits). Replaces
/// only the first hyphen, matching deckfeeder's `s.replace('-', '/')`.
#[must_use]
pub fn to_instrument(symbol: &str) -> String {
    let mut parts = symbol.split('-');
    let _head = parts.next();
    if let Some(tail) = parts.next()
        && tail.len() >= 3
    {
        return symbol.replacen('-', "/", 1);
    }
    symbol.to_owned()
}

/// Base symbol used to pick the display icon: `BTC-USD` → `BTC`,
/// `EURUSD=X` → `EURUSD`. Split on `-`/`=`, uppercased. Matches deckfeeder's
/// `symbol.split(/[-=]/)[0].toUpperCase()`.
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
    fn pair_with_currency_length_tail_splits_on_first_hyphen() {
        assert_eq!(to_instrument("BTC-USD"), "BTC/USD");
        assert_eq!(to_instrument("ETH-EUR"), "ETH/EUR");
    }

    #[test]
    fn class_share_keeps_its_short_tail_hyphen() {
        // `BRK-B` is a class share, not a pair: `B` is one char, below the
        // 3-char currency-tail threshold, so the hyphen stays.
        assert_eq!(to_instrument("BRK-B"), "BRK-B");
    }

    #[test]
    fn bare_code_and_index_pass_through() {
        assert_eq!(to_instrument("AAPL"), "AAPL");
        assert_eq!(to_instrument("^GSPC"), "^GSPC");
    }

    #[test]
    fn base_symbol_takes_the_leading_segment_uppercased() {
        assert_eq!(base_symbol("BTC-USD"), "BTC");
        assert_eq!(base_symbol("EURUSD=X"), "EURUSD");
        assert_eq!(base_symbol("goog"), "GOOG");
        assert_eq!(base_symbol("^GSPC"), "^GSPC");
    }
}
