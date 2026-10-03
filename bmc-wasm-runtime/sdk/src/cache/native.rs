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

use std::cell::RefCell;
use std::collections::HashMap;

use super::{CacheStoreProvider, CachedEntry, Stat};

thread_local! {
    static STORE: RefCell<HashMap<String, CachedEntry>> = RefCell::new(HashMap::new());
}

/// The wall clock, standing in for the host's injected one.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |it| u64::try_from(it.as_millis()).unwrap_or(u64::MAX))
}

pub(super) struct Host;

impl CacheStoreProvider for Host {
    fn put(tag: &str, metadata: &[u8], bytes: &[u8]) {
        STORE.with(|store| {
            store.borrow_mut().insert(
                tag.to_owned(),
                CachedEntry {
                    saved_at: now_millis(),
                    metadata: metadata.to_vec(),
                    bytes: bytes.to_vec(),
                },
            );
        });
    }

    fn read_bytes(tag: &str) -> Option<CachedEntry> {
        STORE.with(|store| store.borrow().get(tag).cloned())
    }

    fn evict(tag: &str) {
        STORE.with(|store| {
            store.borrow_mut().remove(tag);
        });
    }

    fn stat(tag: &str) -> Option<Stat> {
        STORE.with(|store| {
            store.borrow().get(tag).map(|entry| Stat {
                saved_at: entry.saved_at,
                metadata: entry.metadata.clone(),
            })
        })
    }
}
