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
pub use crate::{DivHeight, DocBlock, StoryUi};

// SDK re-exports so stories just need `use crate::prelude::*`
pub use bmc_wasm_sdk::tree::*;
pub use bmc_wasm_sdk::*;
