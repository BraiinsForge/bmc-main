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

//! Nexus prices profile — a cloud API reached through testbed URL rewriting,
//! never LAN discovery, serving the price windows and instrument references
//! the ticker widgets read.
//!
//! Each instrument runs from the seven-day open to the close its fixture recorded,
//! whichever window is asked for.
//! Any other symbol answers 404 on both resources, as Nexus does for one it does not carry.

use std::borrow::Cow;
use std::fmt;

use chrono::{DateTime, Utc};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value as Json, json};

use crate::blueprint::{EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec};
use crate::http_status::HttpStatus;
use crate::noise;
use crate::quantity::NonNegative;

const DATA: &str = "/api/v1/data";
// The cache lifetimes Nexus advertised on the recorded replies.
const PRICES_TTL_SECS: u64 = 120;
const REFERENCE_TTL_SECS: u64 = 1_800;
/// Smooth-noise cells across a window, however many bars it holds.
const WIGGLES: f64 = 12.0;

/// A symbol from [`INSTRUMENTS`], checked against the table at load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbol(&'static str);

impl<'de> Deserialize<'de> for Symbol {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SymbolVisitor;

        impl Visitor<'_> for SymbolVisitor {
            type Value = Symbol;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a symbol the prices profile carries")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Symbol, E> {
                INSTRUMENTS
                    .iter()
                    .find(|instrument| instrument.symbol == value)
                    .map(|instrument| Symbol(instrument.symbol))
                    .ok_or_else(|| {
                        E::custom(format!(
                            "`{value}` is not a symbol the prices profile carries: {}",
                            symbols().join(", ")
                        ))
                    })
            }
        }

        // Rejected inside the visitor for the reason `HttpStatus` gives: json5 keeps the caret.
        deserializer.deserialize_str(SymbolVisitor)
    }
}

impl JsonSchema for Symbol {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("PricesSymbol")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "enum": symbols(),
            "description": "A symbol the prices profile carries, spelled as a widget is configured with it"
        })
    }
}

fn symbols() -> Vec<&'static str> {
    INSTRUMENTS
        .iter()
        .map(|instrument| instrument.symbol)
        .collect()
}

struct Instrument {
    /// Spelled as a widget is configured with it.
    symbol: &'static str,
    /// The path the widgets request it under: a pair split at its dash, `^` percent-encoded.
    path: &'static str,
    /// As Nexus spells it in its replies.
    nexus_symbol: &'static str,
    name: &'static str,
    exchange: &'static str,
    /// Absent on a pair, as on the recorded replies.
    currency: Option<&'static str>,
    /// The first open and the last close of the recorded seven-day window.
    open: f64,
    close: f64,
}

impl Instrument {
    fn listed_in(&self, symbols: &[Symbol]) -> bool {
        symbols.contains(&Symbol(self.symbol))
    }
}

static INSTRUMENTS: [Instrument; 12] = [
    Instrument {
        symbol: "NVDA",
        path: "NVDA",
        nexus_symbol: "NVDA",
        name: "NVIDIA Corporation",
        exchange: "NASDAQ",
        currency: Some("USD"),
        open: 208.2,
        close: 225.05,
    },
    Instrument {
        symbol: "AAPL",
        path: "AAPL",
        nexus_symbol: "AAPL",
        name: "Apple Inc.",
        exchange: "NASDAQ",
        currency: Some("USD"),
        open: 313.64,
        close: 305.69,
    },
    Instrument {
        symbol: "TSLA",
        path: "TSLA",
        nexus_symbol: "TSLA",
        name: "Tesla, Inc.",
        exchange: "NASDAQ",
        currency: Some("USD"),
        open: 399.1,
        close: 339.34,
    },
    Instrument {
        symbol: "MSTR",
        path: "MSTR",
        nexus_symbol: "MSTR",
        name: "Strategy Inc.",
        exchange: "NASDAQ",
        currency: Some("USD"),
        open: 94.78,
        close: 97.66,
    },
    Instrument {
        symbol: "JPM",
        path: "JPM",
        nexus_symbol: "JPM",
        name: "JPMorgan Chase & Co.",
        exchange: "NYSE",
        currency: Some("USD"),
        open: 327.0,
        close: 361.2,
    },
    Instrument {
        symbol: "META",
        path: "META",
        nexus_symbol: "META",
        name: "Meta Platforms, Inc. Class A Common Stock",
        exchange: "NASDAQ",
        currency: Some("USD"),
        open: 652.0,
        close: 569.01,
    },
    Instrument {
        symbol: "SPY",
        path: "SPY",
        nexus_symbol: "SPY",
        name: "SPDR S&P 500 ETF Trust",
        exchange: "NYSE",
        currency: Some("USD"),
        open: 750.91,
        close: 772.68,
    },
    Instrument {
        symbol: "NFLX",
        path: "NFLX",
        nexus_symbol: "NFLX",
        name: "Netflix, Inc.",
        exchange: "NASDAQ",
        currency: Some("USD"),
        open: 72.635,
        close: 76.03,
    },
    Instrument {
        symbol: "^GSPC",
        path: "%5EGSPC",
        nexus_symbol: "^GSPC",
        name: "S&P 500",
        exchange: "SNP",
        currency: Some("USD"),
        open: 7_735.18,
        close: 7_745.06,
    },
    Instrument {
        symbol: "BTC",
        path: "BTC",
        nexus_symbol: "BTC",
        name: "Grayscale Bitcoin Mini Trust ETF",
        exchange: "NYSE",
        currency: Some("USD"),
        open: 28.26,
        close: 28.42,
    },
    Instrument {
        symbol: "BTC-USD",
        path: "BTC/USD",
        nexus_symbol: "BTC/USD",
        name: "Bitcoin US Dollar",
        exchange: "Binance",
        currency: None,
        open: 64_023.51,
        close: 64_276.81,
    },
    Instrument {
        symbol: "EUR-USD",
        path: "EUR/USD",
        nexus_symbol: "EUR/USD",
        name: "Euro / US Dollar",
        exchange: "Forex",
        currency: None,
        open: 1.154_02,
        close: 1.157_69,
    },
];

struct Window {
    token: &'static str,
    candle: &'static str,
    candle_ms: i64,
    /// How many bars Nexus served for the window on the recorded replies.
    bars: usize,
}

static WINDOWS: [Window; 4] = [
    Window {
        token: "1h",
        candle: "1m",
        candle_ms: 60_000,
        bars: 60,
    },
    Window {
        token: "1d",
        candle: "15m",
        candle_ms: 900_000,
        bars: 100,
    },
    Window {
        token: "7d",
        candle: "1h",
        candle_ms: 3_600_000,
        bars: 175,
    },
    Window {
        token: "1mo",
        candle: "1d",
        candle_ms: 86_400_000,
        bars: 35,
    },
];

/// Scenario controls for the simulated Nexus prices and reference resources.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "PricesParams")]
pub struct Params {
    /// HTTP status every route returns after startup, or after `fail_after_secs`.
    pub status: HttpStatus,
    /// Answer 503 until this many seconds of scenario time have elapsed.
    pub warmup_secs: u32,
    /// Answer 200 before this point, then switch to `status`.
    pub fail_after_secs: Option<u32>,
    /// Instruments whose market is shut: their reference reads `is_market_open: false`.
    pub closed: Vec<Symbol>,
    /// Instruments whose windows hold no bars: their prices answer 404
    /// while their reference still resolves.
    pub empty: Vec<Symbol>,
    /// Instruments whose prices answer 503 while the others stay live.
    pub failing: Vec<Symbol>,
    /// Factor on every price, to push the list across magnitudes.
    pub price_scale: NonNegative,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            status: HttpStatus::OK,
            warmup_secs: 0,
            fail_after_secs: None,
            closed: Vec::new(),
            empty: Vec::new(),
            failing: Vec::new(),
            price_scale: NonNegative::from(1.0),
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let mut endpoints = Vec::new();
        for instrument in &INSTRUMENTS {
            let params = self.clone();
            endpoints.push(EndpointSpec {
                method: "GET".to_owned(),
                path: format!("{DATA}/reference/{}", instrument.path),
                response: ResponseSpec::computed(move |ctx| {
                    Response::new(params.status_at(ctx), reference(&params, instrument))
                }),
            });
            // Left unrouted, so the router's own 404 answers as Nexus does for an empty window.
            if instrument.listed_in(&self.empty) {
                continue;
            }
            for window in &WINDOWS {
                let params = self.clone();
                endpoints.push(EndpointSpec {
                    method: "GET".to_owned(),
                    path: format!(
                        "{DATA}/prices/{}/{}/{}",
                        window.token, window.candle, instrument.path
                    ),
                    response: ResponseSpec::computed(move |ctx| {
                        let status = if instrument.listed_in(&params.failing) {
                            HttpStatus::SERVICE_UNAVAILABLE
                        } else {
                            params.status_at(ctx)
                        };
                        let body = prices(&params, instrument, window, Utc::now(), ctx.seed);
                        Response::new(status, body)
                    }),
                });
            }
        }
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints,
            sampler: None,
        }
    }

    fn status_at(&self, ctx: &RequestCtx) -> HttpStatus {
        if ctx.t_s < f64::from(self.warmup_secs) {
            return HttpStatus::SERVICE_UNAVAILABLE;
        }
        if self
            .fail_after_secs
            .is_some_and(|after| ctx.t_s < f64::from(after))
        {
            HttpStatus::OK
        } else {
            self.status
        }
    }
}

fn reference(params: &Params, instrument: &Instrument) -> Json {
    let mut data = json!({
        "exchange": instrument.exchange,
        "instrument": instrument.nexus_symbol,
        "is_market_open": !instrument.listed_in(&params.closed),
        "name": instrument.name,
    });
    insert_currency(&mut data, "currency", instrument);
    json!({
        "cache_age_secs": 0,
        "data": data,
        "resource": format!("reference/{}", instrument.path),
        "ttl_secs": REFERENCE_TTL_SECS,
    })
}

fn insert_currency(data: &mut Json, key: &str, instrument: &Instrument) {
    if let Some(currency) = instrument.currency {
        data[key] = json!(currency);
    }
}

fn prices(
    params: &Params,
    instrument: &Instrument,
    window: &Window,
    now: DateTime<Utc>,
    seed: u64,
) -> Json {
    let mut data = json!({
        "candle_size": window.candle,
        "candles": candles(instrument, window, params.price_scale.get(), now, seed),
        "instrument": instrument.nexus_symbol,
    });
    insert_currency(&mut data, "quote_currency", instrument);
    json!({
        "cache_age_secs": 0,
        "data": data,
        "resource": format!("prices/{}/{}/{}", window.token, window.candle, instrument.path),
        "ttl_secs": PRICES_TTL_SECS,
    })
}

/// The window's bars up to the one `now` falls in:
/// a seeded wander about the straight line from open to close, as wide as the move itself.
#[expect(
    clippy::cast_precision_loss,
    reason = "a window holds a few hundred bars, exact in f64"
)]
fn candles(
    instrument: &Instrument,
    window: &Window,
    scale: f64,
    now: DateTime<Utc>,
    seed: u64,
) -> Vec<Json> {
    let open = instrument.open * scale;
    let close = instrument.close * scale;
    let seed = noise::mix(noise::mix(seed, instrument.path), window.token);
    let steps = i64::try_from(window.bars).expect("BUG: a window holds a few hundred bars");
    let last_ms = now.timestamp_millis().div_euclid(window.candle_ms) * window.candle_ms;
    let first_ms = last_ms - window.candle_ms * (steps - 1);
    let mut previous = open;
    (0..steps)
        .map(|step| {
            let progress = (step + 1) as f64 / steps as f64;
            let bar_close = if step + 1 == steps {
                close
            } else {
                let wander = noise::noise01(seed, progress * WIGGLES) - 0.5;
                open + (close - open) * progress + wander * (close - open).abs()
            };
            let bar_open = std::mem::replace(&mut previous, bar_close);
            json!({
                "t": first_ms + window.candle_ms * step,
                "o": bar_open,
                "h": bar_open.max(bar_close),
                "l": bar_open.min(bar_close),
                "c": bar_close,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::Instant;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    use super::*;
    use crate::cache::Cache;

    fn ctx(t_s: f64) -> RequestCtx {
        RequestCtx {
            query: BTreeMap::new(),
            t_s,
            seed: 1,
            host: None,
            cache: Arc::new(Cache::new::<Vec<_>>(Vec::new())),
        }
    }

    async fn get(params: &Params, path: &str) -> (StatusCode, Json) {
        let resource = params.resource("prices", 20_600);
        let cache = Arc::new(Cache::new::<Vec<_>>(Vec::new()));
        let router = crate::respond::build_router(resource.endpoints, 1, Instant::now(), &cache)
            .expect("BUG: the prices router builds");
        let request = Request::get(path)
            .body(Body::empty())
            .expect("BUG: a GET with an empty body builds");
        let response = router
            .oneshot(request)
            .await
            .expect("BUG: the router responds");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("BUG: the body reads");
        let json = if body.is_empty() {
            Json::Null
        } else {
            serde_json::from_slice(&body).expect("BUG: a non-empty body is JSON")
        };
        (status, json)
    }

    fn known(symbol: &str) -> Symbol {
        serde_json::from_value(json!(symbol)).expect("BUG: the table carries the symbol")
    }

    fn instrument(symbol: &str) -> &'static Instrument {
        INSTRUMENTS
            .iter()
            .find(|instrument| instrument.symbol == symbol)
            .expect("BUG: the table carries the symbol")
    }

    #[test]
    fn a_symbol_the_table_lacks_is_refused_where_it_sits() {
        let source = "{\n  closed: ['TSL'],\n}";
        let err = json5::from_str::<Params>(source).expect_err("BUG: TSL must be rejected");
        let json5::Error::Message { msg, location } = err;
        assert!(
            msg.contains("`TSL` is not a symbol the prices profile carries"),
            "message was: {msg}"
        );
        let location = location.expect("BUG: json5 must stamp the source location");
        assert_eq!(location.line, 2, "the symbol is on the second line");
    }

    #[test]
    fn the_schema_offers_only_what_the_check_accepts() {
        let schema = schemars::schema_for!(Symbol);
        let json = serde_json::to_value(&schema).expect("BUG: schema must serialize");
        let offered = json["enum"]
            .as_array()
            .expect("BUG: schema must enumerate symbols");
        assert_eq!(offered.len(), INSTRUMENTS.len());
        for symbol in offered {
            let symbol = symbol.as_str().expect("BUG: symbols are strings");
            known(symbol);
        }
    }

    #[tokio::test]
    async fn every_window_serves_the_bars_nexus_did() {
        for window in &WINDOWS {
            let path = format!(
                "/api/v1/data/prices/{}/{}/NVDA",
                window.token, window.candle
            );
            let (status, json) = get(&Params::default(), &path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            let candles = json["data"]["candles"]
                .as_array()
                .expect("BUG: a price reply carries candles");
            assert_eq!(candles.len(), window.bars, "{path}");
        }
    }

    #[tokio::test]
    async fn an_index_and_a_pair_route_as_the_widgets_encode_them() {
        let (status, _) = get(&Params::default(), "/api/v1/data/prices/7d/1h/%5EGSPC").await;
        assert_eq!(status, StatusCode::OK);
        let (status, json) = get(&Params::default(), "/api/v1/data/reference/BTC/USD").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["name"], "Bitcoin US Dollar");
        assert_eq!(json["data"].get("currency"), None);
    }

    #[tokio::test]
    async fn an_unknown_symbol_is_not_found_on_either_resource() {
        for path in [
            "/api/v1/data/prices/7d/1h/NONEXS",
            "/api/v1/data/reference/NONEXS",
        ] {
            let (status, _) = get(&Params::default(), path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn an_empty_window_still_resolves_its_instrument() {
        let params = Params {
            empty: vec![known("TSLA")],
            ..Params::default()
        };
        let (status, _) = get(&params, "/api/v1/data/prices/7d/1h/TSLA").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, json) = get(&params, "/api/v1/data/reference/TSLA").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["name"], "Tesla, Inc.");
    }

    #[tokio::test]
    async fn a_failing_symbol_degrades_only_its_own_prices() {
        let params = Params {
            failing: vec![known("JPM")],
            ..Params::default()
        };
        let (status, _) = get(&params, "/api/v1/data/prices/7d/1h/JPM").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let (status, _) = get(&params, "/api/v1/data/reference/JPM").await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = get(&params, "/api/v1/data/prices/7d/1h/NVDA").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[test]
    fn a_shut_market_reads_on_the_reference_alone() {
        let params = Params {
            closed: vec![known("MSTR")],
            ..Params::default()
        };
        assert_eq!(
            reference(&params, instrument("MSTR"))["data"]["is_market_open"],
            false
        );
        assert_eq!(
            reference(&params, instrument("NVDA"))["data"]["is_market_open"],
            true
        );
    }

    #[test]
    fn the_series_runs_from_the_recorded_open_to_its_close() {
        let now = DateTime::parse_from_rfc3339("2026-08-18T11:41:21Z")
            .expect("BUG: a fixed RFC3339 instant")
            .with_timezone(&Utc);
        let window = &WINDOWS[2];
        let bars = candles(instrument("NVDA"), window, 1.0, now, 1);
        assert_eq!(bars[0]["o"], 208.2);
        assert_eq!(bars[window.bars - 1]["c"], 225.05);
        let times: Vec<i64> = bars
            .iter()
            .map(|bar| bar["t"].as_i64().expect("BUG: t is integral"))
            .collect();
        assert!(times.is_sorted(), "bars must ascend");
        assert!(times[window.bars - 1] <= now.timestamp_millis());
    }

    #[test]
    fn the_scale_moves_every_price_and_keeps_the_change() {
        let now = Utc::now();
        let window = &WINDOWS[2];
        let bars = candles(instrument("BTC-USD"), window, 20_000.0, now, 1);
        assert_eq!(bars[0]["o"], 64_023.51 * 20_000.0);
        assert_eq!(bars[window.bars - 1]["c"], 64_276.81 * 20_000.0);
    }

    #[test]
    fn warmup_and_failure_transitions_are_time_driven() {
        let warming = Params {
            warmup_secs: 60,
            ..Params::default()
        };
        assert_eq!(
            warming.status_at(&ctx(59.0)),
            HttpStatus::SERVICE_UNAVAILABLE
        );
        assert_eq!(warming.status_at(&ctx(60.0)), HttpStatus::OK);

        let failing = Params {
            status: HttpStatus::SERVICE_UNAVAILABLE,
            fail_after_secs: Some(60),
            ..Params::default()
        };
        assert_eq!(failing.status_at(&ctx(59.0)), HttpStatus::OK);
        assert_eq!(
            failing.status_at(&ctx(60.0)),
            HttpStatus::SERVICE_UNAVAILABLE
        );
    }
}
