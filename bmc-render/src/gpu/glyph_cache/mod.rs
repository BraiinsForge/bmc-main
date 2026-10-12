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

use super::lru::LruQueue;

mod negative_cache;
mod page;
mod slab;

use negative_cache::{NegativeCache, NegativeReason};
use page::{Page, Pending, Reserved, ScratchEntry};
pub use slab::{Entry, EntrySlab, Link, SlotId};

pub const PAGE_SIZE_PX: usize = 512;
pub const MAX_NORMAL_PAGES: usize = 10;
pub const MAX_RESIDENT_ENTRIES: usize = 8192;
pub const NEGATIVE_CACHE_CAP: usize = 256;
pub const SCRATCH_MAP_CAP: usize = 1024;
pub const MAX_EVICTIONS_PER_MISS: usize = 64;
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

    pub fn inner(&self) -> cosmic_text::CacheKey {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFaultKind {
    Invariant,
    Transient,
}

/// Page creation failures are transient by definition:
/// an Err carries no PageId, so there is nothing to quarantine —
/// the caller skips this frame, counts, and retries later.
/// Only `upload` can fault against a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageCreateFailed;

/// GL abstraction so the cache is unit-testable without a context;
/// the production impl wraps `Canvas<OpenGl>`.
/// Dimensions are `usize` end-to-end:
/// femtovg's `create_image_empty`/`update_image` take `usize`,
/// so matching it here means no conversions at the only real boundary.
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

/// Lifetime tallies. `negative_cache_hits` is a subset of `misses`,
/// `rasterizations` counts calls that reach the rasterizer including empty glyphs,
/// and `uploads` counts successful normal and scratch page uploads.
/// `u64` because the counters are monotonic and the target is 32-bit.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub hits: u64,
    pub misses: u64,
    pub negative_cache_hits: u64,
    pub rasterizations: u64,
    pub uploads: u64,
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

/// A frame draws one batch per run of glyphs sharing a page, so nothing bounds
/// the batch count itself. The record is retained cache storage like everything
/// else here and gets a fixed capacity: twelve batches for each resident page
/// is more page recurrence than an ordering question can read, and past that
/// the overflow count says what was left out.
#[cfg(feature = "atlas-inspect")]
const MAX_RECORDED_BATCHES: usize = 12 * (MAX_NORMAL_PAGES + 1);

pub struct GlyphCache<P: Copy + Eq + core::fmt::Debug> {
    pages: Vec<Page<P>>,
    map: hashbrown::HashMap<GlyphKey, SlotId>,
    slab: EntrySlab,
    lru: LruQueue<SlotId>,
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
    /// The page of this frame's cached-glyph batches,
    /// in order, up to [`MAX_RECORDED_BATCHES`] of them.
    #[cfg(feature = "atlas-inspect")]
    pages_drawn: Vec<P>,
    /// Batches this frame submitted past that cap,
    /// so a reader can tell a complete record from a truncated one.
    #[cfg(feature = "atlas-inspect")]
    batches_over_cap: usize,
    /// The same pair for the frame the renderer last flushed.
    /// Kept apart from `pages_drawn` because the record is only worth reading
    /// once the pixels it describes exist — one flush after the batches went out.
    #[cfg(feature = "atlas-inspect")]
    pages_drawn_last_frame: Vec<P>,
    #[cfg(feature = "atlas-inspect")]
    batches_over_cap_last_frame: usize,
}

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
            #[cfg(feature = "atlas-inspect")]
            pages_drawn: Vec::with_capacity(MAX_RECORDED_BATCHES),
            #[cfg(feature = "atlas-inspect")]
            batches_over_cap: 0,
            #[cfg(feature = "atlas-inspect")]
            pages_drawn_last_frame: Vec::with_capacity(MAX_RECORDED_BATCHES),
            #[cfg(feature = "atlas-inspect")]
            batches_over_cap_last_frame: 0,
        }
    }

    pub fn counters(&self) -> &Counters {
        &self.counters
    }

    #[cfg(feature = "atlas-inspect")]
    pub fn record_drawn_page(&mut self, page: P) {
        if self.pages_drawn.len() == MAX_RECORDED_BATCHES {
            self.batches_over_cap += 1;
            return;
        }
        self.pages_drawn.push(page);
    }

    /// Retire the frame's page record; it stays readable until the next flush.
    #[cfg(feature = "atlas-inspect")]
    pub fn retire_pages_drawn(&mut self) {
        self.pages_drawn_last_frame.clear();
        std::mem::swap(&mut self.pages_drawn_last_frame, &mut self.pages_drawn);
        self.batches_over_cap_last_frame = std::mem::take(&mut self.batches_over_cap);
    }

    #[cfg(feature = "atlas-inspect")]
    pub fn pages_drawn_last_frame(&self) -> &[P] {
        &self.pages_drawn_last_frame
    }

    #[cfg(feature = "atlas-inspect")]
    pub fn batches_over_cap_last_frame(&self) -> usize {
        self.batches_over_cap_last_frame
    }

    /// Live resident entries. The allocation gate refuses to trust its numbers
    /// until the workload has actually filled the cache to its cap.
    #[cfg(feature = "glyph-alloc-gate")]
    pub fn resident_len(&self) -> usize {
        self.slab.len()
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
        #[cfg(feature = "atlas-inspect")]
        let inspection_record_bytes =
            (self.pages_drawn.capacity() + self.pages_drawn_last_frame.capacity()) * size_of::<P>();
        #[cfg(not(feature = "atlas-inspect"))]
        let inspection_record_bytes = 0;

        self.pages.capacity() * size_of::<Page<P>>()
            + self.slab.capacity() * size_of::<Entry>()
            + self.map.allocation_size()
            + size_of::<NegativeCache>()
            + self.scratch_map.allocation_size()
            + inspection_record_bytes
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
            self.counters.negative_cache_hits += 1;
            return reason.into_lookup();
        }
        self.counters.rasterizations += 1;
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
        self.counters.uploads += 1;

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
        self.counters.uploads += 1;
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
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

#[cfg(all(feature = "glyph-alloc-gate", target_os = "linux"))]
pub mod alloc_gate_support;
