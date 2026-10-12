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

//! Key-value persistence for WASM widgets.
//!
//! Provides simple key-value storage that persists across widget restarts
//! and hot-reloads. The host manages file I/O; WASM widgets just call
//! `set` / `get` / `delete`.
//!
//! Off-target there is no host to persist the store, so the calls panic.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::kv;
//!
//! // Store a pairing token
//! kv::set("pairing_guid", b"0xABCDEF1234567890");
//!
//! // Retrieve it later (even after hot-reload)
//! if let Some(guid) = kv::get("pairing_guid") {
//!     log_info!("restored guid: {} bytes", guid.len());
//! }
//!
//! // Clean up
//! kv::delete("pairing_guid");
//! ```

/// Store a value for a key. Overwrites any existing value.
pub fn set(key: &str, value: &[u8]) {
    Platform::set(key, value);
}

/// Retrieve the value for a key. Returns `None` if not found.
#[must_use]
pub fn get(key: &str) -> Option<Vec<u8>> {
    Platform::get(key)
}

/// Retrieve a string value for a key. Returns `None` if not found or not valid UTF-8.
#[must_use]
pub fn get_string(key: &str) -> Option<String> {
    get(key).and_then(|b| String::from_utf8(b).ok())
}

/// Delete a key and its value.
pub fn delete(key: &str) {
    Platform::delete(key);
}

trait KvStore {
    fn set(key: &str, value: &[u8]);
    fn get(key: &str) -> Option<Vec<u8>>;
    fn delete(key: &str);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_kv_set(key_ptr: *const u8, key_len: u32, val_ptr: *const u8, val_len: u32);
        fn host_kv_get(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8, out_cap: u32) -> i32;
        fn host_kv_delete(key_ptr: *const u8, key_len: u32);
    }

    pub(super) struct Host;

    impl super::KvStore for Host {
        fn set(key: &str, value: &[u8]) {
            unsafe {
                host_kv_set(
                    key.as_ptr(),
                    crate::ffi_u32(key.len()),
                    value.as_ptr(),
                    crate::ffi_u32(value.len()),
                );
            }
        }

        fn get(key: &str) -> Option<Vec<u8>> {
            let key_len = crate::ffi_u32(key.len());
            let needed = unsafe { host_kv_get(key.as_ptr(), key_len, core::ptr::null_mut(), 0) };
            let len = u32::try_from(needed).ok()?;
            if len == 0 {
                return Some(Vec::new());
            }
            let mut buf = vec![0_u8; len as usize];
            let len2 = unsafe { host_kv_get(key.as_ptr(), key_len, buf.as_mut_ptr(), len) };
            if len2 < 0 {
                return None;
            }
            Some(buf)
        }

        fn delete(key: &str) {
            unsafe {
                host_kv_delete(key.as_ptr(), crate::ffi_u32(key.len()));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    pub(super) struct Host;

    impl super::KvStore for Host {
        fn set(_: &str, _: &[u8]) {
            panic!("BUG: kv::set() called off-target — no host persists the store")
        }

        fn get(_: &str) -> Option<Vec<u8>> {
            panic!("BUG: kv::get() called off-target — no host persists the store")
        }

        fn delete(_: &str) {
            panic!("BUG: kv::delete() called off-target — no host persists the store")
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
