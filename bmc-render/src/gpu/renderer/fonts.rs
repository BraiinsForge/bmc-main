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

//! The faces the renderer draws with, mapped from their files,
//! and the cosmic-text font system over them.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use cosmic_text::fontdb;
use femtovg::{FontId, TextContext};
use memmap2::Mmap;

/// Resolved by `build.rs`: the font store path in builds Nix makes,
/// the checkout's own `assets/fonts` in everything cargo builds from it.
pub(super) const FONTS_DIR: &str = env!("BMC_RENDER_FONTS_DIR");

/// A face the renderer loads, and what fontdb must parse out of its file.
pub(crate) struct Face {
    pub(super) file: &'static str,
    pub(crate) post_script_name: &'static str,
    pub(crate) weight: u16,
}

/// A [`Face`] whose file the build checks is a TrueType or OpenType font.
/// A checkout without Git LFS holds pointer files in their place,
/// which would otherwise fail only once the host starts.
///
/// The bytes are read by a compile-time assert alone,
/// so none of them reach the binary.
macro_rules! face {
    ($file:literal, $post_script_name:literal, $weight:literal) => {{
        const _: () = assert!(
            is_sfnt(include_bytes!(concat!(
                env!("BMC_RENDER_FONTS_DIR"),
                "/",
                $file
            ))),
            concat!(
                $file,
                " is not a TrueType or OpenType font; is Git LFS set up?"
            ),
        );
        Face {
            file: $file,
            post_script_name: $post_script_name,
            weight: $weight,
        }
    }};
}

/// Whether `data` opens with an sfnt version tag:
/// TrueType's `00 01 00 00` or `true`, OpenType CFF's `OTTO`.
const fn is_sfnt(data: &[u8]) -> bool {
    matches!(
        data,
        [0x00, 0x01, 0x00, 0x00, ..] | [b'O', b'T', b'T', b'O', ..] | [b't', b'r', b'u', b'e', ..]
    )
}

/// The faces in load order.
/// [`build_font_table`] zips this order with femtovg's, so a divergence here
/// would map every glyph to the wrong face.
///
/// [`build_font_table`]: crate::gpu::text::build_font_table
pub(crate) const FACES: [Face; 7] = [
    face!("BraiinsSans-Regular.otf", "BraiinsSans", 400),
    face!("BraiinsSans-SemiBold.otf", "BraiinsSans-SemiBold", 600),
    face!("BraiinsSans-Bold.otf", "BraiinsSans-Bold", 700),
    // BraiinsDeckSans is the display face used by the legacy slint deck;
    // widgets opt in via `FontFamily::DeckSans` in their `TextStyle`.
    face!(
        "BraiinsDeckSans-Regular.otf",
        "BraiinsDeckSans-Regular",
        400
    ),
    face!(
        "BraiinsDeckSans-SemiBold.otf",
        "BraiinsDeckSans-SemiBold",
        600
    ),
    face!("BraiinsDeckSans-Bold.otf", "BraiinsDeckSans-Bold", 700),
    // Covers the glyphs the Braiins faces don't (Greek variants, symbols, …).
    face!("NotoSans-Regular.ttf", "NotoSans-Regular", 400),
];

/// Every face in [`FACES`], each file mapped once.
/// fontdb and femtovg both read the same mapping,
/// so the bytes stay clean, file-backed pages rather than heap copies.
pub(crate) struct Fonts {
    db: fontdb::Database,
    pub(super) maps: Vec<Arc<Mmap>>,
}

impl Fonts {
    /// Map every face from the font directory baked in at build time.
    ///
    /// # Errors
    ///
    /// Names the file that cannot be mapped or does not hold its face.
    pub(crate) fn load() -> Result<Self> {
        Self::load_from(Path::new(FONTS_DIR))
    }

    pub(super) fn load_from(dir: &Path) -> Result<Self> {
        let mut db = fontdb::Database::new();
        let mut maps = Vec::with_capacity(FACES.len());
        for face in &FACES {
            let path = dir.join(face.file);
            let map = Arc::new(
                map_file(&path).with_context(|| format!("map the font face {}", path.display()))?,
            );
            let ids = db.load_font_source(fontdb::Source::SharedFile(
                path.clone(),
                Arc::clone(&map) as _,
            ));
            let parsed: Vec<_> = ids
                .iter()
                .filter_map(|&id| db.face(id))
                .map(|info| (info.post_script_name.as_str(), info.weight.0))
                .collect();
            ensure!(
                parsed == [(face.post_script_name, face.weight)],
                "{} holds {parsed:?}, not {} at weight {}",
                path.display(),
                face.post_script_name,
                face.weight,
            );
            maps.push(map);
        }
        Ok(Self { db, maps })
    }

    /// Register every face with femtovg, in [`FACES`] order,
    /// over the mappings fontdb already reads.
    pub(crate) fn register_femtovg(
        &self,
        text_context: &TextContext,
    ) -> Result<[FontId; FACES.len()]> {
        let ids = self
            .maps
            .iter()
            .zip(&FACES)
            .map(|(map, face)| {
                text_context
                    .add_shared_font_with_index(SharedFace(Arc::clone(map)), 0)
                    .with_context(|| format!("register the font face {} with femtovg", face.file))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ids
            .try_into()
            .expect("BUG: `load_from` maps one file per face"))
    }

    /// One mapping per process, for tests that shape or draw text.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn shared() -> &'static Self {
        static FONTS: std::sync::LazyLock<Fonts> = std::sync::LazyLock::new(|| {
            Fonts::load().expect("BUG: the font directory carries every face")
        });
        &FONTS
    }
}

fn map_file(path: &Path) -> std::io::Result<Mmap> {
    let file = File::open(path)?;
    // SAFETY: a store path is read-only, so the file cannot change under the mapping.
    // Dev and test builds read the checkout instead; don't edit a font while one runs.
    unsafe { Mmap::map(&file) }
}

/// femtovg takes any `AsRef<[u8]>`; `Arc<Mmap>` only derefs to one.
struct SharedFace(Arc<Mmap>);

impl AsRef<[u8]> for SharedFace {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// cosmic-text `FontSystem` holding [`FACES`] and nothing else.
///
/// Paragraphs request "Braiins Sans" or "Braiins Deck Sans" by family name (see
/// [`crate::gpu::text::build_attrs`]), so cosmic-text always prefers them; Noto Sans
/// covers only the glyphs the Braiins faces don't (Greek, Cyrillic, …).
///
/// Deliberately not `FontSystem::new()`, which scans the host's installed fonts:
/// shaping would then vary with whatever the machine happens to have, and the
/// Nix build sandbox has nothing at all — cosmic-text panics there with "no
/// default font found". Tests shape through this for the same reason.
pub(crate) fn build_font_system(fonts: &Fonts) -> cosmic_text::FontSystem {
    cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), fonts.db.clone())
}
