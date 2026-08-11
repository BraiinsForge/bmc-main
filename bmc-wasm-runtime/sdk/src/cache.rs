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

//! Per-instance flash asset cache for WASM widgets.
//!
//! The host curries a per-widget-instance bucket;
//! the widget stores entries under its own opaque tags,
//! and they survive dormancy and restart.
//!
//! The host owns file I/O and the `saved_at` freshness stamp;
//! widgets call `put` / `read_bytes` / `evict`.
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

// Host function imports
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_cache_put(
        tag_ptr: *const u8,
        tag_len: u32,
        meta_ptr: *const u8,
        meta_len: u32,
        bytes_ptr: *const u8,
        bytes_len: u32,
    );
    fn host_cache_get(tag_ptr: *const u8, tag_len: u32, out_ptr: *mut u8, out_cap: u32) -> i32;
    fn host_cache_stat(tag_ptr: *const u8, tag_len: u32, out_ptr: *mut u8, out_cap: u32) -> i32;
    fn host_cache_evict(tag_ptr: *const u8, tag_len: u32);
}

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
    unsafe {
        host_cache_put(
            tag.as_ptr(),
            tag.len() as u32,
            metadata.as_ptr(),
            metadata.len() as u32,
            bytes.as_ptr(),
            bytes.len() as u32,
        );
    }
}

/// Read the whole entry for `tag` into wasm, or `None` on a miss. Copies the
/// blob across the FFI — for small guest state only; assets use the registrars.
#[must_use]
pub fn read_bytes(tag: &str) -> Option<CachedEntry> {
    let len = unsafe { host_cache_get(tag.as_ptr(), tag.len() as u32, core::ptr::null_mut(), 0) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let written =
        unsafe { host_cache_get(tag.as_ptr(), tag.len() as u32, buf.as_mut_ptr(), len as u32) };
    if written < 0 {
        return None;
    }
    decode_record(&buf)
}

/// Evict the entry for `tag`.
pub fn evict(tag: &str) {
    unsafe {
        host_cache_evict(tag.as_ptr(), tag.len() as u32);
    }
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
    let len = unsafe { host_cache_stat(tag.as_ptr(), tag.len() as u32, core::ptr::null_mut(), 0) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let written =
        unsafe { host_cache_stat(tag.as_ptr(), tag.len() as u32, buf.as_mut_ptr(), len as u32) };
    if written < 0 {
        return None;
    }
    let saved_at = u64::from_le_bytes(buf.get(0..8)?.try_into().ok()?);
    Some(Stat {
        saved_at,
        metadata: buf.get(8..)?.to_vec(),
    })
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

// Split the host record `[saved_at u64 | meta_len u32 | metadata | bytes]`.
fn decode_record(buf: &[u8]) -> Option<CachedEntry> {
    let saved_at = u64::from_le_bytes(buf.get(0..8)?.try_into().ok()?);
    let meta_len = u32::from_le_bytes(buf.get(8..12)?.try_into().ok()?) as usize;
    let meta_end = 12 + meta_len;
    Some(CachedEntry {
        saved_at,
        metadata: buf.get(12..meta_end)?.to_vec(),
        bytes: buf.get(meta_end..)?.to_vec(),
    })
}
