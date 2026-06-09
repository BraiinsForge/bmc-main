// Copyright (C) 2026  Braiins Systems s.r.o.

//! Decode the `symbols` param (a JSON-array-of-strings held in a `string`
//! param) into 1..=8 trimmed symbols. Walks the parsed document via the shared
//! `JsonLookup` trait — the same indexed-pointer walk `parse_candles` uses — so
//! the logic is host-pure and unit-tested with a map fake.

use prices::candle::JsonLookup;

/// Maximum symbols the list renders (and the most we ever fetch).
pub const MAX_SYMBOLS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolsError {
    /// The string did not parse as JSON, or parsed to something with no usable
    /// string elements at `/0` (non-array root / non-string elements).
    Invalid,
    /// Parsed fine but yielded zero non-empty symbols.
    Empty,
}

/// Decode up to [`MAX_SYMBOLS`] trimmed, non-empty symbols from a parsed JSON
/// document (a root array of strings). Reads `/0`, `/1`, … until the array
/// ends; entries beyond the eighth are ignored (deckfeeder `slice(0, 8)`).
/// `Empty` when nothing usable remains. The caller maps a JSON parse failure to
/// [`SymbolsError::Invalid`] before calling this.
pub fn decode_symbols(json: &impl JsonLookup) -> Result<Vec<String>, SymbolsError> {
    let mut out = Vec::new();
    let mut path = String::new();
    for i in 0..MAX_SYMBOLS {
        index_path(&mut path, i);
        let Some(raw) = json.str(&path) else {
            break;
        };
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_owned());
        }
    }
    if out.is_empty() {
        Err(SymbolsError::Empty)
    } else {
        Ok(out)
    }
}

/// Build the JSON pointer `/<index>` into `buf` without an allocating format
/// macro (the `no-fmt-in-wasm` gate).
fn index_path(buf: &mut String, index: usize) {
    buf.clear();
    buf.push('/');
    if index == 0 {
        buf.push('0');
        return;
    }
    let mut n = index;
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + u8::try_from(n % 10).expect("BUG: n % 10 is one decimal digit");
        n /= 10;
    }
    for &d in &digits[i..] {
        buf.push(char::from(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prices::candle::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MapJson(BTreeMap<String, String>);
    impl MapJson {
        fn array(items: &[&str]) -> Self {
            let mut m = BTreeMap::new();
            for (i, s) in items.iter().enumerate() {
                let mut p = String::new();
                index_path(&mut p, i);
                m.insert(p, (*s).to_owned());
            }
            MapJson(m)
        }
    }
    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
        fn i64(&self, _: &str) -> Option<i64> {
            None
        }
        fn f64(&self, _: &str) -> Option<f64> {
            None
        }
    }

    #[test]
    fn valid_array_yields_trimmed_symbols() {
        let json = MapJson::array(&["NVDA", "  AAPL  ", "TSLA"]);
        assert_eq!(
            decode_symbols(&json),
            Ok(vec!["NVDA".into(), "AAPL".into(), "TSLA".into()])
        );
    }

    #[test]
    fn empty_entries_are_dropped() {
        let json = MapJson::array(&["NVDA", "  ", "TSLA"]);
        assert_eq!(
            decode_symbols(&json),
            Ok(vec!["NVDA".into(), "TSLA".into()])
        );
    }

    #[test]
    fn more_than_eight_truncates_to_first_eight() {
        let json = MapJson::array(&["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"]);
        let decoded = decode_symbols(&json).expect("eight");
        assert_eq!(decoded.len(), 8);
        assert_eq!(decoded.last().unwrap(), "H");
    }

    #[test]
    fn zero_usable_symbols_is_empty_error() {
        assert_eq!(
            decode_symbols(&MapJson::default()),
            Err(SymbolsError::Empty)
        );
        assert_eq!(
            decode_symbols(&MapJson::array(&["", "  "])),
            Err(SymbolsError::Empty)
        );
    }
}
