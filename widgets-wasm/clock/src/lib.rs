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

//! Clock widget — three faces (analog round / analog rectangular / digital)
//! drawn as views of a moment, a viewport and the operator's params,
//! over the system snapshot.
//!
//! - `model` — the size buckets, the face a viewport draws and how its hands move
//! - `screens` — the views, their shared parts and the fixtures that stage them
//! - `live` — the widget entry points the host calls

#[cfg(target_arch = "wasm32")]
mod live;
mod manifest_params;
pub mod model;
pub mod screens;

pub use manifest_params::{ClockStyle, NumbersFontStyle, Params};
