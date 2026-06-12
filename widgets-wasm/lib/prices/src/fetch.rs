// Copyright (C) 2026  Braiins Systems s.r.o.

//! Nexus price-resource path building and HTTP-status classification. Host-pure.

use crate::instrument::to_instrument;
use crate::period::Period;

/// Build the resource path `prices/<window>/<candle>/<instrument>` with the raw
/// (unencoded) instrument, e.g. `prices/7d/1h/BTC/USD`.
#[must_use]
pub fn resource_path(symbol: &str, period: Period) -> String {
    let mut out = String::from("prices/");
    out.push_str(period.window());
    out.push('/');
    out.push_str(period.candle().token());
    out.push('/');
    out.push_str(&to_instrument(symbol));
    out
}

/// Build the full Nexus URL from a base and the resource path. The instrument
/// may carry characters that need percent-encoding (`^GSPC`, `EURUSD=X`); each
/// resource char is encoded except the unreserved set and the `/` separators,
/// matching what the deckfeeder `URL` constructor produces.
#[must_use]
pub fn prices_url(base: &str, symbol: &str, period: Period) -> String {
    encode_resource_url(base, &resource_path(symbol, period))
}

/// Join `base` and a resource path, percent-encoding each resource byte except
/// the unreserved set and the `/` separators (matching what the deckfeeder `URL`
/// constructor produces). Shared by the price and reference URL builders.
#[must_use]
pub fn encode_resource_url(base: &str, resource: &str) -> String {
    let mut out = String::from(base);
    for byte in resource.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(char::from(byte));
            }
            other => push_percent_encoded(&mut out, other),
        }
    }
    out
}

fn push_percent_encoded(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[(byte >> 4) as usize]));
    out.push(char::from(HEX[(byte & 0x0f) as usize]));
}

/// How an HTTP status folds into a fetch outcome class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchClass {
    Ok,
    InputError,
    Warming,
    Transient,
}

/// Map an HTTP status to a fetch outcome class. 404/400 → `InputError` (a
/// user-correctable symbol/period); 503 → `Warming` (backend warming up); 2xx →
/// `Ok`; anything else → `Transient`.
#[must_use]
pub fn classify(status: u32) -> FetchClass {
    match status {
        200..=299 => FetchClass::Ok,
        400 | 404 => FetchClass::InputError,
        503 => FetchClass::Warming,
        _ => FetchClass::Transient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_path_for_a_crypto_pair() {
        assert_eq!(resource_path("BTC-USD", Period::D7), "prices/7d/1h/BTC/USD");
        assert_eq!(
            resource_path("BTC-USD", Period::Mo1),
            "prices/1mo/4h/BTC/USD"
        );
        assert_eq!(resource_path("AAPL", Period::D1), "prices/1d/15m/AAPL");
    }

    #[test]
    fn prices_url_joins_base_and_keeps_pair_slash() {
        assert_eq!(
            prices_url("https://nexus/api/v1/data/", "BTC-USD", Period::D7),
            "https://nexus/api/v1/data/prices/7d/1h/BTC/USD"
        );
    }

    #[test]
    fn prices_url_percent_encodes_index_and_fx_specials() {
        assert_eq!(
            prices_url("b/", "^GSPC", Period::D7),
            "b/prices/7d/1h/%5EGSPC"
        );
        assert_eq!(
            prices_url("b/", "EURUSD=X", Period::D7),
            "b/prices/7d/1h/EURUSD%3DX"
        );
    }

    #[test]
    fn classify_maps_each_status_band() {
        assert_eq!(classify(200), FetchClass::Ok);
        assert_eq!(classify(204), FetchClass::Ok);
        assert_eq!(classify(400), FetchClass::InputError);
        assert_eq!(classify(404), FetchClass::InputError);
        assert_eq!(classify(503), FetchClass::Warming);
        assert_eq!(classify(500), FetchClass::Transient);
        assert_eq!(classify(0), FetchClass::Transient);
    }
}
