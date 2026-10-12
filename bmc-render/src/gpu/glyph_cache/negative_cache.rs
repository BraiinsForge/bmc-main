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

//! Keys that cannot rasterize, remembered so they are not retried every frame.

use super::GlyphKey;
use super::GlyphLookup;
use super::NEGATIVE_CACHE_CAP;
use super::slab::Link;
use super::slab::SlotId;

/// Keys whose rasterization cannot produce an atlas entry, kept so they cannot
/// re-enter the rasterizer every frame. Storage is inline arrays — an animated
/// font size mints a fresh exact-size key per frame, so a growable map here
/// would just relocate the unbounded growth into the heap.
///
/// Occupied slots are exactly `0..len`;
/// eviction reuses the coldest slot in place,
/// so the arrays are written once and never resized.
pub(super) struct NegativeCache {
    keys: [Option<GlyphKey>; NEGATIVE_CACHE_CAP],
    reasons: [NegativeReason; NEGATIVE_CACHE_CAP],
    prev: [Link; NEGATIVE_CACHE_CAP],
    next: [Link; NEGATIVE_CACHE_CAP],
    head: Link,
    tail: Link,
    pub(super) len: usize,
}

#[derive(Clone, Copy)]
pub(super) enum NegativeReason {
    Missing,
    Oversized,
}

impl NegativeReason {
    pub(super) fn into_lookup<P>(self) -> GlyphLookup<P> {
        match self {
            Self::Missing => GlyphLookup::Missing,
            Self::Oversized => GlyphLookup::Oversized,
        }
    }
}

impl NegativeCache {
    pub(super) fn new() -> Self {
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
    pub(super) fn get(&mut self, key: &GlyphKey) -> Option<NegativeReason> {
        let slot = self.slot_of(key)?;
        self.unlink(slot);
        self.push_hot(slot);
        Some(self.reasons[slot.index()])
    }

    pub(super) fn insert_absent(&mut self, key: GlyphKey, reason: NegativeReason) {
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
