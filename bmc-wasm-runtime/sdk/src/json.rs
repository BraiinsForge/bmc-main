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

//! Host-side JSON parsing with JSON Pointer queries.
//!
//! The host parses JSON using `serde_json` at native speed. The widget
//! queries fields via [RFC 6901](https://datatracker.ietf.org/doc/html/rfc6901)
//! JSON Pointer paths, keeping WASM binaries small.
//!
//! # Example
//!
//! ```ignore
//! let doc = JsonDoc::parse(body);
//! let name = doc.str("/results/0/name").unwrap_or_default();
//! let count = doc.i64("/results/0/count").unwrap_or(0);
//! let ratio = doc.f64("/ratio").unwrap_or(1.0);
//! let active = doc.bool("/active").unwrap_or(false);
//! // doc is freed automatically on Drop
//! ```

use bmc_wasm_protocol::JsonId;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_json_parse(body_ptr: *const u8, body_len: u32) -> u32;
    fn host_json_get_str(
        doc_id: u32,
        path_ptr: *const u8,
        path_len: u32,
        out_ptr: *mut u8,
        out_len: u32,
    ) -> i32;
    fn host_json_get_i64(doc_id: u32, path_ptr: *const u8, path_len: u32) -> i64;
    fn host_json_get_f64(doc_id: u32, path_ptr: *const u8, path_len: u32) -> f64;
    fn host_json_get_bool(doc_id: u32, path_ptr: *const u8, path_len: u32) -> i32;
    fn host_json_free(doc_id: u32);
}

/// Handle to a host-side parsed JSON document.
///
/// Query fields using JSON Pointer paths (RFC 6901).
/// The document is freed on the host when this handle is dropped.
/// `None` represents a parse failure — accessor methods short-circuit.
#[derive(Debug)]
pub struct JsonDoc(Option<JsonId>);

impl JsonDoc {
    /// Parse a JSON byte slice on the host side.
    #[must_use]
    pub fn parse(data: &[u8]) -> Self {
        Self(JsonId::from_wire(unsafe {
            host_json_parse(data.as_ptr(), data.len() as u32)
        }))
    }

    /// Whether the document was parsed successfully.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.0.is_some()
    }

    /// Get a string value at the given JSON Pointer path.
    ///
    /// Returns `None` if the path doesn't exist or the value isn't a string.
    #[must_use]
    pub fn str(&self, path: &str) -> Option<String> {
        let id = self.0?.to_wire();
        // First call with a reasonable buffer to get the actual length
        let mut buf = vec![0_u8; 256];
        let actual = unsafe {
            host_json_get_str(
                id,
                path.as_ptr(),
                path.len() as u32,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        if actual < 0 {
            return None; // not found or wrong type
        }
        let actual = actual as usize;
        if actual <= buf.len() {
            buf.truncate(actual);
            String::from_utf8(buf).ok()
        } else {
            // Retry with exact size
            let mut buf = vec![0u8; actual];
            let len = unsafe {
                host_json_get_str(
                    id,
                    path.as_ptr(),
                    path.len() as u32,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                )
            };
            if len < 0 {
                return None;
            }
            buf.truncate(len as usize);
            String::from_utf8(buf).ok()
        }
    }

    /// Get an i64 value at the given JSON Pointer path.
    ///
    /// Returns `None` if the path doesn't exist. Returns 0 for non-numeric values.
    #[must_use]
    pub fn i64(&self, path: &str) -> Option<i64> {
        let id = self.0?.to_wire();
        let val = unsafe { host_json_get_i64(id, path.as_ptr(), path.len() as u32) };
        // We use i64::MIN as sentinel for "not found"
        if val == i64::MIN { None } else { Some(val) }
    }

    /// Get an f64 value at the given JSON Pointer path.
    ///
    /// Returns `None` if the path doesn't exist.
    #[must_use]
    pub fn f64(&self, path: &str) -> Option<f64> {
        let id = self.0?.to_wire();
        let val = unsafe { host_json_get_f64(id, path.as_ptr(), path.len() as u32) };
        if val.is_nan() { None } else { Some(val) }
    }

    /// Get a boolean value at the given JSON Pointer path.
    ///
    /// Returns `None` if the path doesn't exist or isn't a boolean.
    #[must_use]
    pub fn bool(&self, path: &str) -> Option<bool> {
        let id = self.0?.to_wire();
        match unsafe { host_json_get_bool(id, path.as_ptr(), path.len() as u32) } {
            0 => Some(false),
            1 => Some(true),
            _ => None, // -1 = missing
        }
    }
}

impl Drop for JsonDoc {
    fn drop(&mut self) {
        if let Some(id) = self.0 {
            unsafe { host_json_free(id.to_wire()) }
        }
    }
}
