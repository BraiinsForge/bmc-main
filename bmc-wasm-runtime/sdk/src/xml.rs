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
//! Off the device the same index runs in-process.
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

trait XmlDocProvider {
    type Doc: core::fmt::Debug;

    fn parse(data: &[u8]) -> Option<Self::Doc>;
    fn str(doc: &Self::Doc, path: &str) -> Option<String>;
    fn f64(doc: &Self::Doc, path: &str) -> Option<f64>;
    fn release(doc: &Self::Doc);
}

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

/// Handle to a host-side parsed XML document.
///
/// Query fields using simplified XPath-like paths (see module docs).
/// The document is freed on the host when this handle is dropped.
/// `None` represents a parse failure — accessor methods short-circuit.
#[derive(Debug)]
pub struct XmlDoc(Option<<Platform as XmlDocProvider>::Doc>);

impl XmlDoc {
    /// Parse an XML byte slice on the host side.
    #[must_use]
    pub fn parse(data: &[u8]) -> Self {
        Self(Platform::parse(data))
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
        Platform::str(self.0.as_ref()?, path)
    }

    /// Get an f64 value at the given path (parses the text content as a number).
    ///
    /// Returns `None` if the path doesn't match or the text is not a valid number.
    #[must_use]
    pub fn f64(&self, path: &str) -> Option<f64> {
        Platform::f64(self.0.as_ref()?, path)
    }
}

impl Drop for XmlDoc {
    fn drop(&mut self) {
        if let Some(doc) = &self.0 {
            Platform::release(doc);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
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

    pub(super) struct Host;

    impl super::XmlDocProvider for Host {
        type Doc = XmlId;

        fn parse(data: &[u8]) -> Option<XmlId> {
            XmlId::from_wire(unsafe { host_xml_parse(data.as_ptr(), crate::ffi_u32(data.len())) })
        }

        fn str(doc: &XmlId, path: &str) -> Option<String> {
            let id = doc.to_wire();
            // First call with a reasonable buffer
            let mut buf = vec![0_u8; 256];
            let actual = unsafe {
                host_xml_get_str(
                    id,
                    path.as_ptr(),
                    crate::ffi_u32(path.len()),
                    buf.as_mut_ptr(),
                    crate::ffi_u32(buf.len()),
                )
            };
            let actual = usize::try_from(actual).ok()?;
            if actual <= buf.len() {
                buf.truncate(actual);
                String::from_utf8(buf).ok()
            } else {
                // Retry with exact size
                let mut buf = vec![0_u8; actual];
                let len = unsafe {
                    host_xml_get_str(
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

        fn f64(doc: &XmlId, path: &str) -> Option<f64> {
            let val = unsafe {
                host_xml_get_f64(doc.to_wire(), path.as_ptr(), crate::ffi_u32(path.len()))
            };
            if val.is_nan() { None } else { Some(val) }
        }

        fn release(doc: &XmlId) {
            unsafe { host_xml_free(doc.to_wire()) }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_wasm_protocol::xml::XmlDocumentIndex;

    pub(super) struct Host;

    impl super::XmlDocProvider for Host {
        type Doc = XmlDocumentIndex;

        fn parse(data: &[u8]) -> Option<XmlDocumentIndex> {
            XmlDocumentIndex::parse(data)
        }

        fn str(doc: &XmlDocumentIndex, path: &str) -> Option<String> {
            doc.get_str(path).map(str::to_owned)
        }

        fn f64(doc: &XmlDocumentIndex, path: &str) -> Option<f64> {
            doc.get_f64(path)
        }

        fn release(_doc: &XmlDocumentIndex) {}
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::XmlDoc;

    #[test]
    fn a_malformed_document_answers_nothing() {
        let doc = XmlDoc::parse(b"<root>");
        assert!(!doc.is_valid());
        assert_eq!(doc.str("//root"), None);
    }

    #[test]
    fn a_document_reads_off_the_device_through_the_shared_index() {
        let doc = XmlDoc::parse(
            br#"<item><dc:title xmlns:dc="dc">Launch</dc:title><ttl>15</ttl></item>"#,
        );
        assert_eq!(doc.str("//title").as_deref(), Some("Launch"));
        assert_eq!(doc.f64("//ttl"), Some(15.0));
    }
}
