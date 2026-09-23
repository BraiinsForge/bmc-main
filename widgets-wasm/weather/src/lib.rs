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

//! Weather widget — current conditions and forecast, four sizes and BMM101's own frame.
//! Ported from `deckfeeder/assets/widgets/weather/` (a JS/HTML widget).
//!
//! - `api` — the Nexus endpoint, its reply statuses and the envelope read into a forecast
//! - `model` — the size buckets, the forecast and what the widget holds of it
//! - `display` — the forecast's figures and times as text
//! - `screens` — the views, their shared parts and the fixtures that stage them
//! - `live` — the widget entry points the host calls

pub mod api;
pub mod display;
#[cfg(target_arch = "wasm32")]
mod live;
mod manifest_params;
pub mod model;
pub mod screens;
pub mod weather_code;
pub mod wind;

pub use manifest_params::{Params, TimeZone};
