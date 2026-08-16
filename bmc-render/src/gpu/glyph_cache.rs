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

pub const PAGE_SIZE_PX: usize = 512;
pub const MAX_NORMAL_PAGES: usize = 10;
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
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
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 11 (BDK-696)"))]
pub struct PageCreateFailed;

/// GL abstraction so the cache is unit-testable without a context;
/// the production impl wraps `Canvas<OpenGl>`.
/// Dimensions are `usize` end-to-end:
/// femtovg's `create_image_empty`/`update_image` take `usize`,
/// so matching it here means no conversions at the only real boundary.
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 11 (BDK-696)"))]
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

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 9 (BDK-696)"))]
pub struct RasterGlyph {
    pub width: usize,
    pub height: usize,
    pub left: i32,
    pub top: i32,
    pub coverage: Vec<u8>,
}

/// Glyph bitmap geometry from swash's `Placement`,
/// carried on the entry so a hit can emit a quad without re-rasterizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RasterPlacement {
    pub left: i32,
    pub top: i32,
    pub width: usize,
    pub height: usize,
}

/// One resident glyph: where it lives in the atlas, how to draw it,
/// and its place in the LRU order.
/// `prev`/`next` are slab indices rather than `usize`
/// — an accepted exception to the workspace usize-for-indices rule —
/// so the entry fits the spec's 96-byte metadata budget on 64-bit hosts;
/// on the 32-bit target the two are the same width.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
pub struct Entry {
    pub key: GlyphKey,
    pub page: usize,
    pub alloc_id: etagere::AllocId,
    pub content_x: usize,
    pub content_y: usize,
    pub placement: RasterPlacement,
    pub last_used: Generation,
    pub prev: u32,
    pub next: u32,
}

/// Absent neighbour; also terminates the free-slot chain.
pub const NO_LINK: u32 = u32::MAX;

/// Fixed-capacity storage for resident entries.
/// Slots are never released to the allocator: a freed slot joins a chain
/// threaded through `next` and is handed back by the next `alloc`.
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
pub struct EntrySlab {
    entries: Vec<Entry>,
    free_head: u32,
    live: usize,
}

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
impl EntrySlab {
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(
            u32::try_from(capacity).is_ok_and(|as_u32| as_u32 < NO_LINK),
            "BUG: slab capacity outside the slab index range"
        );
        Self {
            entries: Vec::with_capacity(capacity),
            free_head: NO_LINK,
            live: 0,
        }
    }

    pub fn alloc(&mut self, entry: Entry) -> Option<u32> {
        let capacity = self.entries.capacity();
        let index = if self.free_head == NO_LINK {
            if self.entries.len() == capacity {
                return None;
            }
            self.entries.push(entry);
            let index = self.entries.len() - 1;
            u32::try_from(index).expect("BUG: slab index outside the slab index range")
        } else {
            let index = self.free_head;
            let slot = self.get_mut(index);
            let next_free = slot.next;
            *slot = entry;
            self.free_head = next_free;
            index
        };

        debug_assert_eq!(
            self.entries.capacity(),
            capacity,
            "BUG: entry slab grew past its preallocated capacity"
        );
        self.live += 1;
        Some(index)
    }

    pub fn free(&mut self, index: u32) {
        let free_head = self.free_head;
        self.get_mut(index).next = free_head;
        self.free_head = index;
        self.live -= 1;
    }

    pub fn get(&self, index: u32) -> &Entry {
        self.entries
            .get(index as usize)
            .expect("BUG: slab index refers to no slot")
    }

    pub fn get_mut(&mut self, index: u32) -> &mut Entry {
        self.entries
            .get_mut(index as usize)
            .expect("BUG: slab index refers to no slot")
    }

    /// Slots ever handed out, not slots in use —
    /// never grows past the preallocation,
    /// so the cache's storage bound is this number.
    pub fn capacity(&self) -> usize {
        self.entries.capacity()
    }

    /// Live entries. `entries.len()` is the high-water mark and counts freed
    /// slots too, so it cannot answer this.
    pub fn len(&self) -> usize {
        self.live
    }
}

/// LRU order over slab entries, threaded through their `prev`/`next` links:
/// `head` is the most recently used entry, `tail` the eviction candidate.
/// Holding the links in the entries is what keeps promotion allocation-free.
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
pub struct LruQueue {
    head: u32,
    tail: u32,
}

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
impl LruQueue {
    pub fn new() -> Self {
        Self {
            head: NO_LINK,
            tail: NO_LINK,
        }
    }

    pub fn push_hot(&mut self, slab: &mut EntrySlab, index: u32) {
        let old_head = self.head;
        let entry = slab.get_mut(index);
        entry.prev = NO_LINK;
        entry.next = old_head;

        if old_head == NO_LINK {
            self.tail = index;
        } else {
            slab.get_mut(old_head).prev = index;
        }
        self.head = index;
    }

    pub fn unlink(&mut self, slab: &mut EntrySlab, index: u32) {
        let entry = slab.get_mut(index);
        let (prev, next) = (entry.prev, entry.next);
        debug_assert!(
            (prev != NO_LINK || self.head == index) && (next != NO_LINK || self.tail == index),
            "BUG: unlinking an entry the queue does not hold"
        );
        entry.prev = NO_LINK;
        entry.next = NO_LINK;

        if prev == NO_LINK {
            self.head = next;
        } else {
            slab.get_mut(prev).next = next;
        }
        if next == NO_LINK {
            self.tail = prev;
        } else {
            slab.get_mut(next).prev = prev;
        }
    }

    pub fn promote(&mut self, slab: &mut EntrySlab, index: u32) {
        self.unlink(slab, index);
        self.push_hot(slab, index);
    }

    pub fn coldest(&self) -> Option<u32> {
        (self.tail != NO_LINK).then_some(self.tail)
    }
}

/// One atlas page: the backend's image plus the allocator that packs it.
/// `alloc` is an `Option` so quarantine can drop the allocator's metadata
/// while the page struct and its image stay retained.
struct Page<P> {
    id: P,
    alloc: Option<etagere::BucketedAtlasAllocator>,
    quarantined: bool,
}

impl<P> Page<P> {
    fn new(id: P) -> Self {
        let size = i32::try_from(PAGE_SIZE_PX).expect("BUG: page size exceeds i32");
        Self {
            id,
            alloc: Some(etagere::BucketedAtlasAllocator::new(etagere::size2(
                size, size,
            ))),
            quarantined: false,
        }
    }

    fn allocate(&mut self, size: etagere::Size) -> Option<etagere::Allocation> {
        if self.quarantined {
            return None;
        }
        self.alloc.as_mut()?.allocate(size)
    }

    fn deallocate(&mut self, id: etagere::AllocId) {
        self.alloc
            .as_mut()
            .expect("BUG: page released its allocator while an entry still held a rect")
            .deallocate(id);
    }
}

/// Lifetime tallies. `u64` because they are monotonic over the renderer's
/// lifetime and the target is 32-bit.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub page_create_failures: u64,
    pub upload_transient_failures: u64,
}

/// Where a glyph sits in the atlas, ready to be emitted as a textured quad.
/// The UVs sample the inner `width × height` only, never the 1 px border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad<P> {
    pub page: P,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub placement: RasterPlacement,
}

/// `Missing` is retryable (the glyph has no coverage, or the backend faulted
/// transiently); `Dropped` is not (the glyph can never fit a page).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphLookup<P> {
    Resident(GlyphQuad<P>),
    Missing,
    Dropped,
}

#[expect(clippy::cast_precision_loss, reason = "bounded by PAGE_SIZE_PX")]
fn uv(px: usize) -> f32 {
    px as f32 / PAGE_SIZE_PX as f32
}

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 11 (BDK-696)"))]
pub struct GlyphCache<P: Copy + Eq + core::fmt::Debug> {
    pages: Vec<Page<P>>,
    map: hashbrown::HashMap<GlyphKey, u32>,
    slab: EntrySlab,
    lru: LruQueue,
    generation: Generation,
    counters: Counters,
}

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 11 (BDK-696)"))]
impl<P: Copy + Eq + core::fmt::Debug> GlyphCache<P> {
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            map: hashbrown::HashMap::with_capacity(MAX_RESIDENT_ENTRIES),
            slab: EntrySlab::with_capacity(MAX_RESIDENT_ENTRIES),
            lru: LruQueue::new(),
            generation: 0,
            counters: Counters::default(),
        }
    }

    pub fn counters(&self) -> &Counters {
        &self.counters
    }

    #[expect(dead_code, reason = "consumed in Task 4 (BDK-696)")]
    pub fn end_frame(&mut self) {
        self.generation += 1;
    }

    /// `rasterize` receives the **normalized** key,
    /// so lookup and rasterization can never disagree on subpixel bins.
    pub fn get_or_insert(
        &mut self,
        backend: &mut impl PageBackend<PageId = P>,
        key: cosmic_text::CacheKey,
        rasterize: impl FnOnce(GlyphKey) -> Option<RasterGlyph>,
    ) -> GlyphLookup<P> {
        let key = GlyphKey::normalize(key);

        if let Some(&slot) = self.map.get(&key) {
            self.slab.get_mut(slot).last_used = self.generation;
            self.lru.promote(&mut self.slab, slot);
            return GlyphLookup::Resident(self.quad(slot));
        }

        let Some(raster) = rasterize(key) else {
            return GlyphLookup::Missing;
        };

        let padded_width = raster.width + 2;
        let padded_height = raster.height + 2;
        if padded_width > PAGE_SIZE_PX || padded_height > PAGE_SIZE_PX {
            return GlyphLookup::Dropped;
        }

        let Some((page, allocation)) = self.allocate(backend, padded_width, padded_height) else {
            return GlyphLookup::Missing;
        };
        // The allocation may span the whole shelf height; only its min corner
        // is ours, and the content rect is what we upload.
        let content_x = usize::try_from(allocation.rectangle.min.x)
            .expect("BUG: allocation origin outside the page");
        let content_y = usize::try_from(allocation.rectangle.min.y)
            .expect("BUG: allocation origin outside the page");

        let mut pixels = vec![0_u8; padded_width * padded_height];
        for row in 0..raster.height {
            let source = row * raster.width;
            let target = (row + 1) * padded_width + 1;
            pixels[target..target + raster.width]
                .copy_from_slice(&raster.coverage[source..source + raster.width]);
        }

        let Some(slot) = self.slab.alloc(Entry {
            key,
            page,
            alloc_id: allocation.id,
            content_x,
            content_y,
            placement: RasterPlacement {
                left: raster.left,
                top: raster.top,
                width: raster.width,
                height: raster.height,
            },
            last_used: self.generation,
            prev: NO_LINK,
            next: NO_LINK,
        }) else {
            self.pages[page].deallocate(allocation.id);
            return GlyphLookup::Missing;
        };

        if backend
            .upload(
                self.pages[page].id,
                content_x,
                content_y,
                padded_width,
                padded_height,
                &pixels,
            )
            .is_err()
        {
            self.slab.free(slot);
            self.pages[page].deallocate(allocation.id);
            self.counters.upload_transient_failures += 1;
            return GlyphLookup::Missing;
        }

        self.map.insert(key, slot);
        self.lru.push_hot(&mut self.slab, slot);
        GlyphLookup::Resident(self.quad(slot))
    }

    fn allocate(
        &mut self,
        backend: &mut impl PageBackend<PageId = P>,
        padded_width: usize,
        padded_height: usize,
    ) -> Option<(usize, etagere::Allocation)> {
        let size = etagere::size2(
            i32::try_from(padded_width).expect("BUG: padded dim exceeds i32"),
            i32::try_from(padded_height).expect("BUG: padded dim exceeds i32"),
        );

        if let Some(placed) = self.allocate_on_existing(size) {
            return Some(placed);
        }
        if self.pages.len() >= MAX_NORMAL_PAGES {
            return None;
        }

        let Ok(id) = backend.create_page(PAGE_SIZE_PX) else {
            self.counters.page_create_failures += 1;
            return None;
        };
        self.pages.push(Page::new(id));
        Some(
            self.allocate_on_existing(size)
                .expect("BUG: a fresh page refused a rect that fits the page"),
        )
    }

    fn allocate_on_existing(
        &mut self,
        size: etagere::Size,
    ) -> Option<(usize, etagere::Allocation)> {
        self.pages
            .iter_mut()
            .enumerate()
            .find_map(|(index, page)| page.allocate(size).map(|placed| (index, placed)))
    }

    fn quad(&self, slot: u32) -> GlyphQuad<P> {
        let entry = self.slab.get(slot);
        let u0 = uv(entry.content_x + 1);
        let v0 = uv(entry.content_y + 1);
        GlyphQuad {
            page: self.pages[entry.page].id,
            u0,
            v0,
            u1: u0 + uv(entry.placement.width),
            v1: v0 + uv(entry.placement.height),
            placement: entry.placement,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{PageBackend, PageCreateFailed, PageFaultKind};

    pub(crate) struct MockPage {
        pub size_px: usize,
        pub pixels: Vec<u8>,
    }

    /// In-memory stand-in for the femtovg pages, recording every call so tests
    /// can assert on upload rects and the lifetime page budget, not just texels.
    #[derive(Default)]
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

    /// Cache and slab tests care only that keys stay distinct, so the key is
    /// synthesized rather than shaped — shaping a font system per entry would
    /// dominate the eight-thousand-entry runs.
    fn test_cache_key(index: usize) -> cosmic_text::CacheKey {
        cosmic_text::CacheKey {
            font_id: cosmic_text::fontdb::ID::dummy(),
            glyph_id: u16::try_from(index).expect("BUG: test index outside glyph id range"),
            font_size_bits: 17.0_f32.to_bits(),
            x_bin: SubpixelBin::Zero,
            y_bin: SubpixelBin::Zero,
            font_weight: cosmic_text::fontdb::Weight::NORMAL,
            flags: cosmic_text::CacheKeyFlags::empty(),
        }
    }

    fn test_key(index: usize) -> GlyphKey {
        GlyphKey(test_cache_key(index))
    }

    /// Every field varies with the index
    /// so an entry that came back from a reused slot
    /// can be told apart from the one it replaced.
    fn test_entry(index: usize) -> Entry {
        let index_u32 = u32::try_from(index).expect("BUG: test index outside slab index range");
        Entry {
            key: test_key(index),
            page: index,
            alloc_id: etagere::AllocId::deserialize(index_u32),
            content_x: index,
            content_y: index + 1,
            placement: RasterPlacement {
                left: -1,
                top: 2,
                width: index + 3,
                height: index + 4,
            },
            last_used: Generation::from(index_u32),
            prev: NO_LINK,
            next: NO_LINK,
        }
    }

    fn push_entries(slab: &mut EntrySlab, lru: &mut LruQueue, count: usize) -> Vec<u32> {
        (0..count)
            .map(|i| {
                let idx = slab.alloc(test_entry(i)).expect("BUG: slab full in test");
                lru.push_hot(slab, idx);
                idx
            })
            .collect()
    }

    fn hot_to_cold(slab: &EntrySlab, lru: &LruQueue) -> Vec<u32> {
        let mut order = Vec::new();
        let mut cursor = lru.head;
        while cursor != NO_LINK {
            order.push(cursor);
            cursor = slab.get(cursor).next;
        }
        order
    }

    #[test]
    fn promotion_makes_the_next_entry_the_coldest() {
        let mut slab = EntrySlab::with_capacity(MAX_RESIDENT_ENTRIES);
        let mut lru = LruQueue::new();
        let idx = push_entries(&mut slab, &mut lru, 3);

        assert_eq!(lru.coldest(), Some(idx[0]));
        lru.promote(&mut slab, idx[0]);
        assert_eq!(lru.coldest(), Some(idx[1]));
        assert_eq!(hot_to_cold(&slab, &lru), vec![idx[0], idx[2], idx[1]]);
    }

    /// Promoting what is already hottest routes through the same unlink that
    /// empties a one-entry queue, so both ends have to be re-established.
    #[test]
    fn promoting_the_hottest_entry_keeps_the_queue_intact() {
        let mut slab = EntrySlab::with_capacity(MAX_RESIDENT_ENTRIES);
        let mut lru = LruQueue::new();
        let solo = push_entries(&mut slab, &mut lru, 1);

        lru.promote(&mut slab, solo[0]);
        assert_eq!(hot_to_cold(&slab, &lru), solo);
        assert_eq!(lru.coldest(), Some(solo[0]));

        let rest = push_entries(&mut slab, &mut lru, 2);
        lru.promote(&mut slab, rest[1]);
        assert_eq!(hot_to_cold(&slab, &lru), vec![rest[1], rest[0], solo[0]]);
        assert_eq!(lru.coldest(), Some(solo[0]));
    }

    #[test]
    fn unlinking_from_any_position_keeps_the_rest_ordered() {
        let mut slab = EntrySlab::with_capacity(MAX_RESIDENT_ENTRIES);
        let mut lru = LruQueue::new();
        let idx = push_entries(&mut slab, &mut lru, 3);

        lru.unlink(&mut slab, idx[1]);
        assert_eq!(hot_to_cold(&slab, &lru), vec![idx[2], idx[0]]);

        lru.unlink(&mut slab, idx[2]);
        assert_eq!(hot_to_cold(&slab, &lru), vec![idx[0]]);
        assert_eq!(lru.coldest(), Some(idx[0]));

        lru.unlink(&mut slab, idx[0]);
        assert!(hot_to_cold(&slab, &lru).is_empty());
        assert_eq!(lru.coldest(), None);
    }

    #[test]
    fn an_empty_queue_has_no_coldest_entry() {
        assert_eq!(LruQueue::new().coldest(), None);
    }

    #[test]
    fn millions_of_promotions_never_grow_the_queue() {
        let mut slab = EntrySlab::with_capacity(MAX_RESIDENT_ENTRIES);
        let mut lru = LruQueue::new();
        let idx: Vec<u32> = (0..64)
            .map(|i| {
                let e = slab.alloc(test_entry(i)).expect("BUG: slab full in test");
                lru.push_hot(&mut slab, e);
                e
            })
            .collect();
        let cap_before = slab.capacity();
        for i in 0..2_000_000_usize {
            lru.promote(&mut slab, idx[i % 64]);
        }
        assert_eq!(slab.capacity(), cap_before);
        assert_eq!(slab.len(), 64);
    }

    #[test]
    fn a_full_slab_refuses_to_grow_and_reuses_freed_slots() {
        let mut slab = EntrySlab::with_capacity(MAX_RESIDENT_ENTRIES);
        for i in 0..MAX_RESIDENT_ENTRIES {
            assert!(slab.alloc(test_entry(i)).is_some(), "BUG: slab full early");
        }
        let cap_before = slab.capacity();

        assert!(slab.alloc(test_entry(0)).is_none());
        assert_eq!(slab.capacity(), cap_before);
        assert_eq!(slab.len(), MAX_RESIDENT_ENTRIES);

        let victim = 7;
        slab.free(victim);
        assert_eq!(slab.len(), MAX_RESIDENT_ENTRIES - 1);

        let reused = slab
            .alloc(test_entry(1234))
            .expect("BUG: freed slot was not handed back");
        assert_eq!(reused, victim);
        assert_eq!(slab.get(reused), &test_entry(1234));
        assert_eq!(slab.capacity(), cap_before);
        assert_eq!(slab.len(), MAX_RESIDENT_ENTRIES);
    }

    fn solid_glyph(width: usize, height: usize) -> RasterGlyph {
        RasterGlyph {
            width,
            height,
            left: 1,
            top: -2,
            coverage: vec![0xFF; width * height],
        }
    }

    /// Derived through `f32::from` rather than the cache's own `uv`,
    /// so the expectation is arithmetic the implementation does not supply.
    fn uv_of(px: usize) -> f32 {
        f32::from(u16::try_from(px).expect("BUG: test coordinate outside page range")) / 512.0
    }

    fn resident<P: Copy + Eq + core::fmt::Debug>(lookup: GlyphLookup<P>) -> GlyphQuad<P> {
        match lookup {
            GlyphLookup::Resident(quad) => quad,
            GlyphLookup::Missing => panic!("BUG: expected a resident glyph, got Missing"),
            GlyphLookup::Dropped => panic!("BUG: expected a resident glyph, got Dropped"),
        }
    }

    /// The odd 3 px width is deliberate: its padded row stride of 5 is
    /// accepted by no GL unpack alignment above 1.
    #[test]
    fn miss_uploads_content_rect_with_zeroed_border() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let key = test_cache_key(1);

        let quad = resident(cache.get_or_insert(&mut backend, key, |normalized| {
            assert_eq!(normalized, GlyphKey::normalize(key));
            Some(solid_glyph(3, 5))
        }));

        let &[(page, x, y, width, height)] = backend.uploads.as_slice() else {
            panic!(
                "BUG: expected exactly one upload, got {:?}",
                backend.uploads
            );
        };
        assert_eq!((width, height), (5, 7));
        assert_eq!(
            quad,
            GlyphQuad {
                page,
                u0: uv_of(x + 1),
                v0: uv_of(y + 1),
                u1: uv_of(x + 4),
                v1: uv_of(y + 6),
                placement: RasterPlacement {
                    left: 1,
                    top: -2,
                    width: 3,
                    height: 5,
                },
            }
        );

        let pixels = &backend.pages[page].pixels;
        for row in 0..height {
            for col in 0..width {
                let on_border = row == 0 || row == height - 1 || col == 0 || col == width - 1;
                assert_eq!(
                    pixels[(y + row) * PAGE_SIZE_PX + x + col],
                    if on_border { 0x00 } else { 0xFF },
                    "content texel ({col}, {row})"
                );
            }
        }
    }

    #[test]
    fn hit_returns_same_quad_without_rasterizing() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let key = test_cache_key(1);

        let first = resident(cache.get_or_insert(&mut backend, key, |_| Some(solid_glyph(3, 5))));
        let second = resident(cache.get_or_insert(&mut backend, key, |_| {
            panic!("BUG: a hit must not rasterize")
        }));

        assert_eq!(second, first);
        assert_eq!(backend.uploads.len(), 1);
        assert_eq!(backend.pages_created, 1);
    }

    #[test]
    fn upload_failure_rolls_back_the_allocation() {
        let key = test_cache_key(1);
        let mut control_backend = test_support::MockBackend::default();
        let mut control = GlyphCache::new();
        let _ = control.get_or_insert(&mut control_backend, key, |_| Some(solid_glyph(3, 5)));
        let clean_rect = *control_backend
            .uploads
            .first()
            .expect("BUG: the control insertion uploaded nothing");

        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        backend.fail_next_upload = Some(PageFaultKind::Transient);

        assert_eq!(
            cache.get_or_insert(&mut backend, key, |_| Some(solid_glyph(3, 5))),
            GlyphLookup::Missing
        );
        assert_eq!(cache.counters().upload_transient_failures, 1);
        assert!(!cache.map.contains_key(&GlyphKey::normalize(key)));
        assert_eq!(cache.slab.len(), 0);
        assert!(backend.uploads.is_empty());

        let _ = resident(cache.get_or_insert(&mut backend, key, |_| Some(solid_glyph(3, 5))));
        assert_eq!(backend.uploads, vec![clean_rect]);
    }

    /// The sizes are chosen against etagere's shelf rule —
    /// a shelf serves a request
    /// only while its surplus height is at most the request's own height —
    /// so the 16 px shelf the first rect opens still takes the second,
    /// while anything shorter would open a shelf of its own.
    ///
    /// Asserting the recorded upload rects rather than page texels is
    /// deliberate: the page starts zeroed, so "texels outside stayed 0" would
    /// also hold if the whole zero-filled shelf surplus had been uploaded.
    #[test]
    fn two_glyph_sizes_share_a_shelf_without_overlap() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();

        let _ = resident(
            cache.get_or_insert(&mut backend, test_cache_key(1), |_| Some(solid_glyph(8, 8))),
        );
        let _ = resident(
            cache.get_or_insert(&mut backend, test_cache_key(2), |_| Some(solid_glyph(6, 6))),
        );

        let &[
            (first_page, first_x, first_y, 10, 10),
            (second_page, second_x, second_y, 8, 8),
        ] = backend.uploads.as_slice()
        else {
            panic!(
                "BUG: expected one 10x10 and one 8x8 content rect, got {:?}",
                backend.uploads
            );
        };
        assert_eq!(first_page, second_page);
        assert_eq!(first_y, second_y, "both rects must sit on one shelf");
        assert!(
            first_x + 10 <= second_x || second_x + 8 <= first_x,
            "shelf mates overlap: {first_x}..{} and {second_x}..{}",
            first_x + 10,
            second_x + 8
        );
    }

    #[test]
    fn oversized_raster_is_dropped_without_page_creation() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let key = test_cache_key(1);

        assert_eq!(
            cache.get_or_insert(&mut backend, key, |_| Some(solid_glyph(600, 20))),
            GlyphLookup::Dropped
        );
        assert_eq!(backend.pages_created, 0);
        assert!(backend.uploads.is_empty());
        assert!(!cache.map.contains_key(&GlyphKey::normalize(key)));
    }
}
