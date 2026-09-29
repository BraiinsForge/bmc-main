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

#![allow(clippy::cast_precision_loss)]

//! SpaceX Launch widget for the WASM runtime (BDK-285).
//!
//! Renders the next SpaceX launch as a countdown plus mission details
//! (full/large/medium/small, and BMM101's own frame). Data comes from nexus
//! (`/api/v1/data/spacex/next-launch`), which normalizes and caches the
//! upstream Launch Library 2 feed; the countdown is ticked locally from the
//! device clock between refreshes.
//!
//! - `api` — the Nexus endpoint, what a reply means, and the envelope read into a launch
//! - `model` — the size buckets and the launch the views draw
//! - `screens` — the views, their shared parts and the fixtures that stage them
//! - `live` — the widget entry points the host calls

pub mod api;
#[cfg(target_arch = "wasm32")]
mod live;
pub mod model;
pub mod screens;
