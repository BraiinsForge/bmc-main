// Copyright (C) 2025  Braiins Systems s.r.o.

//! WASM Widget SDK for Braiins Deck.
//!
//! Provides host bindings, layout, and UI primitives for building widgets.

mod colors;
pub mod host;
pub mod ui;

pub use colors::*;
pub use host::{ButtonStyle, draw_text, fill_rect, request_frame, request_frame_after};
pub use ui::*;

/// Shorthand for Props: `props!()` or `props!(gap: 16.0, background: 0xFF)`
#[macro_export]
macro_rules! props {
    () => { $crate::ui::Props::default() };
    ($($field:ident: $value:expr),* $(,)?) => {
        $crate::ui::Props { $($field: $value),*, ..Default::default() }
    };
}

// Re-export taffy for advanced layout needs
pub use taffy;
