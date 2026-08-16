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

use core::num::NonZeroU32;

pub const PAGE_SIZE_PX: usize = 512;
pub const MAX_NORMAL_PAGES: usize = 10;
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
pub const MAX_RESIDENT_ENTRIES: usize = 8192;
pub const NEGATIVE_CACHE_CAP: usize = 256;
pub const SCRATCH_MAP_CAP: usize = 1024;
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
pub const MAX_EVICTIONS_PER_MISS: usize = 64;
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
pub const FULL_RETRY_INTERVAL: usize = 8;
const PRESSURE_LOG_INTERVAL_GENERATIONS: Generation = 8;

/// u64, not usize: generations must never wrap (eviction safety compares
/// them) and 32-bit usize would wrap on-device within device lifetime.
pub type Generation = u64;

/// Cache identity: cosmic-text's key with subpixel bins forced to Zero.
/// Subpixel variants are deliberately not cached.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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

    #[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
    pub fn inner(&self) -> cosmic_text::CacheKey {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
pub enum PageFaultKind {
    Invariant,
    Transient,
}

/// Page creation failures are transient by definition:
/// an Err carries no PageId, so there is nothing to quarantine —
/// the caller skips this frame, counts, and retries later.
/// Only `upload` can fault against a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
pub struct PageCreateFailed;

/// GL abstraction so the cache is unit-testable without a context;
/// the production impl wraps `Canvas<OpenGl>`.
/// Dimensions are `usize` end-to-end:
/// femtovg's `create_image_empty`/`update_image` take `usize`,
/// so matching it here means no conversions at the only real boundary.
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
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

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
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

/// A compact slab index. Encoding the index plus one gives [`Option`] a
/// four-byte niche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotId(NonZeroU32);

impl SlotId {
    fn from_index(index: usize) -> Self {
        let encoded = u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(NonZeroU32::new)
            .expect("BUG: slab index outside the slab index range");
        Self(encoded)
    }

    fn index(self) -> usize {
        usize::try_from(self.0.get() - 1).expect("BUG: slab index exceeds usize")
    }
}

pub type Link = Option<SlotId>;

/// One resident glyph: where it lives in the atlas, how to draw it,
/// and its place in the LRU order.
/// The fixed-width links keep the entry within the 96-byte metadata budget
/// on 64-bit hosts; they match `usize` on the 32-bit target.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: GlyphKey,
    pub page: usize,
    pub alloc_id: etagere::AllocId,
    pub content_x: usize,
    pub content_y: usize,
    pub placement: RasterPlacement,
    pub last_used: Generation,
    pub prev: Link,
    pub next: Link,
}

/// Fixed-capacity storage for resident entries.
/// Slots are never released to the allocator: a freed slot joins a chain
/// threaded through `next` and is handed back by the next `alloc`.
#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
pub struct EntrySlab {
    entries: Vec<Entry>,
    free_head: Link,
    live: usize,
}

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
impl EntrySlab {
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(
            u32::try_from(capacity).is_ok(),
            "BUG: slab capacity outside the slab index range"
        );
        Self {
            entries: Vec::with_capacity(capacity),
            free_head: None,
            live: 0,
        }
    }

    pub fn alloc(&mut self, entry: Entry) -> Option<SlotId> {
        let capacity = self.entries.capacity();
        let index = if let Some(index) = self.free_head {
            let slot = self.get_mut(index);
            let next_free = slot.next;
            *slot = entry;
            self.free_head = next_free;
            index
        } else {
            if self.entries.len() == capacity {
                return None;
            }
            self.entries.push(entry);
            SlotId::from_index(self.entries.len() - 1)
        };

        debug_assert_eq!(
            self.entries.capacity(),
            capacity,
            "BUG: entry slab grew past its preallocated capacity"
        );
        self.live += 1;
        Some(index)
    }

    pub fn free(&mut self, index: SlotId) {
        let free_head = self.free_head;
        self.get_mut(index).next = free_head;
        self.free_head = Some(index);
        self.live -= 1;
    }

    pub fn get(&self, index: SlotId) -> &Entry {
        self.entries
            .get(index.index())
            .expect("BUG: slab index refers to no slot")
    }

    pub fn get_mut(&mut self, index: SlotId) -> &mut Entry {
        self.entries
            .get_mut(index.index())
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
    head: Link,
    tail: Link,
}

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 3 (BDK-696)"))]
impl LruQueue {
    pub fn new() -> Self {
        Self {
            head: None,
            tail: None,
        }
    }

    pub fn push_hot(&mut self, slab: &mut EntrySlab, index: SlotId) {
        let old_head = self.head;
        let entry = slab.get_mut(index);
        entry.prev = None;
        entry.next = old_head;

        if let Some(old_head) = old_head {
            slab.get_mut(old_head).prev = Some(index);
        } else {
            self.tail = Some(index);
        }
        self.head = Some(index);
    }

    pub fn unlink(&mut self, slab: &mut EntrySlab, index: SlotId) {
        let entry = slab.get_mut(index);
        let (prev, next) = (entry.prev, entry.next);
        debug_assert!(
            (prev.is_some() || self.head == Some(index))
                && (next.is_some() || self.tail == Some(index)),
            "BUG: unlinking an entry the queue does not hold"
        );
        entry.prev = None;
        entry.next = None;

        if let Some(prev) = prev {
            slab.get_mut(prev).next = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            slab.get_mut(next).prev = prev;
        } else {
            self.tail = prev;
        }
    }

    pub fn promote(&mut self, slab: &mut EntrySlab, index: SlotId) {
        self.unlink(slab, index);
        self.push_hot(slab, index);
    }

    pub fn coldest(&self) -> Option<SlotId> {
        self.tail
    }
}

/// Keys whose rasterization cannot produce an atlas entry, kept so they cannot
/// re-enter the rasterizer every frame. Storage is inline arrays — an animated
/// font size mints a fresh exact-size key per frame, so a growable map here
/// would just relocate the unbounded growth into the heap.
///
/// Occupied slots are exactly `0..len`;
/// eviction reuses the coldest slot in place,
/// so the arrays are written once and never resized.
struct NegativeCache {
    keys: [Option<GlyphKey>; NEGATIVE_CACHE_CAP],
    reasons: [NegativeReason; NEGATIVE_CACHE_CAP],
    prev: [Link; NEGATIVE_CACHE_CAP],
    next: [Link; NEGATIVE_CACHE_CAP],
    head: Link,
    tail: Link,
    len: usize,
}

#[derive(Clone, Copy)]
enum NegativeReason {
    Missing,
    Oversized,
}

impl NegativeReason {
    fn into_lookup<P>(self) -> GlyphLookup<P> {
        match self {
            Self::Missing => GlyphLookup::Missing,
            Self::Oversized => GlyphLookup::Oversized,
        }
    }
}

impl NegativeCache {
    fn new() -> Self {
        Self {
            keys: [None; NEGATIVE_CACHE_CAP],
            reasons: [NegativeReason::Missing; NEGATIVE_CACHE_CAP],
            prev: [None; NEGATIVE_CACHE_CAP],
            next: [None; NEGATIVE_CACHE_CAP],
            head: None,
            tail: None,
            len: 0,
        }
    }

    /// Promotes what it finds: a glyph still being drawn every frame must not
    /// age out under the sizes that arrived after it.
    fn get(&mut self, key: &GlyphKey) -> Option<NegativeReason> {
        let slot = self.slot_of(key)?;
        self.unlink(slot);
        self.push_hot(slot);
        Some(self.reasons[slot.index()])
    }

    fn insert_absent(&mut self, key: GlyphKey, reason: NegativeReason) {
        debug_assert!(
            self.slot_of(&key).is_none(),
            "BUG: inserting a key already present in the negative cache"
        );

        let slot = if self.len == NEGATIVE_CACHE_CAP {
            let coldest = self
                .tail
                .expect("BUG: full negative cache has no coldest slot");
            self.unlink(coldest);
            coldest
        } else {
            let slot = SlotId::from_index(self.len);
            self.len += 1;
            slot
        };
        self.keys[slot.index()] = Some(key);
        self.reasons[slot.index()] = reason;
        self.push_hot(slot);
    }

    fn slot_of(&self, key: &GlyphKey) -> Option<SlotId> {
        let found = self.keys[..self.len]
            .iter()
            .position(|slot| slot.as_ref() == Some(key))?;
        Some(SlotId::from_index(found))
    }

    fn push_hot(&mut self, slot: SlotId) {
        let old_head = self.head;
        self.prev[slot.index()] = None;
        self.next[slot.index()] = old_head;

        if let Some(old_head) = old_head {
            self.prev[old_head.index()] = Some(slot);
        } else {
            self.tail = Some(slot);
        }
        self.head = Some(slot);
    }

    fn unlink(&mut self, slot: SlotId) {
        let (prev, next) = (self.prev[slot.index()], self.next[slot.index()]);
        debug_assert!(
            (prev.is_some() || self.head == Some(slot))
                && (next.is_some() || self.tail == Some(slot)),
            "BUG: unlinking a key the negative cache does not hold"
        );

        if let Some(prev) = prev {
            self.next[prev.index()] = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            self.prev[next.index()] = prev;
        } else {
            self.tail = prev;
        }
    }
}

/// One atlas page: the backend's image plus the allocator that packs it.
/// `alloc` is an `Option` so quarantine can drop the allocator's metadata
/// while the page struct and its image stay retained.
struct Page<P> {
    id: P,
    alloc: Option<etagere::BucketedAtlasAllocator>,
    /// Every rect goes through the two methods below, so a leak is exactly
    /// a gap between these: an eviction path that never handed its rect back.
    #[cfg(test)]
    allocs: usize,
    #[cfg(test)]
    deallocs: usize,
}

impl<P> Page<P> {
    fn new(id: P) -> Self {
        let size = i32::try_from(PAGE_SIZE_PX).expect("BUG: page size exceeds i32");
        Self {
            id,
            alloc: Some(etagere::BucketedAtlasAllocator::new(etagere::size2(
                size, size,
            ))),
            #[cfg(test)]
            allocs: 0,
            #[cfg(test)]
            deallocs: 0,
        }
    }

    fn allocate(&mut self, size: etagere::Size) -> Option<etagere::Allocation> {
        let placed = self.alloc.as_mut()?.allocate(size)?;
        #[cfg(test)]
        {
            self.allocs += 1;
        }
        Some(placed)
    }

    fn deallocate(&mut self, id: etagere::AllocId) {
        self.alloc
            .as_mut()
            .expect("BUG: page released its allocator while an entry still held a rect")
            .deallocate(id);
        #[cfg(test)]
        {
            self.deallocs += 1;
        }
    }
}

/// A rect held on a page before any entry owns it.
/// It stays apart from [`Entry`] because the entry cap can force the rect
/// back, to be re-taken on the far side of an eviction.
#[derive(Clone, Copy)]
struct Reserved {
    page: usize,
    alloc_id: etagere::AllocId,
    content_x: usize,
    content_y: usize,
}

impl Reserved {
    /// The allocation may span the whole shelf height; only its min corner
    /// is ours, and the content rect is what gets uploaded.
    fn new(page: usize, allocation: etagere::Allocation) -> Self {
        Self {
            page,
            alloc_id: allocation.id,
            content_x: usize::try_from(allocation.rectangle.min.x)
                .expect("BUG: allocation origin outside the page"),
            content_y: usize::try_from(allocation.rectangle.min.y)
                .expect("BUG: allocation origin outside the page"),
        }
    }
}

/// A rasterized glyph ready for whichever page will take it.
/// The bitmap carries a 1 px transparent border, so linear sampling
/// cannot pull a neighbour's texels into the glyph.
struct Pending {
    placement: RasterPlacement,
    padded_width: usize,
    padded_height: usize,
    pixels: Vec<u8>,
}

impl Pending {
    fn new(raster: &RasterGlyph) -> Self {
        let padded_width = raster.width + 2;
        let padded_height = raster.height + 2;
        let mut pixels = vec![0_u8; padded_width * padded_height];
        for row in 0..raster.height {
            let source = row * raster.width;
            let target = (row + 1) * padded_width + 1;
            pixels[target..target + raster.width]
                .copy_from_slice(&raster.coverage[source..source + raster.width]);
        }

        Self {
            placement: RasterPlacement {
                left: raster.left,
                top: raster.top,
                width: raster.width,
                height: raster.height,
            },
            padded_width,
            padded_height,
            pixels,
        }
    }

    fn size(&self) -> etagere::Size {
        etagere::size2(
            i32::try_from(self.padded_width).expect("BUG: padded dim exceeds i32"),
            i32::try_from(self.padded_height).expect("BUG: padded dim exceeds i32"),
        )
    }
}

/// A glyph packed into the scratch page for the current frame only.
/// It holds no atlas allocation and no LRU links:
/// the whole map goes at `end_frame`.
#[derive(Clone, Copy)]
struct ScratchEntry {
    content_x: usize,
    content_y: usize,
    placement: RasterPlacement,
}

/// Lifetime tallies. `u64` because they are monotonic over the renderer's
/// lifetime and the target is 32-bit.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub max_evictions_per_miss: u64,
    pub scratch_uses: u64,
    pub glyphs_dropped: u64,
    pub glyphs_oversized: u64,
    pub cache_invariant_failures: u64,
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

/// `Missing` is retryable, `Oversized` can never fit an atlas page, and
/// `Dropped` means the cache could not serve the glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphLookup<P> {
    Resident(GlyphQuad<P>),
    Missing,
    Oversized,
    Dropped,
}

#[expect(clippy::cast_precision_loss, reason = "bounded by PAGE_SIZE_PX")]
fn uv(px: usize) -> f32 {
    px as f32 / PAGE_SIZE_PX as f32
}

/// The 1 px border the upload wrote around the content is never sampled.
fn quad_at<P>(
    page: P,
    content_x: usize,
    content_y: usize,
    placement: RasterPlacement,
) -> GlyphQuad<P> {
    let u0 = uv(content_x + 1);
    let v0 = uv(content_y + 1);
    GlyphQuad {
        page,
        u0,
        v0,
        u1: u0 + uv(placement.width),
        v1: v0 + uv(placement.height),
        placement,
    }
}

/// Why the normal pages could not take a rect.
/// Only the first reaches the scratch page: a backend that just refused
/// a page will refuse the scratch one too, and be counted twice for it.
enum AllocFailure {
    Unservable,
    PageCreate,
}

/// Which page an invariant fault named. The two are quarantined differently:
/// a normal page is retired from a set that shrinks, while the scratch page
/// is the whole scratch service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Faulted {
    Normal(usize),
    Scratch,
}

/// The spec's metadata table prices these three by size:
/// a layout change that made one bigger would blow the ceiling silently.
const _: () = assert!(size_of::<Entry>() <= 96);
const _: () = assert!(size_of::<GlyphKey>() <= 28);
const _: () = assert!(size_of::<NegativeCache>() <= 10 * 1024);
const _: () = assert!(size_of::<Link>() == size_of::<u32>());

/// etagere's shelves and buckets are private, so the spec bounds them
/// from page geometry — 512 px of height at the 8 px minimum shelf,
/// plus bucket splits — rather than measuring them.
const ETAGERE_NORMAL_PAGE_ESTIMATE: usize = 8 * 1024;

/// The scratch allocator tracks each allocation individually.
/// Its 64-byte per-entry budget also covers shelf metadata and vector growth slack.
const ETAGERE_SCRATCH_ESTIMATE: usize = 64 * SCRATCH_MAP_CAP;

/// The owned swash `ScaleContext` at its high-water — per-face state,
/// hinting cache, and outline scratch for the largest glyph we rasterize.
const SWASH_CONTEXT_ESTIMATE: usize = 512 * 1024;

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
pub struct GlyphCache<P: Copy + Eq + core::fmt::Debug> {
    pages: Vec<Page<P>>,
    map: hashbrown::HashMap<GlyphKey, SlotId>,
    slab: EntrySlab,
    lru: LruQueue,
    negative: NegativeCache,
    /// Created on first need, then kept for the renderer's lifetime.
    /// A per-frame image would churn a GL texture that `glDeleteTextures`
    /// need not have freed, leaving several behind a backlogged driver.
    scratch: Option<P>,
    scratch_packer: etagere::AtlasAllocator,
    scratch_map: hashbrown::HashMap<GlyphKey, ScratchEntry>,
    /// Both latches hold for the renderer's lifetime: an invariant fault is
    /// deterministic, so any state that expired would just retry it.
    scratch_latched_off: bool,
    failed_closed: bool,
    generation: Generation,
    counters: Counters,
    counters_at_last_log: Counters,
    next_log_generation: Generation,
}

#[cfg_attr(not(test), expect(dead_code, reason = "consumed in Task 12 (BDK-696)"))]
impl<P: Copy + Eq + core::fmt::Debug> GlyphCache<P> {
    pub fn new() -> Self {
        Self {
            pages: Vec::with_capacity(MAX_NORMAL_PAGES),
            map: hashbrown::HashMap::with_capacity(MAX_RESIDENT_ENTRIES),
            slab: EntrySlab::with_capacity(MAX_RESIDENT_ENTRIES),
            lru: LruQueue::new(),
            negative: NegativeCache::new(),
            scratch: None,
            scratch_packer: etagere::AtlasAllocator::new(etagere::size2(
                i32::try_from(PAGE_SIZE_PX).expect("BUG: page size exceeds i32"),
                i32::try_from(PAGE_SIZE_PX).expect("BUG: page size exceeds i32"),
            )),
            scratch_map: hashbrown::HashMap::with_capacity(SCRATCH_MAP_CAP),
            scratch_latched_off: false,
            failed_closed: false,
            generation: 0,
            counters: Counters::default(),
            counters_at_last_log: Counters::default(),
            next_log_generation: 0,
        }
    }

    pub fn counters(&self) -> &Counters {
        &self.counters
    }

    /// The spec's metadata ceiling, measured rather than assumed:
    /// what this cache allocates itself at its real capacity, plus estimates
    /// for the two dependencies that expose no capacity of their own.
    pub fn metadata_capacity_bytes(&self) -> usize {
        self.app_owned_metadata_bytes()
            + self.pages.len() * ETAGERE_NORMAL_PAGE_ESTIMATE
            + ETAGERE_SCRATCH_ESTIMATE
            + SWASH_CONTEXT_ESTIMATE
    }

    /// Gray8, so a page texture is exactly its pixel count.
    pub fn resident_atlas_bytes(&self) -> usize {
        self.page_textures() * PAGE_SIZE_PX * PAGE_SIZE_PX
    }

    /// Counted at real capacity, not live length:
    /// a length cap is not an allocation cap.
    /// The maps report their own allocation
    /// rather than having it reconstructed from `capacity()`,
    /// which is `len + growth_left` and so falls back toward the live count
    /// as tombstones drain the slack of a table that keeps its buckets.
    fn app_owned_metadata_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<Page<P>>()
            + self.slab.capacity() * size_of::<Entry>()
            + self.map.allocation_size()
            + size_of::<NegativeCache>()
            + self.scratch_map.allocation_size()
    }

    /// The scratch page counts: it is retained for the renderer's lifetime
    /// and costs a page texture like any other.
    fn page_textures(&self) -> usize {
        self.pages.len() + usize::from(self.scratch.is_some())
    }

    pub fn end_frame(&mut self) {
        self.log_pressure();
        self.generation += 1;
        self.scratch_packer.clear();
        self.scratch_map.clear();
    }

    /// Aggregate pressure across frames because logging every affected glyph
    /// would flood the log.
    fn log_pressure(&mut self) {
        let (base, now) = (self.counters_at_last_log, self.counters);
        let evictions = now.evictions - base.evictions;
        let scratch_uses = now.scratch_uses - base.scratch_uses;
        let glyphs_dropped = now.glyphs_dropped - base.glyphs_dropped;
        let glyphs_oversized = now.glyphs_oversized - base.glyphs_oversized;
        let page_create_failures = now.page_create_failures - base.page_create_failures;
        let upload_transient_failures =
            now.upload_transient_failures - base.upload_transient_failures;

        let tallies = [
            evictions,
            scratch_uses,
            glyphs_dropped,
            glyphs_oversized,
            page_create_failures,
            upload_transient_failures,
        ];
        if tallies == [0; 6] || self.generation < self.next_log_generation {
            return;
        }

        tracing::warn!(
            evictions,
            scratch_uses,
            glyphs_dropped,
            glyphs_oversized,
            page_create_failures,
            upload_transient_failures,
            "glyph cache under pressure"
        );
        self.counters_at_last_log = now;
        self.next_log_generation = self.generation + PRESSURE_LOG_INTERVAL_GENERATIONS;
    }

    /// `rasterize` receives the **normalized** key,
    /// so lookup and rasterization can never disagree on subpixel bins.
    pub fn get_or_insert(
        &mut self,
        backend: &mut impl PageBackend<PageId = P>,
        key: cosmic_text::CacheKey,
        rasterize: impl FnOnce(GlyphKey) -> Option<RasterGlyph>,
    ) -> GlyphLookup<P> {
        // Nothing is left to serve from and nothing may be created,
        // so the glyph goes before the rasterizer ever sees it.
        if self.failed_closed {
            self.counters.glyphs_dropped += 1;
            return GlyphLookup::Dropped;
        }

        let key = GlyphKey::normalize(key);

        if let Some(&slot) = self.map.get(&key) {
            self.counters.hits += 1;
            self.slab.get_mut(slot).last_used = self.generation;
            self.lru.promote(&mut self.slab, slot);
            return GlyphLookup::Resident(self.quad(slot));
        }
        self.counters.misses += 1;

        // Before the rasterizer, not after the normal pages refuse again:
        // a repeat of an unservable glyph would otherwise re-rasterize
        // and re-run the eviction sweep for every occurrence in the frame.
        if let Some(&entry) = self.scratch_map.get(&key) {
            self.counters.scratch_uses += 1;
            return GlyphLookup::Resident(quad_at(
                self.scratch
                    .expect("BUG: scratch entry without a scratch page"),
                entry.content_x,
                entry.content_y,
                entry.placement,
            ));
        }

        if let Some(reason) = self.negative.get(&key) {
            return reason.into_lookup();
        }
        let Some(raster) = rasterize(key) else {
            self.negative.insert_absent(key, NegativeReason::Missing);
            return GlyphLookup::Missing;
        };

        if raster.width > PAGE_SIZE_PX - 2 || raster.height > PAGE_SIZE_PX - 2 {
            self.counters.glyphs_oversized += 1;
            self.negative.insert_absent(key, NegativeReason::Oversized);
            return GlyphLookup::Oversized;
        }
        let pending = Pending::new(&raster);

        let needed = pending.size();
        let mut reserved = match self.allocate(backend, needed) {
            Ok(reserved) => reserved,
            Err(AllocFailure::PageCreate) => return GlyphLookup::Missing,
            Err(AllocFailure::Unservable) => {
                return self.serve_from_scratch(backend, key, &pending);
            }
        };

        let generation = self.generation;
        let placement = pending.placement;
        let entry = |reserved: Reserved| Entry {
            key,
            page: reserved.page,
            alloc_id: reserved.alloc_id,
            content_x: reserved.content_x,
            content_y: reserved.content_y,
            placement,
            last_used: generation,
            prev: None,
            next: None,
        };

        let slot = if let Some(slot) = self.slab.alloc(entry(reserved)) {
            slot
        } else {
            // Hand the rect back before evicting: carried through the loop
            // it gets abandoned as soon as eviction hands out another rect,
            // bleeding one per miss for as long as the cap holds.
            self.pages[reserved.page].deallocate(reserved.alloc_id);
            let Some(freed) = self.evict_until_placed(needed) else {
                return self.serve_from_scratch(backend, key, &pending);
            };
            reserved = freed;
            self.slab
                .alloc(entry(reserved))
                .expect("BUG: eviction placed a rect without freeing a slab slot")
        };

        if let Err(fault) = backend.upload(
            self.pages[reserved.page].id,
            reserved.content_x,
            reserved.content_y,
            pending.padded_width,
            pending.padded_height,
            &pending.pixels,
        ) {
            self.roll_back_insertion(reserved, slot);
            debug_assert!(
                fault != PageFaultKind::Invariant,
                "BUG: invariant fault uploading {}x{} at ({}, {}) to page {}",
                pending.padded_width,
                pending.padded_height,
                reserved.content_x,
                reserved.content_y,
                reserved.page
            );
            match fault {
                PageFaultKind::Transient => self.counters.upload_transient_failures += 1,
                PageFaultKind::Invariant => {
                    self.handle_invariant_fault(Faulted::Normal(reserved.page));
                }
            }
            return GlyphLookup::Missing;
        }

        self.map.insert(key, slot);
        self.lru.push_hot(&mut self.slab, slot);
        GlyphLookup::Resident(self.quad(slot))
    }

    fn allocate(
        &mut self,
        backend: &mut impl PageBackend<PageId = P>,
        needed: etagere::Size,
    ) -> Result<Reserved, AllocFailure> {
        if let Some(reserved) = self.allocate_on_existing(needed) {
            return Ok(reserved);
        }
        if self.pages.len() >= MAX_NORMAL_PAGES {
            return self
                .evict_until_placed(needed)
                .ok_or(AllocFailure::Unservable);
        }

        let Ok(id) = backend.create_page(PAGE_SIZE_PX) else {
            self.counters.page_create_failures += 1;
            return Err(AllocFailure::PageCreate);
        };
        self.pages.push(Page::new(id));
        Ok(self
            .allocate_on_existing(needed)
            .expect("BUG: a fresh page refused a rect that fits the page"))
    }

    /// The only path past the normal pages. The glyph goes to the shared
    /// scratch page for this frame alone, taking no slab slot and no LRU
    /// place: `end_frame` drops the whole map.
    fn serve_from_scratch(
        &mut self,
        backend: &mut impl PageBackend<PageId = P>,
        key: GlyphKey,
        pending: &Pending,
    ) -> GlyphLookup<P> {
        if self.scratch_latched_off {
            self.counters.glyphs_dropped += 1;
            return GlyphLookup::Dropped;
        }
        if self.scratch_map.len() == SCRATCH_MAP_CAP {
            self.counters.glyphs_dropped += 1;
            return GlyphLookup::Dropped;
        }

        let page = if let Some(page) = self.scratch {
            page
        } else {
            let Ok(page) = backend.create_page(PAGE_SIZE_PX) else {
                self.counters.page_create_failures += 1;
                return GlyphLookup::Missing;
            };
            self.scratch = Some(page);
            page
        };

        let Some(allocation) = self.scratch_packer.allocate(pending.size()) else {
            self.counters.glyphs_dropped += 1;
            return GlyphLookup::Dropped;
        };
        let content_x = usize::try_from(allocation.rectangle.min.x)
            .expect("BUG: allocation origin outside the scratch page");
        let content_y = usize::try_from(allocation.rectangle.min.y)
            .expect("BUG: allocation origin outside the scratch page");
        if let Err(fault) = backend.upload(
            page,
            content_x,
            content_y,
            pending.padded_width,
            pending.padded_height,
            &pending.pixels,
        ) {
            debug_assert!(
                fault != PageFaultKind::Invariant,
                "BUG: invariant fault uploading {}x{} at ({}, {}) to the scratch page",
                pending.padded_width,
                pending.padded_height,
                content_x,
                content_y
            );
            self.scratch_packer.deallocate(allocation.id);
            match fault {
                PageFaultKind::Transient => self.counters.upload_transient_failures += 1,
                PageFaultKind::Invariant => self.handle_invariant_fault(Faulted::Scratch),
            }
            return GlyphLookup::Missing;
        }
        self.scratch_map.insert(
            key,
            ScratchEntry {
                content_x,
                content_y,
                placement: pending.placement,
            },
        );
        self.counters.scratch_uses += 1;
        GlyphLookup::Resident(quad_at(page, content_x, content_y, pending.placement))
    }

    /// Undoes an insertion whose upload failed.
    /// The slot goes straight back to the slab:
    /// the entry was never linked into the LRU nor inserted into the map,
    /// and unlinking a detached node would rewrite the queue's ends
    /// from its absent links, cutting the live entries loose.
    fn roll_back_insertion(&mut self, reserved: Reserved, slot: SlotId) {
        self.slab.free(slot);
        self.pages[reserved.page].deallocate(reserved.alloc_id);
    }

    /// Retires what the fault implicated, for the renderer's lifetime.
    ///
    /// The image and the page slot are kept: deleting the texture would churn
    /// what the driver need not free, and a replacement would push the atlas
    /// past the page textures it is allowed to ever create. Only the packing
    /// metadata goes, and capacity shrinks by the page.
    fn handle_invariant_fault(&mut self, faulted: Faulted) {
        self.counters.cache_invariant_failures += 1;
        tracing::error!(
            ?faulted,
            cache_invariant_failures = self.counters.cache_invariant_failures,
            "glyph cache fault: quarantining the page for good"
        );

        match faulted {
            Faulted::Scratch => {
                self.scratch_latched_off = true;
                self.scratch_map.clear();
            }
            Faulted::Normal(page) => {
                self.forget_entries_on(page);
                self.pages[page].alloc = None;
                self.failed_closed = self.pages.len() == MAX_NORMAL_PAGES
                    && self.pages.iter().all(|page| page.alloc.is_none());
            }
        }
    }

    /// Map membership is what makes an entry committed, so the map is what
    /// this walks: the slab can still hold the provisional slot of the very
    /// insertion that faulted.
    ///
    /// The rects are not handed back — the page's allocator is about to go.
    fn forget_entries_on(&mut self, page: usize) {
        let doomed: Vec<SlotId> = self
            .map
            .values()
            .copied()
            .filter(|&slot| self.slab.get(slot).page == page)
            .collect();

        for slot in doomed {
            let key = self.slab.get(slot).key;
            self.lru.unlink(&mut self.slab, slot);
            self.slab.free(slot);
            self.map
                .remove(&key)
                .expect("BUG: resident entry absent from the key map");
        }
    }

    fn allocate_on_existing(&mut self, needed: etagere::Size) -> Option<Reserved> {
        self.pages.iter_mut().enumerate().find_map(|(index, page)| {
            page.allocate(needed)
                .map(|placed| Reserved::new(index, placed))
        })
    }

    /// Frees the coldest entries until the rect fits.
    ///
    /// Entries stamped with the current generation are never freed:
    /// femtovg holds their quads in draw commands queued until the flush,
    /// and overwriting their texels would corrupt the frame being built.
    /// The pop count is bounded: with adversarial fragmentation a tall rect
    /// finds no shelf until a large fraction of the cold population drains.
    fn evict_until_placed(&mut self, needed: etagere::Size) -> Option<Reserved> {
        let mut pops = 0_usize;
        let reserved = loop {
            let Some(cold) = self.lru.coldest() else {
                break None;
            };
            if self.slab.get(cold).last_used == self.generation {
                break None;
            }
            if pops == MAX_EVICTIONS_PER_MISS {
                break None;
            }

            let freed_page = self.remove_entry(cold);
            self.counters.evictions += 1;
            pops += 1;

            let retry_all = pops.is_multiple_of(FULL_RETRY_INTERVAL);
            if let Some(reserved) = self.try_allocate(needed, freed_page, retry_all) {
                break Some(reserved);
            }
        };

        let pops = Generation::try_from(pops).expect("BUG: pop count outside the counter range");
        self.counters.max_evictions_per_miss = self.counters.max_evictions_per_miss.max(pops);
        reserved
    }

    /// Sweeping every page after every pop is the expensive part of eviction,
    /// and only the page that just gave space can plausibly serve the rect,
    /// so the full sweep runs once per `FULL_RETRY_INTERVAL` pops.
    fn try_allocate(
        &mut self,
        needed: etagere::Size,
        freed_page: usize,
        retry_all: bool,
    ) -> Option<Reserved> {
        if retry_all {
            return self.allocate_on_existing(needed);
        }
        self.pages[freed_page]
            .allocate(needed)
            .map(|placed| Reserved::new(freed_page, placed))
    }

    /// Reports the page whose space was freed.
    ///
    /// Freeing a slab slot threads the free chain through the very links
    /// the LRU uses, so the entry must leave the queue first.
    fn remove_entry(&mut self, slot: SlotId) -> usize {
        let entry = self.slab.get(slot);
        let (key, page, alloc_id) = (entry.key, entry.page, entry.alloc_id);

        self.lru.unlink(&mut self.slab, slot);
        self.slab.free(slot);
        self.map
            .remove(&key)
            .expect("BUG: resident entry absent from the key map");
        self.pages[page].deallocate(alloc_id);
        page
    }

    fn quad(&self, slot: SlotId) -> GlyphQuad<P> {
        let entry = self.slab.get(slot);
        quad_at(
            self.pages[entry.page].id,
            entry.content_x,
            entry.content_y,
            entry.placement,
        )
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
    use std::cell::Cell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
            prev: None,
            next: None,
        }
    }

    fn push_entries(slab: &mut EntrySlab, lru: &mut LruQueue, count: usize) -> Vec<SlotId> {
        (0..count)
            .map(|i| {
                let idx = slab.alloc(test_entry(i)).expect("BUG: slab full in test");
                lru.push_hot(slab, idx);
                idx
            })
            .collect()
    }

    fn hot_to_cold(slab: &EntrySlab, lru: &LruQueue) -> Vec<SlotId> {
        let mut order = Vec::new();
        let mut cursor = lru.head;
        while let Some(slot) = cursor {
            order.push(slot);
            cursor = slab.get(slot).next;
        }
        order
    }

    /// The same order read through the `prev` links: a botched unlink leaves
    /// the two directions disagreeing, which the forward walk alone hides.
    fn cold_to_hot(slab: &EntrySlab, lru: &LruQueue) -> Vec<SlotId> {
        let mut order = Vec::new();
        let mut cursor = lru.tail;
        while let Some(slot) = cursor {
            order.push(slot);
            cursor = slab.get(slot).prev;
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
        let idx: Vec<SlotId> = (0..64)
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

        let victim = SlotId::from_index(7);
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

    /// Coverage that differs per texel, so a blit shifted by a row or a column
    /// shows up as a byte difference; a solid glyph would hide it.
    fn ramp_glyph(width: usize, height: usize) -> RasterGlyph {
        RasterGlyph {
            width,
            height,
            left: 1,
            top: -2,
            coverage: (0..width * height)
                .map(|texel| {
                    u8::try_from(texel % 251).expect("BUG: ramp value outside the byte range")
                })
                .collect(),
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
            GlyphLookup::Oversized => panic!("BUG: expected a resident glyph, got Oversized"),
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
    fn oversized_rasters_are_memoized_without_page_creation() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();

        for (index, dimensions) in [(511, 20), (20, 511)].into_iter().enumerate() {
            let key = test_cache_key(index);
            assert_eq!(
                cache.get_or_insert(&mut backend, key, |_| {
                    Some(solid_glyph(dimensions.0, dimensions.1))
                }),
                GlyphLookup::Oversized
            );
            assert_eq!(
                cache.get_or_insert(&mut backend, key, |_| panic!(
                    "BUG: a known-oversized glyph must not rasterize"
                )),
                GlyphLookup::Oversized
            );
            assert!(!cache.map.contains_key(&GlyphKey::normalize(key)));
        }
        assert_eq!(backend.pages_created, 0);
        assert!(backend.uploads.is_empty());
    }

    /// Padded to 402 px, which leaves no shelf a second one could share.
    /// Each of these glyphs takes a page to itself, so twenty of them fill
    /// the whole normal budget — instead of thousands of small ones.
    const PAGE_FILLING_GLYPH_PX: usize = 400;

    fn page_filling_glyph() -> RasterGlyph {
        solid_glyph(PAGE_FILLING_GLYPH_PX, PAGE_FILLING_GLYPH_PX)
    }

    /// Fills every normal page with entries no frame still references:
    /// each insert ends its own frame, so no entry carries this generation
    /// and every one of them is an eviction candidate.
    fn fill_cold_pages(cache: &mut GlyphCache<usize>, backend: &mut test_support::MockBackend) {
        for index in 0..MAX_NORMAL_PAGES {
            let _ = resident(cache.get_or_insert(backend, test_cache_key(index), |_| {
                Some(page_filling_glyph())
            }));
            cache.end_frame();
        }
        assert_eq!(
            backend.pages_created, MAX_NORMAL_PAGES,
            "BUG: a page-filling glyph must open a page of its own"
        );
        assert_eq!(cache.counters().evictions, 0);
    }

    #[test]
    fn a_full_atlas_evicts_the_coldest_entry_to_serve_a_miss() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);

        let newcomer = test_cache_key(MAX_NORMAL_PAGES);
        let _ = resident(cache.get_or_insert(&mut backend, newcomer, |_| {
            Some(solid_glyph(PAGE_FILLING_GLYPH_PX, PAGE_FILLING_GLYPH_PX))
        }));

        assert_eq!(cache.counters().evictions, 1);
        assert!(
            !cache.map.contains_key(&test_key(0)),
            "the coldest entry is the one that must go"
        );
        assert!(cache.map.contains_key(&GlyphKey::normalize(newcomer)));
        assert_eq!(cache.slab.len(), MAX_NORMAL_PAGES);
        assert_eq!(cache.pages.len(), MAX_NORMAL_PAGES);
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES);
    }

    /// 4x4 padded rects are small enough that one page holds all 8192 of them,
    /// so the entry cap binds while nineteen pages' worth of pixels stand free.
    #[test]
    fn entry_cap_evicts_before_pixel_space() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        for index in 0..MAX_RESIDENT_ENTRIES {
            let _ = resident(
                cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                    Some(solid_glyph(2, 2))
                }),
            );
            cache.end_frame();
        }
        assert_eq!(cache.slab.len(), MAX_RESIDENT_ENTRIES);
        assert_eq!(cache.counters().evictions, 0);

        let newcomer = test_cache_key(MAX_RESIDENT_ENTRIES);
        let _ = resident(cache.get_or_insert(&mut backend, newcomer, |_| Some(solid_glyph(2, 2))));

        assert!(cache.counters().evictions > 0);
        assert!(cache.slab.len() <= MAX_RESIDENT_ENTRIES);
        assert_eq!(cache.slab.len(), cache.map.len());
        assert!(!cache.map.contains_key(&test_key(0)));
        assert!(cache.map.contains_key(&GlyphKey::normalize(newcomer)));
        assert!(
            cache.pages.len() < MAX_NORMAL_PAGES,
            "pixel space was never the constraint here"
        );
    }

    /// Draws every entry of a full atlas in the current frame,
    /// leaving the cache with no eviction candidate at all.
    fn touch_every_resident_entry(
        cache: &mut GlyphCache<usize>,
        backend: &mut test_support::MockBackend,
    ) {
        for index in 0..MAX_NORMAL_PAGES {
            let _ = resident(cache.get_or_insert(backend, test_cache_key(index), |_| {
                panic!("BUG: a resident glyph must not rasterize")
            }));
        }
    }

    /// Overwriting a rect that a queued draw command still references
    /// would corrupt the frame being built, so a cache touched end to end
    /// keeps every entry however badly the miss needs the space.
    #[test]
    fn current_frame_entries_are_never_evicted() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);

        let _ = cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| {
            Some(page_filling_glyph())
        });

        assert_eq!(cache.counters().evictions, 0);
        assert_eq!(cache.map.len(), MAX_NORMAL_PAGES);
        assert_eq!(
            cache.counters().hits,
            u64::try_from(MAX_NORMAL_PAGES).expect("BUG: page cap outside the counter range")
        );
    }

    /// Fills the normal pages with 32 px padded rects,
    /// stopping once eviction proves they are full.
    /// How many rects fit a page is etagere's packing to decide, not ours.
    fn fill_with_small_glyphs(
        cache: &mut GlyphCache<usize>,
        backend: &mut test_support::MockBackend,
    ) -> usize {
        let mut inserted = 0;
        while cache.counters().evictions == 0 {
            assert!(inserted < 20_000, "BUG: the normal pages never filled");
            let _ = resident(cache.get_or_insert(backend, test_cache_key(inserted), |_| {
                Some(solid_glyph(30, 30))
            }));
            cache.end_frame();
            inserted += 1;
        }
        inserted
    }

    /// A 302 px rect needs a shelf taller than any three 32 px shelves
    /// coalesced. However long the loop runs, freeing cold entries can never
    /// serve it — only the bound stops it.
    #[test]
    fn eviction_bound_holds() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let inserted = fill_with_small_glyphs(&mut cache, &mut backend);
        let evictions_before = cache.counters().evictions;

        let _ = cache.get_or_insert(&mut backend, test_cache_key(inserted), |_| {
            Some(solid_glyph(300, 300))
        });

        let bound =
            u64::try_from(MAX_EVICTIONS_PER_MISS).expect("BUG: bound outside the counter range");
        assert_eq!(cache.counters().evictions - evictions_before, bound);
        assert_eq!(cache.counters().max_evictions_per_miss, bound);
    }

    /// A fixed shuffle rather than the natural order:
    /// which bin arrives first must not decide what the cache stores.
    const SHUFFLED_BINS: [(SubpixelBin, SubpixelBin); 16] = [
        (SubpixelBin::Two, SubpixelBin::One),
        (SubpixelBin::Zero, SubpixelBin::Three),
        (SubpixelBin::Three, SubpixelBin::Three),
        (SubpixelBin::One, SubpixelBin::Zero),
        (SubpixelBin::Zero, SubpixelBin::One),
        (SubpixelBin::Three, SubpixelBin::Zero),
        (SubpixelBin::Two, SubpixelBin::Three),
        (SubpixelBin::One, SubpixelBin::Two),
        (SubpixelBin::Zero, SubpixelBin::Zero),
        (SubpixelBin::Three, SubpixelBin::One),
        (SubpixelBin::One, SubpixelBin::Three),
        (SubpixelBin::Two, SubpixelBin::Zero),
        (SubpixelBin::Zero, SubpixelBin::Two),
        (SubpixelBin::Three, SubpixelBin::Two),
        (SubpixelBin::One, SubpixelBin::One),
        (SubpixelBin::Two, SubpixelBin::Two),
    ];

    fn keyed_with_bins(x_bin: SubpixelBin, y_bin: SubpixelBin) -> cosmic_text::CacheKey {
        cosmic_text::CacheKey {
            x_bin,
            y_bin,
            ..test_cache_key(1)
        }
    }

    #[test]
    fn sixteen_bin_lookups_rasterize_once_with_zero_bins() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let mut rasterized = Vec::new();
        let mut quads = Vec::new();

        for (x_bin, y_bin) in SHUFFLED_BINS {
            quads.push(resident(cache.get_or_insert(
                &mut backend,
                keyed_with_bins(x_bin, y_bin),
                |normalized| {
                    rasterized.push(normalized);
                    Some(ramp_glyph(3, 5))
                },
            )));
        }

        assert_eq!(rasterized.len(), 1);
        assert_eq!(rasterized[0].0.x_bin, SubpixelBin::Zero);
        assert_eq!(rasterized[0].0.y_bin, SubpixelBin::Zero);
        assert_eq!(backend.uploads.len(), 1);
        assert!(quads.iter().all(|quad| *quad == quads[0]));
        assert_eq!(cache.counters().misses, 1);
        assert_eq!(cache.counters().hits, 15);
    }

    /// The page bytes a fresh cache holds after one insert with these bins.
    fn coverage_of_first_insert(x_bin: SubpixelBin, y_bin: SubpixelBin) -> Vec<u8> {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let _ = resident(cache.get_or_insert(
            &mut backend,
            keyed_with_bins(x_bin, y_bin),
            |normalized| {
                assert_eq!(normalized.0.x_bin, SubpixelBin::Zero);
                assert_eq!(normalized.0.y_bin, SubpixelBin::Zero);
                Some(ramp_glyph(3, 5))
            },
        ));
        backend.pages.remove(0).pixels
    }

    /// The shuffled run above only exercises whichever bin leads it.
    /// A leak of the raw bins into rasterization or placement stays invisible
    /// for the other fifteen first arrivals.
    #[test]
    fn every_bin_first_inserter_stores_identical_coverage() {
        let reference = coverage_of_first_insert(SubpixelBin::Zero, SubpixelBin::Zero);
        for (x_bin, y_bin) in SHUFFLED_BINS {
            assert!(
                coverage_of_first_insert(x_bin, y_bin) == reference,
                "first inserter ({x_bin:?}, {y_bin:?}) stored different coverage"
            );
        }
    }

    /// An orphan is an eviction path that skipped the deallocate,
    /// and counting live allocations is what catches one.
    /// etagere's `allocated_space` cannot: it charges whole shelf rectangles
    /// and returns nothing until an entire bucket drains.
    #[test]
    fn entry_cap_churn_does_not_leak_atlas_space() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        for index in 0..MAX_RESIDENT_ENTRIES * 3 {
            let _ = cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                Some(solid_glyph(2, 2))
            });
            cache.end_frame();
        }

        let live: usize = cache
            .pages
            .iter()
            .map(|page| page.allocs - page.deallocs)
            .sum();
        assert!(cache.counters().evictions > 0);
        assert_eq!(live, cache.slab.len());
        assert_eq!(live, cache.map.len());
        assert!(cache.slab.len() <= MAX_RESIDENT_ENTRIES);
    }

    /// Distinct exact-size keys of the kind an animated font size mints,
    /// one per frame, for every space character.
    /// The step divides exactly, so no two steps can collide in `font_size_bits`.
    fn empty_key_at(step: usize) -> cosmic_text::CacheKey {
        let step = f32::from(u16::try_from(step).expect("BUG: test step outside the size range"));
        cosmic_text::CacheKey {
            font_size_bits: (17.0 + step / 16.0).to_bits(),
            ..test_cache_key(1)
        }
    }

    #[test]
    fn a_glyph_with_no_coverage_rasterizes_once() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let key = test_cache_key(1);

        assert_eq!(
            cache.get_or_insert(&mut backend, key, |_| None),
            GlyphLookup::Missing
        );
        assert_eq!(
            cache.get_or_insert(&mut backend, key, |_| panic!(
                "BUG: a known-empty glyph must not rasterize"
            )),
            GlyphLookup::Missing
        );
        assert_eq!(backend.pages_created, 0);
        assert!(backend.uploads.is_empty());
    }

    /// Sizes far past the cap, because the negative cache exists to survive
    /// exactly that: an unbounded `None` map would grow the heap forever.
    const EMPTY_SIZE_STEPS: usize = 500;

    #[test]
    fn hundreds_of_empty_sizes_stay_bounded_and_drop_the_oldest() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();

        for step in 0..EMPTY_SIZE_STEPS {
            assert_eq!(
                cache.get_or_insert(&mut backend, empty_key_at(step), |_| None),
                GlyphLookup::Missing
            );
            assert!(cache.negative.len <= NEGATIVE_CACHE_CAP);
        }
        assert_eq!(cache.negative.len, NEGATIVE_CACHE_CAP);

        assert_eq!(
            cache.get_or_insert(
                &mut backend,
                empty_key_at(EMPTY_SIZE_STEPS - 1),
                |_| panic!("BUG: the newest empty key must still be known")
            ),
            GlyphLookup::Missing
        );

        let mut rasterized = false;
        assert_eq!(
            cache.get_or_insert(&mut backend, empty_key_at(0), |_| {
                rasterized = true;
                None
            }),
            GlyphLookup::Missing
        );
        assert!(rasterized, "the oldest empty key must have aged out");
        assert_eq!(cache.negative.len, NEGATIVE_CACHE_CAP);
        assert_eq!(backend.pages_created, 0);
    }

    /// The whole structure is inline arrays, so its footprint is its size —
    /// nothing behind it can grow, and the spec's 40 bytes per entry holds.
    #[test]
    fn the_negative_cache_owns_no_heap_storage() {
        assert!(
            size_of::<NegativeCache>() <= NEGATIVE_CACHE_CAP * 40,
            "negative cache footprint {} exceeds its budget",
            size_of::<NegativeCache>()
        );
        assert!(
            size_of::<NegativeCache>() >= NEGATIVE_CACHE_CAP * size_of::<Option<GlyphKey>>(),
            "negative cache footprint {} is below inline storage requirement",
            size_of::<NegativeCache>()
        );
    }

    fn scratch_page_of(cache: &GlyphCache<usize>) -> usize {
        cache
            .scratch
            .expect("BUG: no scratch page was ever created")
    }

    /// The miss the protected cache cannot serve has to go somewhere,
    /// and the scratch page is the only place left that costs no eviction.
    #[test]
    fn protected_cache_routes_to_scratch() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);

        let quad = resident(cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_NORMAL_PAGES),
            |_| Some(page_filling_glyph()),
        ));

        assert_eq!(quad.page, scratch_page_of(&cache));
        assert_eq!(cache.counters().scratch_uses, 1);
        assert_eq!(cache.counters().evictions, 0);
        assert_eq!(cache.map.len(), MAX_NORMAL_PAGES);
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
    }

    /// A repeat of an unservable glyph must not re-run the eviction sweep:
    /// every attempt destroys up to `MAX_EVICTIONS_PER_MISS` cold entries,
    /// so one glyph repeated in a word would strip the cache several times.
    #[test]
    fn scratch_dedups_within_frame() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let inserted = fill_with_small_glyphs(&mut cache, &mut backend);
        let evictions_before = cache.counters().evictions;
        let uploads_before = backend.uploads.len();

        let key = test_cache_key(inserted);
        let rasterizations = Cell::new(0_usize);
        let quads: Vec<GlyphQuad<usize>> = (0..3)
            .map(|_| {
                resident(cache.get_or_insert(&mut backend, key, |_| {
                    rasterizations.set(rasterizations.get() + 1);
                    Some(solid_glyph(300, 300))
                }))
            })
            .collect();

        assert_eq!(rasterizations.get(), 1);
        assert!(quads.iter().all(|quad| *quad == quads[0]));
        assert_eq!(quads[0].page, scratch_page_of(&cache));
        assert_eq!(backend.uploads.len(), uploads_before + 1);
        assert_eq!(cache.counters().scratch_uses, 3);
        assert_eq!(
            cache.counters().evictions - evictions_before,
            u64::try_from(MAX_EVICTIONS_PER_MISS).expect("BUG: bound outside the counter range"),
            "the repeats must not sweep the cache again"
        );
    }

    /// The image is what must survive the flush, never its contents:
    /// a glyph on the scratch page next frame is one that got there again.
    #[test]
    fn scratch_resets_after_flush_but_keeps_image() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);

        let key = test_cache_key(MAX_NORMAL_PAGES);
        let first =
            resident(cache.get_or_insert(&mut backend, key, |_| Some(page_filling_glyph())));
        let page = scratch_page_of(&cache);
        let pages_created = backend.pages_created;

        cache.end_frame();
        assert!(cache.scratch_map.is_empty());
        assert_eq!(cache.scratch, Some(page));

        touch_every_resident_entry(&mut cache, &mut backend);
        let rasterizations = Cell::new(0_usize);
        let second = resident(cache.get_or_insert(&mut backend, key, |_| {
            rasterizations.set(rasterizations.get() + 1);
            Some(page_filling_glyph())
        }));

        assert_eq!(rasterizations.get(), 1);
        assert_eq!(second, first);
        assert_eq!(backend.pages_created, pages_created);
    }

    /// Two page-filling glyphs cannot share the 512 px scratch page,
    /// so the second one is what the frame gives up on.
    #[test]
    fn scratch_full_drops_and_counts() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);

        let _ = resident(cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_NORMAL_PAGES),
            |_| Some(page_filling_glyph()),
        ));
        assert_eq!(cache.counters().glyphs_dropped, 0);

        assert_eq!(
            cache.get_or_insert(
                &mut backend,
                test_cache_key(MAX_NORMAL_PAGES + 1),
                |_| Some(page_filling_glyph())
            ),
            GlyphLookup::Dropped
        );
        assert_eq!(cache.counters().glyphs_dropped, 1);
        assert_eq!(cache.counters().evictions, 0);
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
    }

    /// Tiny glyphs would pack thousands of frame-local entries into one page,
    /// so the map — not the pixels — is what bounds the scratch metadata.
    #[test]
    fn scratch_map_cap_drops() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        for index in 0..MAX_RESIDENT_ENTRIES {
            let _ = resident(
                cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                    Some(solid_glyph(2, 2))
                }),
            );
            cache.end_frame();
        }
        for index in 0..MAX_RESIDENT_ENTRIES {
            let _ = resident(
                cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                    panic!("BUG: a resident glyph must not rasterize")
                }),
            );
        }

        for extra in 0..SCRATCH_MAP_CAP {
            let _ = resident(cache.get_or_insert(
                &mut backend,
                test_cache_key(MAX_RESIDENT_ENTRIES + extra),
                |_| Some(solid_glyph(12, 12)),
            ));
        }
        assert_eq!(cache.scratch_map.len(), SCRATCH_MAP_CAP);
        assert_eq!(cache.counters().glyphs_dropped, 0);

        assert_eq!(
            cache.get_or_insert(
                &mut backend,
                test_cache_key(MAX_RESIDENT_ENTRIES + SCRATCH_MAP_CAP),
                |_| Some(solid_glyph(12, 12))
            ),
            GlyphLookup::Dropped
        );
        assert_eq!(cache.counters().glyphs_dropped, 1);
        assert_eq!(cache.counters().evictions, 0);
    }

    #[test]
    fn churn_never_exceeds_resident_and_scratch_page_cap() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);

        let mut key = MAX_NORMAL_PAGES;
        for _ in 0..8 {
            for _ in 0..MAX_NORMAL_PAGES + 5 {
                let _ = cache.get_or_insert(&mut backend, test_cache_key(key), |_| {
                    Some(page_filling_glyph())
                });
                key += 1;
            }
            cache.end_frame();

            for _ in 0..64 {
                let _ = cache.get_or_insert(&mut backend, test_cache_key(key), |_| {
                    Some(solid_glyph(30, 30))
                });
                key += 1;
            }
            cache.end_frame();
        }

        assert!(cache.counters().evictions > 0);
        assert!(cache.counters().scratch_uses > 0);
        assert!(cache.counters().glyphs_dropped > 0);
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
    }

    #[test]
    fn scratch_create_failure_is_transient_and_counted() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);

        let key = test_cache_key(MAX_NORMAL_PAGES);
        backend.fail_next_create = true;
        assert_eq!(
            cache.get_or_insert(&mut backend, key, |_| Some(page_filling_glyph())),
            GlyphLookup::Missing
        );
        assert_eq!(cache.counters().page_create_failures, 1);
        assert_eq!(cache.scratch, None);
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES);

        let quad = resident(cache.get_or_insert(&mut backend, key, |_| Some(page_filling_glyph())));
        assert_eq!(quad.page, scratch_page_of(&cache));
        assert_eq!(cache.counters().page_create_failures, 1);
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
    }

    /// A failed upload must return its reservation so the next glyph can
    /// still use the whole scratch page.
    #[test]
    fn scratch_upload_failure_releases_allocation() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);

        backend.fail_next_upload = Some(PageFaultKind::Transient);
        assert_eq!(
            cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| Some(
                page_filling_glyph()
            )),
            GlyphLookup::Missing
        );
        assert_eq!(cache.counters().upload_transient_failures, 1);
        assert!(cache.scratch_map.is_empty());
        assert!(cache.scratch_packer.is_empty());

        let padded = PAGE_FILLING_GLYPH_PX + 2;
        let _ = resident(cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_NORMAL_PAGES + 1),
            |_| Some(page_filling_glyph()),
        ));

        let &(page, x, y, width, height) = backend
            .uploads
            .last()
            .expect("BUG: the scratch insertion uploaded nothing");
        assert_eq!(
            (page, x, y, width, height),
            (scratch_page_of(&cache), 0, 0, padded, padded)
        );
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
    }

    /// Counts WARN records without a `tracing-subscriber` dev dependency.
    /// The diagnostic's whole point is that it fires once per interval
    /// however many glyphs suffered, and only a subscriber can witness it.
    #[derive(Clone, Default)]
    struct WarnCounter(Arc<AtomicUsize>);

    impl WarnCounter {
        fn count(&self) -> usize {
            self.0.load(Ordering::Relaxed)
        }

        fn reset(&self) {
            self.0.store(0, Ordering::Relaxed);
        }
    }

    impl tracing::Subscriber for WarnCounter {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::WARN
        }

        fn event(&self, _: &tracing::Event<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }

        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// Reaches the diagnostic's callsite from a thread that has a subscriber
    /// installed, then rebuilds the interest cache.
    ///
    /// tracing caches a callsite's interest process-wide on first reach,
    /// so a sibling test reaching it first from a subscriber-less thread
    /// would pin it disabled for the whole test binary.
    fn arm_pressure_callsite() {
        let mut cache = GlyphCache::<usize>::new();
        cache.counters.glyphs_dropped += 1;
        cache.end_frame();
        tracing::callsite::rebuild_interest_cache();
    }

    /// Runs `scenario` with a WARN counter installed and reports what it saw.
    fn counting_warns(scenario: impl FnOnce(&WarnCounter)) -> usize {
        let warns = WarnCounter::default();
        tracing::subscriber::with_default(warns.clone(), || {
            arm_pressure_callsite();
            warns.reset();
            scenario(&warns);
        });
        warns.count()
    }

    #[test]
    fn repeated_failing_frames_log_once_per_interval() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();

        let records = counting_warns(|_| {
            for frame in 0..3 {
                backend.fail_next_upload = Some(PageFaultKind::Transient);
                assert_eq!(
                    cache.get_or_insert(&mut backend, test_cache_key(frame), |_| Some(
                        solid_glyph(3, 5)
                    )),
                    GlyphLookup::Missing
                );
                cache.end_frame();
            }
        });

        assert_eq!(cache.counters().upload_transient_failures, 3);
        assert_eq!(records, 1);
    }

    #[test]
    fn one_record_carries_every_tally_of_a_churning_frame() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();

        let records = counting_warns(|warns| {
            fill_cold_pages(&mut cache, &mut backend);
            assert_eq!(warns.count(), 0, "quiet frames must stay silent");

            // A fresh entry on every page stamps the atlas with this frame,
            // so what follows can only go to scratch.
            for index in 0..MAX_NORMAL_PAGES {
                let _ = resident(cache.get_or_insert(
                    &mut backend,
                    test_cache_key(MAX_NORMAL_PAGES + index),
                    |_| Some(page_filling_glyph()),
                ));
            }
            let _ = resident(cache.get_or_insert(
                &mut backend,
                test_cache_key(2 * MAX_NORMAL_PAGES),
                |_| Some(page_filling_glyph()),
            ));
            let _ = cache.get_or_insert(
                &mut backend,
                test_cache_key(2 * MAX_NORMAL_PAGES + 1),
                |_| Some(page_filling_glyph()),
            );
            let _ = cache.get_or_insert(
                &mut backend,
                test_cache_key(2 * MAX_NORMAL_PAGES + 2),
                |_| Some(solid_glyph(600, 20)),
            );
            backend.fail_next_upload = Some(PageFaultKind::Transient);
            let _ = cache.get_or_insert(
                &mut backend,
                test_cache_key(2 * MAX_NORMAL_PAGES + 3),
                |_| Some(solid_glyph(3, 5)),
            );
            cache.end_frame();
        });

        assert_eq!(
            cache.counters().evictions,
            u64::try_from(MAX_NORMAL_PAGES).expect("BUG: page cap outside the counter range")
        );
        assert_eq!(cache.counters().scratch_uses, 1);
        assert_eq!(cache.counters().glyphs_dropped, 1);
        assert_eq!(cache.counters().glyphs_oversized, 1);
        assert_eq!(cache.counters().upload_transient_failures, 1);
        assert_eq!(records, 1);
    }

    /// [`fill_cold_pages`] gives every glyph a page of its own in key order,
    /// so this page holds exactly `test_cache_key(3)`.
    const FAULTED_PAGE: usize = 3;

    /// Quarantine is release-build behaviour —
    /// a debug build asserts on the fault instead —
    /// so the tests reach it through the handler, not an upload.
    #[test]
    fn invariant_fault_quarantines_the_page() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);

        cache.handle_invariant_fault(Faulted::Normal(FAULTED_PAGE));

        assert_eq!(cache.counters().cache_invariant_failures, 1);
        assert!(cache.pages[FAULTED_PAGE].alloc.is_none());
        assert!(!cache.map.contains_key(&test_key(FAULTED_PAGE)));
        assert_eq!(cache.map.len(), MAX_NORMAL_PAGES - 1);
        assert_eq!(cache.slab.len(), MAX_NORMAL_PAGES - 1);
        assert_eq!(
            cache.pages.len(),
            MAX_NORMAL_PAGES,
            "the faulted page stays retained, image and all"
        );

        let rasterized = Cell::new(false);
        let quad =
            resident(
                cache.get_or_insert(&mut backend, test_cache_key(FAULTED_PAGE), |_| {
                    rasterized.set(true);
                    Some(page_filling_glyph())
                }),
            );

        assert!(rasterized.get(), "a forgotten key must miss again");
        assert_ne!(quad.page, cache.pages[FAULTED_PAGE].id);
        assert_eq!(
            backend.pages_created, MAX_NORMAL_PAGES,
            "a quarantined page is never replaced"
        );
        assert_eq!(
            cache
                .pages
                .iter()
                .filter(|page| page.alloc.is_some())
                .count(),
            MAX_NORMAL_PAGES - 1,
            "capacity shrinks by the faulted page for good"
        );
    }

    /// Past the negative cache's capacity, so its oldest keys age out:
    /// quarantine kept there would age out with them and retry the fault.
    const NEGATIVE_CHURN_STEPS: usize = 300;

    #[test]
    fn quarantine_survives_negative_churn() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        cache.handle_invariant_fault(Faulted::Normal(FAULTED_PAGE));
        let faulted_id = cache.pages[FAULTED_PAGE].id;
        let uploads_before = backend.uploads.len();

        for step in 1..=NEGATIVE_CHURN_STEPS {
            assert_eq!(
                cache.get_or_insert(&mut backend, empty_key_at(step), |_| None),
                GlyphLookup::Missing
            );
        }
        assert_eq!(cache.negative.len, NEGATIVE_CACHE_CAP);

        let quad = resident(cache.get_or_insert(
            &mut backend,
            test_cache_key(FAULTED_PAGE),
            |_| Some(page_filling_glyph()),
        ));

        assert_ne!(quad.page, faulted_id);
        assert!(cache.pages[FAULTED_PAGE].alloc.is_none());
        assert!(
            backend.uploads[uploads_before..]
                .iter()
                .all(|&(page, ..)| page != faulted_id),
            "nothing may be uploaded to a quarantined page"
        );
    }

    #[test]
    fn scratch_fault_latches_scratch_off() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);
        let _ = resident(cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_NORMAL_PAGES),
            |_| Some(page_filling_glyph()),
        ));
        let pages_created = backend.pages_created;

        cache.handle_invariant_fault(Faulted::Scratch);

        assert!(cache.scratch_latched_off);
        assert!(cache.scratch_map.is_empty());
        assert_eq!(cache.counters().cache_invariant_failures, 1);
        assert_eq!(
            cache.get_or_insert(
                &mut backend,
                test_cache_key(MAX_NORMAL_PAGES + 1),
                |_| Some(page_filling_glyph())
            ),
            GlyphLookup::Dropped
        );
        assert_eq!(cache.counters().glyphs_dropped, 1);

        cache.end_frame();
        touch_every_resident_entry(&mut cache, &mut backend);
        assert_eq!(
            cache.get_or_insert(
                &mut backend,
                test_cache_key(MAX_NORMAL_PAGES + 2),
                |_| Some(page_filling_glyph())
            ),
            GlyphLookup::Dropped,
            "the latch outlives the frame that tripped it"
        );
        assert_eq!(cache.counters().glyphs_dropped, 2);
        assert_eq!(
            backend.pages_created, pages_created,
            "no replacement scratch page is ever created"
        );
    }

    #[test]
    fn all_pages_quarantined_fails_closed() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);

        for page in 0..MAX_NORMAL_PAGES {
            assert!(!cache.failed_closed, "one healthy page is still a cache");
            cache.handle_invariant_fault(Faulted::Normal(page));
        }

        assert!(cache.failed_closed);
        assert!(cache.map.is_empty());
        assert_eq!(cache.slab.len(), 0);
        let pages_created = backend.pages_created;

        for index in 0..3 {
            assert_eq!(
                cache.get_or_insert(&mut backend, test_cache_key(index), |_| panic!(
                    "BUG: a failed-closed cache must not rasterize"
                )),
                GlyphLookup::Dropped
            );
        }
        assert_eq!(cache.counters().glyphs_dropped, 3);
        assert_eq!(backend.pages_created, pages_created);
    }

    /// The mid-insertion state an upload fault rolls back:
    /// a rect reserved on a page
    /// and a slab slot the map and the LRU have never seen.
    fn provisional_entry(
        cache: &mut GlyphCache<usize>,
        backend: &mut test_support::MockBackend,
        key: GlyphKey,
    ) -> (Reserved, SlotId) {
        let pending = Pending::new(&solid_glyph(30, 30));
        let Ok(reserved) = cache.allocate(backend, pending.size()) else {
            panic!("BUG: the test cache refused a rect it had room for");
        };
        let slot = cache
            .slab
            .alloc(Entry {
                key,
                page: reserved.page,
                alloc_id: reserved.alloc_id,
                content_x: reserved.content_x,
                content_y: reserved.content_y,
                placement: pending.placement,
                last_used: cache.generation,
                prev: None,
                next: None,
            })
            .expect("BUG: slab full in test");
        (reserved, slot)
    }

    /// Unlinking the provisional entry would rewrite the queue's ends
    /// from its absent links, cutting every survivor loose —
    /// which only entries on other pages, untouched by the quarantine,
    /// can witness.
    #[test]
    fn invariant_fault_preserves_unrelated_lru_order() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        for index in 0..3 {
            let _ = resident(
                cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                    Some(page_filling_glyph())
                }),
            );
            cache.end_frame();
        }
        let _ = resident(cache.get_or_insert(&mut backend, test_cache_key(1), |_| {
            panic!("BUG: a resident glyph must not rasterize")
        }));

        let (reserved, slot) = provisional_entry(&mut cache, &mut backend, test_key(9));
        assert_eq!(
            reserved.page, 0,
            "the provisional rect must land on the page the fault will quarantine"
        );
        let forgotten = cache.map[&test_key(0)];
        let expected: Vec<SlotId> = cold_to_hot(&cache.slab, &cache.lru)
            .into_iter()
            .filter(|entry| *entry != forgotten)
            .collect();

        cache.roll_back_insertion(reserved, slot);
        cache.handle_invariant_fault(Faulted::Normal(reserved.page));

        assert_eq!(cold_to_hot(&cache.slab, &cache.lru), expected);
        assert_eq!(
            hot_to_cold(&cache.slab, &cache.lru),
            expected.iter().rev().copied().collect::<Vec<SlotId>>()
        );
        assert!(cache.map.contains_key(&test_key(1)));
        assert!(cache.map.contains_key(&test_key(2)));
        assert_eq!(cache.map.len(), 2);
        assert_eq!(cache.slab.len(), 2);
    }

    /// The fault means our own rect geometry or page bookkeeping is wrong,
    /// so a debug build stops at it instead of quarantining and carrying on.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "invariant fault")]
    fn an_invariant_upload_fault_panics_in_debug() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::<usize>::new();
        backend.fail_next_upload = Some(PageFaultKind::Invariant);

        let _ = cache.get_or_insert(&mut backend, test_cache_key(1), |_| Some(solid_glyph(3, 5)));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "invariant fault")]
    fn an_invariant_scratch_fault_panics_in_debug() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        fill_cold_pages(&mut cache, &mut backend);
        touch_every_resident_entry(&mut cache, &mut backend);
        backend.fail_next_upload = Some(PageFaultKind::Invariant);

        let _ = cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| {
            Some(page_filling_glyph())
        });
    }

    const METADATA_CEILING_BYTES: usize = 3 * 1024 * 1024;

    const GLYPH_IDS_PER_SIZE: usize = 1 << 16;

    /// `test_cache_key` runs out of distinct keys at the 16-bit glyph id,
    /// and a churn outrunning the resident cap many times over needs more,
    /// so the size moves on whenever the glyph ids wrap.
    #[expect(
        clippy::integer_division,
        reason = "the size advances once per exhausted run of glyph ids"
    )]
    fn churn_key(step: usize) -> cosmic_text::CacheKey {
        let size_step = u16::try_from(step / GLYPH_IDS_PER_SIZE)
            .expect("BUG: churn step outside the size range");
        cosmic_text::CacheKey {
            font_size_bits: (17.0 + f32::from(size_step) / 16.0).to_bits(),
            ..test_cache_key(step % GLYPH_IDS_PER_SIZE)
        }
    }

    /// Every path that could enlarge a container, in one run:
    /// tiny glyphs past the resident cap with their predecessors cold,
    /// so eviction churns the slab and the key map;
    /// the same again with every predecessor drawn this frame,
    /// so the scratch page and its map fill and then drop;
    /// and empty keys past the negative cache.
    /// All 21 page textures exist by the end.
    fn churn_past_every_cap(
        cache: &mut GlyphCache<usize>,
        backend: &mut test_support::MockBackend,
        cold_steps: usize,
    ) {
        fill_cold_pages(cache, backend);

        for step in 0..cold_steps {
            let _ = cache.get_or_insert(backend, churn_key(step), |_| Some(solid_glyph(2, 2)));
            cache.end_frame();
            backend.uploads.clear();
        }

        let hot_steps = MAX_RESIDENT_ENTRIES + SCRATCH_MAP_CAP + 1;
        for step in cold_steps..cold_steps + hot_steps {
            let _ = cache.get_or_insert(backend, churn_key(step), |_| Some(solid_glyph(2, 2)));
        }
        cache.end_frame();
        backend.uploads.clear();

        for step in 0..EMPTY_SIZE_STEPS {
            let _ = cache.get_or_insert(backend, empty_key_at(step), |_| None);
        }
        cache.end_frame();
    }

    /// Long enough to cross every cap, and short of the eviction count
    /// at which hashbrown gives up rehashing the key map in place.
    const BOUNDED_CHURN_STEPS: usize = 2 * MAX_RESIDENT_ENTRIES;

    /// Nothing a frame does may enlarge a container `new()` sized;
    /// the estimates still fit once all resident and scratch pages exist.
    #[test]
    fn metadata_bound_holds_at_allocated_capacity() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let when_new = cache.app_owned_metadata_bytes();

        churn_past_every_cap(&mut cache, &mut backend, BOUNDED_CHURN_STEPS);

        assert!(cache.counters().evictions > 0);
        assert!(cache.counters().scratch_uses > 0);
        assert!(cache.counters().glyphs_dropped > 0);
        assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
        assert_eq!(cache.app_owned_metadata_bytes(), when_new);
        assert!(
            cache.metadata_capacity_bytes() <= METADATA_CEILING_BYTES,
            "metadata {} exceeds the ceiling",
            cache.metadata_capacity_bytes()
        );
        assert_eq!(
            cache.resident_atlas_bytes(),
            (MAX_NORMAL_PAGES + 1) * PAGE_SIZE_PX * PAGE_SIZE_PX
        );
    }

    /// Past this many evictions hashbrown stops rehashing the key map in place
    /// and doubles its table instead: tombstones exhaust the slack of a table
    /// held at 8 192 live entries.
    const SUSTAINED_CHURN_STEPS: usize = 150_000;

    /// So the ceiling covers the table hashbrown ends up with,
    /// not the one `new()` reserved: a bound that only held before the resize
    /// is a bound the renderer outgrows in service.
    #[test]
    fn the_ceiling_covers_the_key_maps_grown_table() {
        let mut backend = test_support::MockBackend::default();
        let mut cache = GlyphCache::new();
        let when_new = cache.app_owned_metadata_bytes();

        churn_past_every_cap(&mut cache, &mut backend, SUSTAINED_CHURN_STEPS);

        assert!(
            cache.app_owned_metadata_bytes() > when_new,
            "the churn no longer reaches the resize, so the ceiling proves nothing here"
        );
        assert!(
            cache.metadata_capacity_bytes() <= METADATA_CEILING_BYTES,
            "metadata {} exceeds the ceiling",
            cache.metadata_capacity_bytes()
        );
    }

    /// hashbrown's widest control group; a narrower target is over-counted,
    /// which is the safe direction for a ceiling.
    const HASHBROWN_GROUP_BYTES: usize = 16;

    /// The spec's accounting table models hashbrown 0.16.1's table
    /// from a capacity: a power-of-two bucket count at a 7/8 load factor,
    /// with one control byte per bucket
    /// and a trailing group the probe reads past the end.
    #[expect(
        clippy::integer_division,
        reason = "hashbrown's own bucket rounding, truncation included"
    )]
    fn map_bucket_bytes(capacity: usize, kv_size: usize) -> usize {
        let buckets = (capacity * 8 / 7).next_power_of_two();
        buckets * kv_size + buckets + HASHBROWN_GROUP_BYTES
    }

    /// The ceiling was derived from that model at the two maps' fresh
    /// capacities. A bump to the load factor, the rounding or the control bytes
    /// would leave the model under the allocation, and the derivation void.
    #[test]
    fn the_spec_model_covers_hashbrowns_own_allocation() {
        let map = hashbrown::HashMap::<GlyphKey, SlotId>::with_capacity(MAX_RESIDENT_ENTRIES);
        assert_eq!(map.capacity(), 14_336);
        assert!(
            map_bucket_bytes(map.capacity(), size_of::<(GlyphKey, SlotId)>())
                >= map.allocation_size()
        );

        let scratch = hashbrown::HashMap::<GlyphKey, ScratchEntry>::with_capacity(SCRATCH_MAP_CAP);
        assert_eq!(scratch.capacity(), 1_792);
        assert!(
            map_bucket_bytes(scratch.capacity(), size_of::<(GlyphKey, ScratchEntry)>())
                >= scratch.allocation_size()
        );
    }

    /// Why the counter asks hashbrown what it allocated
    /// instead of rebuilding the table from `capacity()`:
    /// capacity is `len + growth_left`, and removals that leave tombstones
    /// drain that slack without returning a single bucket.
    /// Modelled from the drained capacity,
    /// a grown table prices at half what it holds.
    #[test]
    fn a_tombstone_drained_table_is_priced_at_its_real_allocation() {
        let mut map = hashbrown::HashMap::<GlyphKey, SlotId>::with_capacity(MAX_RESIDENT_ENTRIES);
        let fresh_allocation = map.allocation_size();
        let mut inserted = 0;
        while map.allocation_size() == fresh_allocation {
            map.insert(test_key(inserted), SlotId::from_index(0));
            inserted += 1;
        }
        while map.len() < map.capacity() {
            map.insert(test_key(inserted), SlotId::from_index(0));
            inserted += 1;
        }
        let grown_allocation = map.allocation_size();

        for step in 0..inserted {
            if map.len() == MAX_RESIDENT_ENTRIES {
                break;
            }
            map.remove(&test_key(step));
        }

        assert_eq!(map.len(), MAX_RESIDENT_ENTRIES);
        assert!(
            map_bucket_bytes(map.capacity(), size_of::<(GlyphKey, u32)>()) < grown_allocation,
            "capacity {} still prices the whole table, so the drain never happened and this \
             proves nothing",
            map.capacity()
        );
        assert_eq!(map.allocation_size(), grown_allocation);
    }
}
