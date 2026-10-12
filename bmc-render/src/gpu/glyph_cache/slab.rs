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

//! Fixed-capacity storage for resident entries, linked into the LRU.

use super::Generation;
use super::GlyphKey;
use super::RasterPlacement;
use crate::gpu::lru::LruStore;
use core::num::NonZeroU32;

/// A compact slab index. Encoding the index plus one gives [`Option`] a
/// four-byte niche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotId(NonZeroU32);

impl SlotId {
    pub(super) fn from_index(index: usize) -> Self {
        let encoded = u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(NonZeroU32::new)
            .expect("BUG: slab index outside the slab index range");
        Self(encoded)
    }

    pub(super) fn index(self) -> usize {
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
pub struct EntrySlab {
    entries: Vec<Entry>,
    free_head: Link,
    live: usize,
}

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
    #[cfg(any(test, feature = "glyph-alloc-gate"))]
    pub fn len(&self) -> usize {
        self.live
    }
}

impl LruStore<SlotId> for EntrySlab {
    fn links(&self, index: SlotId) -> (Link, Link) {
        let entry = self.get(index);
        (entry.prev, entry.next)
    }

    fn set_links(&mut self, index: SlotId, prev: Link, next: Link) {
        let entry = self.get_mut(index);
        entry.prev = prev;
        entry.next = next;
    }

    fn set_prev(&mut self, index: SlotId, prev: Link) {
        self.get_mut(index).prev = prev;
    }

    fn set_next(&mut self, index: SlotId, next: Link) {
        self.get_mut(index).next = next;
    }
}
