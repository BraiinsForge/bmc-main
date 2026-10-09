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
//! The design-system vocabulary and tree-wire primitives live in `bmc-ui`;
//! this crate re-exports them so one dependency covers the whole contract.

pub mod assets;
pub mod fetch;
pub mod ids;
pub mod image_meta;
pub mod json;
pub mod params;
pub mod system;
pub mod tags;
pub mod time;
pub mod timer;
pub mod version;
pub mod versioned_snapshot;
#[cfg(feature = "host")]
pub mod xml;

pub use bmc_ui::{
    animation, arc, colors, display, fill, mesh, nodes, progress, relative_time, skeleton, svg,
};

/// Not in the list above: at `bmc_ui`'s root the `text` module
/// sits beside the `text` node builder, which a plain re-export would carry along.
pub mod text {
    pub use bmc_ui::text::*;
}

pub use animation::*;
pub use arc::*;
pub use assets::*;
pub use colors::*;
pub use display::{DisplayShape, ViewportShape};
pub use fetch::{FetchOutcome, MediaTypePart};
pub use fill::*;
pub use ids::*;
pub use image_meta::{decode_image_meta, encode_image_meta};
pub use mesh::{
    FLAG_HAS_NORMAL_MAP, FLAG_HAS_TANGENTS, FLAG_HAS_TEXTURE, FLAG_HAS_UVS, HEADER_SIZE,
    MAX_TEXTURE_SIZE, MAX_TRIANGLES, MAX_VERTICES, MESH_MAGIC, TextureFormat, VERTEX_SIZE,
    VERTEX_SIZE_NO_UV,
};
pub use nodes::*;
pub use progress::*;
pub use relative_time::*;
pub use skeleton::*;
pub use svg::*;
pub use tags::*;
pub use text::*;
pub use time::*;
pub use timer::*;
pub use version::*;
