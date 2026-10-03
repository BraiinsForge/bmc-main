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

use super::{CacheStoreProvider, CachedEntry, Stat};

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

pub(super) struct Host;

impl CacheStoreProvider for Host {
    fn put(tag: &str, metadata: &[u8], bytes: &[u8]) {
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

    fn read_bytes(tag: &str) -> Option<CachedEntry> {
        let len =
            unsafe { host_cache_get(tag.as_ptr(), tag.len() as u32, core::ptr::null_mut(), 0) };
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

    fn evict(tag: &str) {
        unsafe {
            host_cache_evict(tag.as_ptr(), tag.len() as u32);
        }
    }

    fn stat(tag: &str) -> Option<Stat> {
        let len =
            unsafe { host_cache_stat(tag.as_ptr(), tag.len() as u32, core::ptr::null_mut(), 0) };
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let written = unsafe {
            host_cache_stat(tag.as_ptr(), tag.len() as u32, buf.as_mut_ptr(), len as u32)
        };
        if written < 0 {
            return None;
        }
        let saved_at = u64::from_le_bytes(buf.get(0..8)?.try_into().ok()?);
        Some(Stat {
            saved_at,
            metadata: buf.get(8..)?.to_vec(),
        })
    }
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
