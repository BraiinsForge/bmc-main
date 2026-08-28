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

//! Shared protocol definitions for WASM widgets.
//! This crate contains constants and types shared between the SDK (WASM) and host.

pub mod animation;
pub mod arc;
pub mod assets;
pub mod colors;
pub mod display;
pub mod fetch;
pub mod fill;
pub mod ids;
pub mod image_meta;
pub mod mesh;
pub mod nodes;
pub mod params;
pub mod progress;
pub mod relative_time;
pub mod skeleton;
pub mod svg;
pub mod system;
pub mod tags;
pub mod text;
pub mod time;
pub mod version;
pub mod versioned_snapshot;
pub(crate) mod wire;

pub use animation::*;
pub use arc::*;
pub use assets::*;
pub use colors::*;
pub use display::{DisplayShape, ViewportShape};
pub use fetch::{FetchOutcome, MediaTypePart};
pub use fill::*;
pub use ids::*;
pub use image_meta::{decode_image_meta, encode_image_meta};
pub use mesh::*;
pub use nodes::*;
pub use progress::*;
pub use relative_time::*;
pub use skeleton::*;
pub use svg::*;
pub use tags::*;
pub use text::*;
pub use time::*;
pub use version::*;
