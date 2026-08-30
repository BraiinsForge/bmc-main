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

//! Shared price-series logic for the ticker widgets: period/candle mapping,
//! symbol→instrument mapping, the Nexus candle envelope parser, sparkline
//! geometry, closed-market presentation, and HTTP-status classification.
//!
//! Everything here is host-pure and unit-tested except the `wasm32`-gated
//! `impl JsonLookup for JsonDoc` in [`candle`]. The widget owns the poll loop
//! and the live `fetch` call; this crate only builds paths and parses bodies.

/// Default Nexus data API base URL the ticker widgets fetch from.
pub const NEXUS_BASE: &str = "https://nexus.braiinsforge.com/api/v1/data/";

/// Per-call fetch timeout for Nexus requests. Nexus long-polls a cold
/// windowed resource for ~13 s before answering, so this must exceed that
/// or the reply degrades into a client-side timeout.
pub const FETCH_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(15);

pub mod candle;
pub mod chart;
pub mod closed_market;
pub mod fetch;
pub mod format;
pub mod instrument;
pub mod period;
pub mod reference;
pub mod transition;
