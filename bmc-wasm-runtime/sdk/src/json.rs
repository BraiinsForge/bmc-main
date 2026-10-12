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
//! Off the device the same readers run in-process.
//!
//! # Example
//!
//! ```ignore
//! let doc = JsonDoc::parse(body);
//! let name = doc.str("/results/0/name"); // `None` unless a string
//! let count = doc.i64("/results/0/count"); // `None` unless an integer
//! let ratio = doc.f64("/ratio").unwrap_or(1.0); // a neutral fallback, where one exists
//! let active = doc.bool("/active").unwrap_or(false);
//! // doc is freed automatically on Drop
//! ```

pub use bmc_wasm_protocol::json::JsonKind;

trait JsonDocProvider {
    type Doc: core::fmt::Debug;

    fn parse(data: &[u8]) -> Option<Self::Doc>;
    fn str(doc: &Self::Doc, path: &str) -> Option<String>;
    fn i64(doc: &Self::Doc, path: &str) -> Option<i64>;
    fn f64(doc: &Self::Doc, path: &str) -> Option<f64>;
    fn bool(doc: &Self::Doc, path: &str) -> Option<bool>;
    fn kind(doc: &Self::Doc, path: &str) -> Option<JsonKind>;
    fn len(doc: &Self::Doc, path: &str) -> Option<usize>;
    fn release(doc: &Self::Doc);
}

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

/// Handle to a host-side parsed JSON document.
///
/// Query fields using JSON Pointer paths (RFC 6901).
/// The document is freed on the host when this handle is dropped.
/// `None` represents a parse failure — accessor methods short-circuit.
#[derive(Debug)]
pub struct JsonDoc(Option<<Platform as JsonDocProvider>::Doc>);

impl JsonDoc {
    /// Parse a JSON byte slice on the host side.
    #[must_use]
    pub fn parse(data: &[u8]) -> Self {
        Self(Platform::parse(data))
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
        Platform::str(self.0.as_ref()?, path)
    }

    /// Get an i64 value at the given JSON Pointer path.
    ///
    /// Returns `None` if the path doesn't exist or doesn't hold an integer that fits an `i64`;
    /// on the device a literal `i64::MIN` is `None` too, since the host spells absence with it.
    /// A decimal such as `325.0` is `None` as well; read it with [`Self::f64`].
    #[must_use]
    pub fn i64(&self, path: &str) -> Option<i64> {
        Platform::i64(self.0.as_ref()?, path)
    }

    /// Get an f64 value at the given JSON Pointer path.
    ///
    /// Returns `None` if the path doesn't exist or holds no number.
    #[must_use]
    pub fn f64(&self, path: &str) -> Option<f64> {
        Platform::f64(self.0.as_ref()?, path)
    }

    /// Get a boolean value at the given JSON Pointer path.
    ///
    /// Returns `None` if the path doesn't exist or isn't a boolean.
    #[must_use]
    pub fn bool(&self, path: &str) -> Option<bool> {
        Platform::bool(self.0.as_ref()?, path)
    }

    /// What the value at the path is, or `None` where there is none.
    ///
    /// This is how absence is told apart from emptiness:
    ///  - an empty array answers `Some(Array)`,
    ///  - a missing key answers `None` — where the typed getters answer `None` for both.
    #[must_use]
    pub fn kind(&self, path: &str) -> Option<JsonKind> {
        Platform::kind(self.0.as_ref()?, path)
    }

    /// Element count of the array or object at the path,
    /// `None` for anything that has none.
    #[must_use]
    pub fn len(&self, path: &str) -> Option<usize> {
        Platform::len(self.0.as_ref()?, path)
    }
}

impl Drop for JsonDoc {
    fn drop(&mut self) {
        if let Some(doc) = &self.0 {
            Platform::release(doc);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_wasm_protocol::JsonId;

    use super::JsonKind;

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
        fn host_json_kind(doc_id: u32, path_ptr: *const u8, path_len: u32) -> i32;
        fn host_json_len(doc_id: u32, path_ptr: *const u8, path_len: u32) -> i32;
        fn host_json_free(doc_id: u32);
    }

    pub(super) struct Host;

    impl super::JsonDocProvider for Host {
        type Doc = JsonId;

        fn parse(data: &[u8]) -> Option<JsonId> {
            JsonId::from_wire(unsafe { host_json_parse(data.as_ptr(), crate::ffi_u32(data.len())) })
        }

        fn str(doc: &JsonId, path: &str) -> Option<String> {
            let id = doc.to_wire();
            // First call with a reasonable buffer to get the actual length
            let mut buf = vec![0_u8; 256];
            let actual = unsafe {
                host_json_get_str(
                    id,
                    path.as_ptr(),
                    crate::ffi_u32(path.len()),
                    buf.as_mut_ptr(),
                    crate::ffi_u32(buf.len()),
                )
            };
            // Negative: not found, or the wrong type.
            let actual = usize::try_from(actual).ok()?;
            if actual <= buf.len() {
                buf.truncate(actual);
                String::from_utf8(buf).ok()
            } else {
                // Retry with exact size
                let mut buf = vec![0_u8; actual];
                let len = unsafe {
                    host_json_get_str(
                        id,
                        path.as_ptr(),
                        crate::ffi_u32(path.len()),
                        buf.as_mut_ptr(),
                        crate::ffi_u32(buf.len()),
                    )
                };
                buf.truncate(usize::try_from(len).ok()?);
                String::from_utf8(buf).ok()
            }
        }

        fn i64(doc: &JsonId, path: &str) -> Option<i64> {
            let val = unsafe {
                host_json_get_i64(doc.to_wire(), path.as_ptr(), crate::ffi_u32(path.len()))
            };
            // The host answers `i64::MIN` for "not found",
            // so a literal `i64::MIN` reads as `None` here, unlike off the device.
            if val == i64::MIN { None } else { Some(val) }
        }

        fn f64(doc: &JsonId, path: &str) -> Option<f64> {
            let val = unsafe {
                host_json_get_f64(doc.to_wire(), path.as_ptr(), crate::ffi_u32(path.len()))
            };
            if val.is_nan() { None } else { Some(val) }
        }

        fn bool(doc: &JsonId, path: &str) -> Option<bool> {
            match unsafe {
                host_json_get_bool(doc.to_wire(), path.as_ptr(), crate::ffi_u32(path.len()))
            } {
                0 => Some(false),
                1 => Some(true),
                _ => None, // -1 = missing
            }
        }

        fn kind(doc: &JsonId, path: &str) -> Option<JsonKind> {
            JsonKind::from_wire(unsafe {
                host_json_kind(doc.to_wire(), path.as_ptr(), crate::ffi_u32(path.len()))
            })
        }

        fn len(doc: &JsonId, path: &str) -> Option<usize> {
            usize::try_from(unsafe {
                host_json_len(doc.to_wire(), path.as_ptr(), crate::ffi_u32(path.len()))
            })
            .ok()
        }

        fn release(doc: &JsonId) {
            unsafe { host_json_free(doc.to_wire()) }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_wasm_protocol::json::{self, Value};

    use super::JsonKind;

    pub(super) struct Host;

    impl super::JsonDocProvider for Host {
        type Doc = Value;

        fn parse(data: &[u8]) -> Option<Value> {
            json::parse(data)
        }

        fn str(doc: &Value, path: &str) -> Option<String> {
            json::str_at(doc, path).map(str::to_owned)
        }

        fn i64(doc: &Value, path: &str) -> Option<i64> {
            json::i64_at(doc, path)
        }

        fn f64(doc: &Value, path: &str) -> Option<f64> {
            json::f64_at(doc, path)
        }

        fn bool(doc: &Value, path: &str) -> Option<bool> {
            json::bool_at(doc, path)
        }

        fn kind(doc: &Value, path: &str) -> Option<JsonKind> {
            json::kind_at(doc, path)
        }

        fn len(doc: &Value, path: &str) -> Option<usize> {
            json::len_at(doc, path)
        }

        fn release(_doc: &Value) {}
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{JsonDoc, JsonKind};

    #[test]
    fn a_malformed_document_answers_nothing() {
        let doc = JsonDoc::parse(b"{\"unterminated\": ");
        assert!(!doc.is_valid());
        assert_eq!(doc.kind(""), None);
    }

    #[test]
    fn a_document_reads_off_the_device_through_the_shared_readers() {
        let doc =
            JsonDoc::parse(br#"{"name": "s21", "rows": [1, 2], "min": -9223372036854775808}"#);
        assert_eq!(doc.str("/name").as_deref(), Some("s21"));
        assert_eq!(doc.len("/rows"), Some(2));
        assert_eq!(doc.kind("/rows"), Some(JsonKind::Array));
        assert_eq!(
            doc.i64("/min"),
            Some(i64::MIN),
            "off the device `i64::MIN` is a value, not the host's sentinel"
        );
    }
}
