// Copyright (C) 2026  Braiins Systems s.r.o.

//! Best-effort instrument display-name resource (`reference/<instrument>`).
//! Used by ticker-list; the single-sparkline widget does not show a name.

use crate::candle::JsonLookup;
use crate::fetch::encode_resource_url;
use crate::instrument::to_instrument;

/// Build the resource path `reference/<instrument>`.
#[must_use]
pub fn reference_path(symbol: &str) -> String {
    let mut out = String::from("reference/");
    out.push_str(&to_instrument(symbol));
    out
}

/// Build the full Nexus URL for an instrument's reference resource.
#[must_use]
pub fn reference_url(base: &str, symbol: &str) -> String {
    encode_resource_url(base, &reference_path(symbol))
}

/// Parse the display name from a reference response (`{ data: { name } }`).
/// `None` for a missing or empty name — an absent name is a supported steady
/// state, never an error. Mirrors deckfeeder `reference.name`.
#[must_use]
pub fn parse_name(json: &impl JsonLookup) -> Option<String> {
    let name = json.str("/data/name")?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candle::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MapJson(BTreeMap<String, String>);
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
    fn reference_path_and_url_use_the_instrument_mapping() {
        assert_eq!(reference_path("BTC-USD"), "reference/BTC/USD");
        assert_eq!(reference_path("AAPL"), "reference/AAPL");
        assert_eq!(
            reference_url("https://nexus/api/v1/data/", "BTC-USD"),
            "https://nexus/api/v1/data/reference/BTC/USD"
        );
        assert_eq!(reference_url("b/", "^GSPC"), "b/reference/%5EGSPC");
    }

    #[test]
    fn name_is_trimmed_and_empty_becomes_none() {
        let mut json = MapJson::default();
        json.0.insert("/data/name".into(), "  Apple Inc.  ".into());
        assert_eq!(parse_name(&json), Some("Apple Inc.".to_owned()));

        let mut empty = MapJson::default();
        empty.0.insert("/data/name".into(), "   ".into());
        assert_eq!(parse_name(&empty), None);

        assert_eq!(parse_name(&MapJson::default()), None);
    }
}
