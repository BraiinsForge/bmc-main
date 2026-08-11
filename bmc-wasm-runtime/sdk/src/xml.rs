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

//! Host-side XML document parsing for WASM widgets.
//!
//! Mirrors the [`JsonDoc`](crate::json::JsonDoc) pattern. The host parses XML using
//! `roxmltree` and the WASM side queries values via simplified XPath-like paths.
//!
//! # Path syntax
//!
//! - `//local_name` — text content of the first element with that local name
//!   (namespace-agnostic, e.g. `//title` matches `<dc:title>`)
//! - `//local_name/@attr` — attribute value on the first matching element
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::xml::XmlDoc;
//!
//! let xml = XmlDoc::parse(b"<root><title>Hello</title></root>");
//! assert_eq!(xml.str("//title"), Some("Hello".into()));
//! ```

use bmc_wasm_protocol::XmlId;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_xml_parse(body_ptr: *const u8, body_len: u32) -> u32;
    fn host_xml_get_str(
        doc_id: u32,
        path_ptr: *const u8,
        path_len: u32,
        out_ptr: *mut u8,
        out_len: u32,
    ) -> i32;
    fn host_xml_get_f64(doc_id: u32, path_ptr: *const u8, path_len: u32) -> f64;
    fn host_xml_free(doc_id: u32);
}

/// Handle to a host-side parsed XML document.
///
/// Query fields using simplified XPath-like paths (see module docs).
/// The document is freed on the host when this handle is dropped.
/// `None` represents a parse failure — accessor methods short-circuit.
#[derive(Debug)]
pub struct XmlDoc(Option<XmlId>);

impl XmlDoc {
    /// Parse an XML byte slice on the host side.
    #[must_use]
    pub fn parse(data: &[u8]) -> Self {
        Self(XmlId::from_wire(unsafe {
            host_xml_parse(data.as_ptr(), data.len() as u32)
        }))
    }

    /// Whether the document was parsed successfully.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.0.is_some()
    }

    /// Get a string value at the given path.
    ///
    /// Returns `None` if the path doesn't match any element.
    #[must_use]
    pub fn str(&self, path: &str) -> Option<String> {
        let id = self.0?.to_wire();
        // First call with a reasonable buffer
        let mut buf = vec![0u8; 256];
        let actual = unsafe {
            host_xml_get_str(
                id,
                path.as_ptr(),
                path.len() as u32,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        if actual < 0 {
            return None;
        }
        let actual = actual as usize;
        if actual <= buf.len() {
            buf.truncate(actual);
            String::from_utf8(buf).ok()
        } else {
            // Retry with exact size
            let mut buf = vec![0u8; actual];
            let len = unsafe {
                host_xml_get_str(
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

    /// Get an f64 value at the given path (parses the text content as a number).
    ///
    /// Returns `None` if the path doesn't match or the text is not a valid number.
    #[must_use]
    pub fn f64(&self, path: &str) -> Option<f64> {
        let id = self.0?.to_wire();
        let val = unsafe { host_xml_get_f64(id, path.as_ptr(), path.len() as u32) };
        if val.is_nan() { None } else { Some(val) }
    }
}

impl Drop for XmlDoc {
    fn drop(&mut self) {
        if let Some(id) = self.0 {
            unsafe { host_xml_free(id.to_wire()) }
        }
    }
}
