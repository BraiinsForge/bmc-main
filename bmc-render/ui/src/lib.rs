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

//! Design-system vocabulary and tree-wire primitives shared by the native renderer
//! and the widget SDK: colours, text styles, fills, arcs, node and draw tags,
//! the component builders and keyword macros over the tree,
//! renderer resource ids and the asset descriptors the `include_*!` macros emit.
//!
//! Pure on every target: no host calls and no `target_arch` gates,
//! so the renderer and a wasm widget compile the very same crate.

pub mod animation;
pub mod arc;
pub mod assets;
pub mod codec;
pub mod colors;
pub mod components;
pub mod display;
pub mod fill;
pub mod ids;
pub mod macros;
pub mod mesh;
pub mod nodes;
pub mod orientation;
pub mod partition;
pub mod progress;
pub mod relative_time;
pub mod skeleton;
pub mod skin;
pub mod svg;
pub mod text;
pub mod time;
pub mod tree;
pub mod typography;
pub(crate) mod wire;

pub use animation::*;
pub use arc::*;
pub use assets::*;
pub use codec::{
    DecodeError, Decoded, EncodeError, decode, encode, encode_into, try_encode, try_encode_into,
};
pub use colors::*;
pub use components::{
    modal::*, notification::*, number_input::*, progress_bar::*, relative_time::*,
    status_overlay::*, switcher::*, tag::*, text::*,
};
pub use display::{DisplayShape, ViewportShape};
pub use fill::*;
pub use ids::*;
pub use macros::__macro_string_from;
pub use mesh::*;
pub use nodes::*;
pub use orientation::*;
pub use progress::*;
pub use relative_time::*;
pub use skeleton::*;
pub use skin::*;
pub use svg::*;
pub use text::*;
pub use time::*;
pub use tree::*;
