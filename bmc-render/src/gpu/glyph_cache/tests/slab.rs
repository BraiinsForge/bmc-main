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

use super::*;

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
