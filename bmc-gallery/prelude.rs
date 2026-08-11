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

//! Re-exports for scene authors: the whole gallery prelude, the Deck kit, and
//! the SDK a scene builds its tree with.
//!
//! ```ignore
//! use bmc_gallery::prelude::*;
//! ```

pub use gallery::prelude::*;

pub use crate::kit::DeckSize::{Auto, Full, Large, Medium, Page, Round, Small};
pub use crate::kit::DivHeight::Auto as AutoH;
pub use crate::kit::{
    AUTO_HEIGHT_MAX, ActionEvent, CustomRenderFn, DEVICE_HEIGHT, DEVICE_WIDTH, DeckSceneCtx,
    DeckSize, DivHeight, Fired, RodioSink,
};

pub use bmc_wasm_sdk::tree::*;
pub use bmc_wasm_sdk::*;

/// Convert a pixel value to `f32`.
///
/// Pixel coordinates in scenes are always small enough
/// for lossless `u32 → f32` conversion.
#[must_use]
#[inline]
#[expect(
    clippy::cast_precision_loss,
    reason = "the doc above is the contract: scene pixel coordinates are small"
)]
pub const fn px(v: u32) -> f32 {
    v as f32
}

/// Narrow an index for the tree's `u32` fields — scenes count in the dozens.
#[must_use]
#[inline]
#[expect(
    clippy::cast_possible_truncation,
    reason = "a scene indexes a handful of cells, never four billion"
)]
pub const fn idx(v: usize) -> u32 {
    v as u32
}
