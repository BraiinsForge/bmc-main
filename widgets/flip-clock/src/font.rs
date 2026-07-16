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
