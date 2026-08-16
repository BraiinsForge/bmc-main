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

//! Application-owned glyph atlas: cache identity and the page backend it
//! uploads through.

#[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
pub const PAGE_SIZE_PX: usize = 512;
#[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
pub const MAX_NORMAL_PAGES: usize = 10;
#[expect(dead_code, reason = "consumed in Task 2 (BDK-696)")]
pub const MAX_RESIDENT_ENTRIES: usize = 8192;
#[expect(dead_code, reason = "consumed in Task 5 (BDK-696)")]
pub const NEGATIVE_CACHE_CAP: usize = 256;
#[expect(dead_code, reason = "consumed in Task 6 (BDK-696)")]
pub const SCRATCH_MAP_CAP: usize = 1024;
#[expect(dead_code, reason = "consumed in Task 4 (BDK-696)")]
pub const MAX_EVICTIONS_PER_MISS: usize = 64;
#[expect(dead_code, reason = "consumed in Task 4 (BDK-696)")]
pub const FULL_RETRY_INTERVAL: usize = 8;

/// u64, not usize: generations must never wrap (eviction safety compares
/// them) and 32-bit usize would wrap on-device within device lifetime.
#[expect(dead_code, reason = "consumed in Task 2 (BDK-696)")]
pub type Generation = u64;

/// Cache identity: cosmic-text's key with subpixel bins forced to Zero.
/// Subpixel variants are deliberately not cached.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
pub struct GlyphKey(cosmic_text::CacheKey);

impl GlyphKey {
    #[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
    pub fn normalize(key: cosmic_text::CacheKey) -> Self {
        assert!(
            !key.flags.contains(cosmic_text::CacheKeyFlags::PIXEL_FONT),
            "PIXEL_FONT glyphs are unsupported: bins are positioning inputs there"
        );
        Self(cosmic_text::CacheKey {
            x_bin: cosmic_text::SubpixelBin::Zero,
            y_bin: cosmic_text::SubpixelBin::Zero,
            ..key
        })
    }

    #[expect(dead_code, reason = "consumed in Task 11 (BDK-696)")]
    pub fn inner(&self) -> cosmic_text::CacheKey {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
pub enum PageFaultKind {
    Invariant,
    Transient,
}

/// Page creation failures are transient by definition:
/// an Err carries no PageId, so there is nothing to quarantine —
/// the caller skips this frame, counts, and retries later.
/// Only `upload` can fault against a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
pub struct PageCreateFailed;

/// GL abstraction so the cache is unit-testable without a context;
/// the production impl wraps `Canvas<OpenGl>`.
/// Dimensions are `usize` end-to-end:
/// femtovg's `create_image_empty`/`update_image` take `usize`,
/// so matching it here means no conversions at the only real boundary.
#[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
pub trait PageBackend {
    type PageId: Copy + Eq + core::fmt::Debug;
    fn create_page(&mut self, size_px: usize) -> Result<Self::PageId, PageCreateFailed>;
    fn upload(
        &mut self,
        page: Self::PageId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[u8],
    ) -> Result<(), PageFaultKind>;
}

#[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
pub struct RasterGlyph {
    pub width: usize,
    pub height: usize,
    pub left: i32,
    pub top: i32,
    pub coverage: Vec<u8>,
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{PageBackend, PageCreateFailed, PageFaultKind};

    #[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
    pub(crate) struct MockPage {
        pub size_px: usize,
        pub pixels: Vec<u8>,
    }

    /// In-memory stand-in for the femtovg pages, recording every call so tests
    /// can assert on upload rects and the lifetime page budget, not just texels.
    #[derive(Default)]
    #[expect(dead_code, reason = "consumed in Task 3 (BDK-696)")]
    pub(crate) struct MockBackend {
        pub pages: Vec<MockPage>,
        /// Lifetime count, never decremented — the ≤ 21 page bound is about
        /// textures ever created, not textures currently held.
        pub pages_created: usize,
        pub uploads: Vec<(usize, usize, usize, usize, usize)>,
        pub fail_next_upload: Option<PageFaultKind>,
        pub fail_next_create: bool,
    }

    impl PageBackend for MockBackend {
        type PageId = usize;

        fn create_page(&mut self, size_px: usize) -> Result<Self::PageId, PageCreateFailed> {
            if self.fail_next_create {
                self.fail_next_create = false;
                return Err(PageCreateFailed);
            }
            self.pages.push(MockPage {
                size_px,
                pixels: vec![0; size_px * size_px],
            });
            self.pages_created += 1;
            Ok(self.pages.len() - 1)
        }

        fn upload(
            &mut self,
            page: Self::PageId,
            x: usize,
            y: usize,
            width: usize,
            height: usize,
            pixels: &[u8],
        ) -> Result<(), PageFaultKind> {
            if let Some(kind) = self.fail_next_upload.take() {
                return Err(kind);
            }

            let target = self
                .pages
                .get_mut(page)
                .expect("BUG: upload to unknown page");
            assert_eq!(
                pixels.len(),
                width * height,
                "BUG: coverage size disagrees with upload rect"
            );
            assert!(
                x + width <= target.size_px && y + height <= target.size_px,
                "BUG: upload rect leaves the page"
            );

            for row in 0..height {
                let src = row * width;
                let dst = (y + row) * target.size_px + x;
                target.pixels[dst..dst + width].copy_from_slice(&pixels[src..src + width]);
            }
            self.uploads.push((page, x, y, width, height));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_text::SubpixelBin;

    const SANS: &str = "Braiins Sans";
    const DECK_SANS: &str = "Braiins Deck Sans";

    /// Shape through the embedded faces, exactly as the draw path does,
    /// and take the first glyph's physical key — so `font_id`, `font_weight`
    /// and flags are the ones production really produces.
    fn shape_one_glyph(text: &str, font_size: f32, family: &str) -> cosmic_text::CacheKey {
        let mut font_system = crate::gpu::renderer::build_font_system();
        let mut buffer = cosmic_text::Buffer::new(
            &mut font_system,
            cosmic_text::Metrics::new(font_size, font_size),
        );
        buffer.set_text(
            &mut font_system,
            text,
            &cosmic_text::Attrs::new().family(cosmic_text::Family::Name(family)),
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        buffer
            .layout_runs()
            .next()
            .expect("BUG: shaping produced no layout run")
            .glyphs
            .first()
            .expect("BUG: shaped layout run carries no glyph")
            .physical((0.0, 0.0), 1.0)
            .cache_key
    }

    fn any_key(x_bin: SubpixelBin, y_bin: SubpixelBin) -> cosmic_text::CacheKey {
        let mut key = shape_one_glyph("A", 17.0, SANS);
        key.x_bin = x_bin;
        key.y_bin = y_bin;
        key
    }

    fn with_other_font(key: cosmic_text::CacheKey) -> cosmic_text::CacheKey {
        let other = shape_one_glyph("A", 17.0, DECK_SANS);
        assert_ne!(
            other.font_id, key.font_id,
            "the two families must resolve to different faces for this to separate anything"
        );
        cosmic_text::CacheKey {
            font_id: other.font_id,
            ..key
        }
    }

    fn with_weight(key: cosmic_text::CacheKey, weight: u16) -> cosmic_text::CacheKey {
        cosmic_text::CacheKey {
            font_weight: cosmic_text::fontdb::Weight(weight),
            ..key
        }
    }

    fn with_fake_italic(key: cosmic_text::CacheKey) -> cosmic_text::CacheKey {
        cosmic_text::CacheKey {
            flags: key.flags | cosmic_text::CacheKeyFlags::FAKE_ITALIC,
            ..key
        }
    }

    #[test]
    fn all_sixteen_bin_combinations_normalize_to_one_key() {
        let bins = [
            SubpixelBin::Zero,
            SubpixelBin::One,
            SubpixelBin::Two,
            SubpixelBin::Three,
        ];
        let base = GlyphKey::normalize(any_key(SubpixelBin::Zero, SubpixelBin::Zero));
        for x in bins {
            for y in bins {
                assert_eq!(GlyphKey::normalize(any_key(x, y)), base);
            }
        }
    }

    /// Normalization erases the bins and nothing else:
    /// cosmic-text picks the face, the variable-font weight
    /// and synthetic italic per glyph,
    /// so folding any of those together would show one rasterization
    /// for two visibly different glyphs.
    #[test]
    fn font_weight_and_flags_do_not_alias() {
        let base = any_key(SubpixelBin::Zero, SubpixelBin::Zero);
        for variant in [
            with_other_font(base),
            with_weight(base, 700),
            with_fake_italic(base),
        ] {
            assert_ne!(GlyphKey::normalize(variant), GlyphKey::normalize(base));
        }
    }

    #[test]
    #[should_panic(expected = "PIXEL_FONT")]
    fn pixel_font_flag_is_rejected() {
        let mut key = any_key(SubpixelBin::Zero, SubpixelBin::Zero);
        key.flags = cosmic_text::CacheKeyFlags::PIXEL_FONT;
        let _ = GlyphKey::normalize(key);
    }
}
