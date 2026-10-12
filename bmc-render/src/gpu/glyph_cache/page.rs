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

//! Atlas pages and the glyph rects reserved or pending on them.

use super::PAGE_SIZE_PX;
use super::RasterGlyph;
use super::RasterPlacement;

/// One atlas page: the backend's image plus the allocator that packs it.
/// `alloc` is an `Option` so quarantine can drop the allocator's metadata
/// while the page struct and its image stay retained.
pub(super) struct Page<P> {
    pub(super) id: P,
    pub(super) alloc: Option<etagere::BucketedAtlasAllocator>,
    /// Every rect goes through the two methods below, so a leak is exactly
    /// a gap between these: an eviction path that never handed its rect back.
    #[cfg(test)]
    pub(super) allocs: usize,
    #[cfg(test)]
    pub(super) deallocs: usize,
}

impl<P> Page<P> {
    pub(super) fn new(id: P) -> Self {
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

    pub(super) fn allocate(&mut self, size: etagere::Size) -> Option<etagere::Allocation> {
        let placed = self.alloc.as_mut()?.allocate(size)?;
        #[cfg(test)]
        {
            self.allocs += 1;
        }
        Some(placed)
    }

    pub(super) fn deallocate(&mut self, id: etagere::AllocId) {
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
/// It stays apart from [`super::Entry`] because the entry cap can force the rect
/// back, to be re-taken on the far side of an eviction.
#[derive(Clone, Copy)]
pub(super) struct Reserved {
    pub(super) page: usize,
    pub(super) alloc_id: etagere::AllocId,
    pub(super) content_x: usize,
    pub(super) content_y: usize,
}

impl Reserved {
    /// The allocation may span the whole shelf height; only its min corner
    /// is ours, and the content rect is what gets uploaded.
    pub(super) fn new(page: usize, allocation: etagere::Allocation) -> Self {
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
pub(super) struct Pending {
    pub(super) placement: RasterPlacement,
    pub(super) padded_width: usize,
    pub(super) padded_height: usize,
    pub(super) pixels: Vec<u8>,
}

impl Pending {
    pub(super) fn new(raster: &RasterGlyph) -> Self {
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

    pub(super) fn size(&self) -> etagere::Size {
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
pub(super) struct ScratchEntry {
    pub(super) content_x: usize,
    pub(super) content_y: usize,
    pub(super) placement: RasterPlacement,
}
