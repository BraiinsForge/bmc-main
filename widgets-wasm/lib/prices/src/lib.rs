// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared price-series logic for the ticker widgets: period/candle mapping,
//! symbol→instrument mapping, the Nexus candle envelope parser, the market-open
//! recency heuristic, sparkline geometry, and HTTP-status classification.
//!
//! Everything here is host-pure and unit-tested except the `wasm32`-gated
//! `impl JsonLookup for JsonDoc` in [`candle`]. The widget owns the poll loop
//! and the live `fetch` call; this crate only builds paths and parses bodies.

pub mod candle;
pub mod chart;
pub mod fetch;
pub mod format;
pub mod instrument;
pub mod period;
pub mod reference;
