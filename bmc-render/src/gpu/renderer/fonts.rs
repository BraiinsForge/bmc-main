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

//! The faces embedded in the binary, and the cosmic-text font system over them.

use cosmic_text::fontdb;

// Embed BraiinsSans fonts at compile time from the top-level assets directory.
pub(super) const FONT_REGULAR: &[u8] =
    include_bytes!("../../../../assets/fonts/BraiinsSans-Regular.otf");

pub(super) const FONT_SEMIBOLD: &[u8] =
    include_bytes!("../../../../assets/fonts/BraiinsSans-SemiBold.otf");

pub(super) const FONT_BOLD: &[u8] = include_bytes!("../../../../assets/fonts/BraiinsSans-Bold.otf");

// BraiinsDeckSans is the display face used by the legacy slint deck;
// widgets opt in via `FontFamily::DeckSans` in their `TextStyle`.
pub(super) const FONT_DECK_REGULAR: &[u8] =
    include_bytes!("../../../../assets/fonts/BraiinsDeckSans-Regular.otf");

pub(super) const FONT_DECK_SEMIBOLD: &[u8] =
    include_bytes!("../../../../assets/fonts/BraiinsDeckSans-SemiBold.otf");

pub(super) const FONT_DECK_BOLD: &[u8] =
    include_bytes!("../../../../assets/fonts/BraiinsDeckSans-Bold.otf");

/// Fallback font for glyphs not covered by the Braiins faces (Greek, symbols, etc.).
pub(super) const FONT_FALLBACK: &[u8] =
    include_bytes!("../../../../assets/fonts/NotoSans-Regular.ttf");

/// cosmic-text `FontSystem` holding the embedded faces and nothing else.
///
/// Paragraphs request "Braiins Sans" or "Braiins Deck Sans" by family name (see
/// [`crate::gpu::text::build_attrs`]), so cosmic-text always prefers them; Noto Sans
/// covers only the glyphs the Braiins faces don't (Greek, Cyrillic, …).
///
/// Deliberately not `FontSystem::new()`, which scans the host's installed fonts:
/// shaping would then vary with whatever the machine happens to have, and the
/// Nix build sandbox has nothing at all — cosmic-text panics there with "no
/// default font found". Tests shape through this for the same reason.
pub(crate) fn build_font_system() -> cosmic_text::FontSystem {
    let mut db = fontdb::Database::new();
    db.load_font_data(FONT_REGULAR.to_vec());
    db.load_font_data(FONT_SEMIBOLD.to_vec());
    db.load_font_data(FONT_BOLD.to_vec());
    db.load_font_data(FONT_DECK_REGULAR.to_vec());
    db.load_font_data(FONT_DECK_SEMIBOLD.to_vec());
    db.load_font_data(FONT_DECK_BOLD.to_vec());
    db.load_font_data(FONT_FALLBACK.to_vec());
    cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), db)
}
