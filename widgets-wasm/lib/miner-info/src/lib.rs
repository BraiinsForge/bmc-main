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

//! Shared data layer for the Miner Info widgets.
//!
//! `model` holds the two data bags the faces render from; `api` and `public`
//! build the BOS and Braiins public-API requests and fold their replies into
//! those bags; `format` and `layout` turn the values and the viewport into the
//! strings and metrics `face` draws with.
//!
//! Every dimensional quantity comes from [`bmc_wasm_sdk::units`]. `money` holds
//! the fiat figures, which are not dimensional — there is no canonical currency
//! and no rate-free conversion — so they carry their currency as data instead.
//! `availability` carries the "no value yet" state a plain quantity cannot
//! express.
//!
//! `engine` is the crate's one `wasm32`-gated module: it owns the poll loop
//! and the live `fetch`. Everything else is host-pure and unit-tested.
//! Each widget hands it a config reader at `init`, and the `view` it returns
//! picks that widget's endpoints.

pub mod api;
pub mod availability;
pub mod engine;
pub mod face;
pub mod format;
pub mod layout;
pub mod model;
pub mod money;
pub mod public;
