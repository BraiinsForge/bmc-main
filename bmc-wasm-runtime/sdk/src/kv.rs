// Copyright (C) 2026  Braiins Systems s.r.o.

//! Key-value persistence for WASM widgets.
//!
//! Provides simple key-value storage that persists across widget restarts
//! and hot-reloads. The host manages file I/O; WASM widgets just call
//! `kv_set` / `kv_get` / `kv_delete`.
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

// Host function imports
unsafe extern "C" {
    fn host_kv_set(key_ptr: *const u8, key_len: u32, val_ptr: *const u8, val_len: u32);
    fn host_kv_get(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8, out_cap: u32) -> i32;
    fn host_kv_delete(key_ptr: *const u8, key_len: u32);
}

/// Store a value for a key. Overwrites any existing value.
pub fn set(key: &str, value: &[u8]) {
    unsafe {
        host_kv_set(
            key.as_ptr(),
            key.len() as u32,
            value.as_ptr(),
            value.len() as u32,
        );
    }
}

/// Retrieve the value for a key. Returns `None` if not found.
///
/// Uses a two-call pattern: first call gets the length, second call
/// reads the data into an allocated buffer.
#[must_use]
pub fn get(key: &str) -> Option<Vec<u8>> {
    let len = unsafe { host_kv_get(key.as_ptr(), key.len() as u32, core::ptr::null_mut(), 0) };
    if len < 0 {
        return None;
    }
    if len == 0 {
        return Some(Vec::new());
    }
    let mut buf = vec![0u8; len as usize];
    let len2 = unsafe { host_kv_get(key.as_ptr(), key.len() as u32, buf.as_mut_ptr(), len as u32) };
    if len2 < 0 {
        return None;
    }
    Some(buf)
}

/// Retrieve a string value for a key. Returns `None` if not found or not valid UTF-8.
#[must_use]
pub fn get_string(key: &str) -> Option<String> {
    get(key).and_then(|b| String::from_utf8(b).ok())
}

/// Delete a key and its value.
pub fn delete(key: &str) {
    unsafe {
        host_kv_delete(key.as_ptr(), key.len() as u32);
    }
}
