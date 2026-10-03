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

//! Per-instance asset cache for WASM widgets.
//!
//! On the device the host curries a per-widget-instance flash bucket;
//! the widget stores entries under its own opaque tags,
//! and they survive dormancy and restart.
//! The host owns file I/O and the `saved_at` freshness stamp;
//! widgets call `put` / `read_bytes` / `evict`.
//!
//! On native builds the store is in-memory and per-thread,
//! so the storybook and tests can seed and read entries without a host.
//! Nothing persists — callers reseed, which the storybook's
//! every-frame story reruns already do.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::cache;
//!
//! cache::put("logo@480", b"w=480;h=480", &rgba);
//! if let Some(entry) = cache::read_bytes("logo@480") {
//!     redraw(&entry.bytes);
//! }
//! ```

trait CacheStoreProvider {
    fn put(tag: &str, metadata: &[u8], bytes: &[u8]);
    fn read_bytes(tag: &str) -> Option<CachedEntry>;
    fn evict(tag: &str);
    fn stat(tag: &str) -> Option<Stat>;
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

/// A cached entry: opaque caller metadata, the payload bytes, and the host's
/// `saved_at` stamp (UTC epoch milliseconds, from the injected clock).
#[derive(Debug, Clone)]
pub struct CachedEntry {
    pub saved_at: u64,
    pub metadata: Vec<u8>,
    pub bytes: Vec<u8>,
}

/// Store `bytes` under `tag` with opaque `metadata`, overwriting any prior entry.
pub fn put(tag: &str, metadata: &[u8], bytes: &[u8]) {
    Platform::put(tag, metadata, bytes);
}

/// Read the whole entry for `tag` into wasm, or `None` on a miss. Copies the
/// blob across the FFI — for small guest state only; assets use the registrars.
#[must_use]
pub fn read_bytes(tag: &str) -> Option<CachedEntry> {
    Platform::read_bytes(tag)
}

/// Evict the entry for `tag`.
pub fn evict(tag: &str) {
    Platform::evict(tag);
}

/// An entry's freshness header: the host `saved_at` stamp plus the opaque
/// caller metadata, fetched without pulling the payload bytes into wasm.
#[derive(Debug, Clone)]
pub struct Stat {
    pub saved_at: u64,
    pub metadata: Vec<u8>,
}

/// Peek the entry for `tag` — `saved_at` + metadata only, no payload. `None` on a miss.
#[must_use]
pub fn stat(tag: &str) -> Option<Stat> {
    Platform::stat(tag)
}

/// A lazily-resolved, host-side reference to this widget's cache entry for `tag`.
/// Pass it to a type registrar (e.g. `assets::register_image`) which resolves
/// the bytes host-side — they never enter wasm.
#[derive(Debug, Clone, Copy)]
pub struct CacheSource<'a> {
    tag: &'a str,
}

impl CacheSource<'_> {
    #[must_use]
    pub fn tag(&self) -> &str {
        self.tag
    }
}

/// Build a [`CacheSource`] for `tag` — nothing is read until a registrar consumes it.
#[must_use]
pub fn lazy_get(tag: &str) -> CacheSource<'_> {
    CacheSource { tag }
}

#[cfg(test)]
mod tests {
    use super::{evict, put, read_bytes, stat};

    #[test]
    fn an_entry_round_trips() {
        put("trip", b"meta", b"payload");
        let entry = read_bytes("trip").expect("BUG: just stored");
        assert_eq!(entry.metadata, b"meta");
        assert_eq!(entry.bytes, b"payload");
        assert!(entry.saved_at > 0, "saved_at carries the wall clock");
    }

    #[test]
    fn stat_skips_the_payload() {
        put("peek", b"header", b"heavy payload");
        let stat = stat("peek").expect("BUG: just stored");
        assert_eq!(stat.metadata, b"header");
    }

    #[test]
    fn put_overwrites() {
        put("twice", b"old", b"old");
        put("twice", b"new", b"new");
        assert_eq!(read_bytes("twice").expect("BUG: just stored").bytes, b"new");
    }

    #[test]
    fn evict_removes_and_misses_read_as_none() {
        put("gone", b"", b"x");
        evict("gone");
        assert!(read_bytes("gone").is_none());
        assert!(stat("never-stored").is_none());
    }
}
