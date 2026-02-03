// Copyright (C) 2025  Braiins Systems s.r.o.

//! WASM Widget SDK for Braiins Deck.
//!
//! Provides host bindings and UI primitives for building widgets.
//! Layout is computed on the host side for minimal WASM binary size.

pub mod animation;
mod colors;
pub mod host;
pub mod tree;

pub use colors::*;
pub use host::{ButtonStyle, draw_text, fill_rect, request_frame, request_frame_after};
pub use tree::{
    Draw, Node, PropsData, TreeRenderResult,
    col, row, center, text, button, spacer, canvas, render_ui,
    rect, centered, orbit, rotated,
    begin_tree, finish_tree, with_buffer,
};

/// Shorthand for PropsData: `props!()` or `props!(gap: 16.0, background: 0xFF)`
#[macro_export]
macro_rules! props {
    () => { $crate::tree::PropsData::default() };
    ($($field:ident: $value:expr),* $(,)?) => {
        $crate::tree::PropsData { $($field: $value),*, ..Default::default() }
    };
}
