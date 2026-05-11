// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! Shared font loading for flip-clock widget
//!
//! Provides a single shared `FontRef` instance to avoid duplicate font parsing
//! overhead in both 2D texture generation and 3D mesh tessellation.

use ab_glyph::FontRef;
use std::sync::LazyLock;

/// Embedded font - Braiins Deck Sans Regular (weight 400)
const FONT_DATA: &[u8] = include_bytes!("../../../assets/fonts/BraiinsDeckSans-Regular.otf");

/// Shared font reference, parsed once and reused across all digit rendering.
pub static FONT: LazyLock<FontRef<'static>> = LazyLock::new(|| {
    FontRef::try_from_slice(FONT_DATA).expect("BUG: embedded font data is invalid")
});
