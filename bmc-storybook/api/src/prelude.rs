// Copyright (C) 2026  Braiins Systems s.r.o.

//! Re-exports for story authors.
//!
//! ```ignore
//! use crate::prelude::*;
//! ```

pub use crate::knobs::{
    ColorKnob, Nudge, Pad2DKnob, Pad2DSpec, SelectKnob, SliderKnob, StoryCtx, TextKnob, ToggleKnob,
};
/// `story_meta!{}` — file-level group declaration (`story_meta! { title: "Button" }`)
pub use crate::story_meta;
/// `#[story]` — attribute for marking individual story functions
pub use bmc_storybook_macros::story;

// Document model types
pub use crate::DivHeight::Auto as AutoH;
pub use crate::FrameSize::{self, Auto, Full, Large, Medium, Small};
pub use crate::{CustomRenderFn, DivHeight, DocBlock, StoryUi};

// Audio sink shared between the storybook bin and cdylib.
pub use crate::audio::RodioSink;

// SDK re-exports so stories just need `use crate::prelude::*`
pub use bmc_wasm_sdk::tree::*;
pub use bmc_wasm_sdk::*;

/// Convert a pixel value to `f32` without clippy noise.
///
/// Pixel coordinates in storybook stories are always small enough
/// for lossless `u32 → f32` conversion.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "pixel coordinates are small")]
#[inline]
pub const fn px(v: u32) -> f32 {
    v as f32
}

/// Convert a `usize` index to `u32` for pixel math.
#[must_use]
#[expect(clippy::cast_possible_truncation, reason = "story indices are small")]
#[inline]
pub const fn idx(v: usize) -> u32 {
    v as u32
}

/// Integer division for grid math without clippy noise.
#[must_use]
#[expect(
    clippy::integer_division,
    reason = "intentional for grid row/col index"
)]
#[inline]
pub const fn grid_div(a: u32, b: u32) -> u32 {
    a / b
}
