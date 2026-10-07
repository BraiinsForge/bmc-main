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

//! How a widget reads XML: the host answers for a wasm guest,
//! and the native SDK reads the same way through this index.

use std::collections::HashMap;

/// Owned lookup data for one parsed XML document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmlDocumentIndex {
    first_text_by_local_name: HashMap<String, String>,
    first_attrs_by_local_name: HashMap<String, HashMap<String, String>>,
}

impl XmlDocumentIndex {
    /// Parse XML once and build the owned lookup index;
    /// `None` for bytes that are not UTF-8 or not well-formed XML.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let xml = core::str::from_utf8(bytes).ok()?;
        let doc = roxmltree::Document::parse(xml).ok()?;
        Some(Self::from_document(&doc))
    }

    /// Resolve a supported XML path to its text.
    #[must_use]
    pub fn get_str(&self, path: &str) -> Option<&str> {
        match parse_xml_lookup_path(path)? {
            XmlLookupPath::Text(local_name) => self
                .first_text_by_local_name
                .get(local_name)
                .map(String::as_str),
            XmlLookupPath::Attribute {
                local_name,
                attr_name,
            } => self
                .first_attrs_by_local_name
                .get(local_name)
                .and_then(|attrs| attrs.get(attr_name))
                .map(String::as_str),
        }
    }

    /// The text at the path read as a number.
    /// `None` for a `NaN`, which the host's wire uses to mean absent.
    #[must_use]
    pub fn get_f64(&self, path: &str) -> Option<f64> {
        self.get_str(path)?
            .parse::<f64>()
            .ok()
            .filter(|value| !value.is_nan())
    }

    fn from_document(doc: &roxmltree::Document<'_>) -> Self {
        let mut first_text_by_local_name = HashMap::new();
        let mut first_attrs_by_local_name = HashMap::new();

        for node in doc.descendants().filter(roxmltree::Node::is_element) {
            let local_name = node.tag_name().name().to_owned();

            first_attrs_by_local_name
                .entry(local_name.clone())
                .or_insert_with(|| {
                    node.attributes()
                        .map(|attr| (attr.name().to_owned(), attr.value().to_owned()))
                        .collect()
                });

            if let Some(text) = extract_text_children(node) {
                first_text_by_local_name.entry(local_name).or_insert(text);
            }
        }

        Self {
            first_text_by_local_name,
            first_attrs_by_local_name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XmlLookupPath<'a> {
    Text(&'a str),
    Attribute {
        local_name: &'a str,
        attr_name: &'a str,
    },
}

fn parse_xml_lookup_path(path: &str) -> Option<XmlLookupPath<'_>> {
    let path = path.strip_prefix("//")?;

    if let Some((element_part, attr_name)) = path.split_once("/@") {
        return Some(XmlLookupPath::Attribute {
            local_name: strip_namespace_prefix(element_part),
            attr_name,
        });
    }

    Some(XmlLookupPath::Text(strip_namespace_prefix(path)))
}

fn strip_namespace_prefix(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn extract_text_children(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let text: String = node
        .children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|child| child.text())
        .collect();

    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::XmlDocumentIndex;

    const FEED_XML: &[u8] = br#"
        <rss xmlns:dc="http://purl.org/dc/elements/1.1/">
            <channel>
                <item>
                    <title></title>
                    <dc:title>Launch</dc:title>
                    <res protocolInfo="http-get:*:audio/mpeg:*" duration="00:01:02" />
                    <ttl>15</ttl>
                    <ratio>NaN</ratio>
                    <title>Fallback</title>
                </item>
            </channel>
        </rss>
    "#;

    fn feed() -> XmlDocumentIndex {
        XmlDocumentIndex::parse(FEED_XML).expect("BUG: test XML should build an index")
    }

    fn lookup_text<'a>(index: &'a XmlDocumentIndex, local_name: &str) -> Option<&'a str> {
        index
            .first_text_by_local_name
            .get(local_name)
            .map(String::as_str)
    }

    fn lookup_attr<'a>(
        index: &'a XmlDocumentIndex,
        local_name: &str,
        attr_name: &str,
    ) -> Option<&'a str> {
        index
            .first_attrs_by_local_name
            .get(local_name)
            .and_then(|attrs| attrs.get(attr_name))
            .map(String::as_str)
    }

    #[test]
    fn xml_index_matches_text_lookup_semantics() {
        let index = feed();

        assert_eq!(index.get_str("//title"), Some("Launch"));
        assert_eq!(index.get_str("//dc:title"), Some("Launch"));
        assert_eq!(index.get_str("//missing"), None);
        assert_eq!(lookup_text(&index, "title"), Some("Launch"));
        assert_eq!(lookup_text(&index, "ttl"), Some("15"));
    }

    #[test]
    fn xml_index_matches_attribute_lookup_semantics() {
        let index = feed();

        assert_eq!(index.get_str("//res/@duration"), Some("00:01:02"));
        assert_eq!(lookup_attr(&index, "res", "duration"), Some("00:01:02"));
        assert_eq!(
            lookup_attr(&index, "res", "protocolInfo"),
            Some("http-get:*:audio/mpeg:*")
        );
        assert_eq!(index.get_str("//res/@missing"), None);
        assert_eq!(lookup_attr(&index, "res", "missing"), None);
    }

    #[test]
    fn xml_index_uses_first_matching_element_for_attributes() {
        let xml = br#"
            <root>
                <entry />
                <entry duration="10" />
            </root>
        "#;

        let index = XmlDocumentIndex::parse(xml).expect("BUG: test XML should build an index");

        assert_eq!(index.get_str("//entry/@duration"), None);
        assert_eq!(lookup_attr(&index, "entry", "duration"), None);
    }

    #[test]
    fn xml_index_rejects_invalid_xml() {
        assert_eq!(XmlDocumentIndex::parse(b"<root>"), None);
    }

    #[test]
    fn xml_index_rejects_bytes_that_are_not_utf8() {
        assert_eq!(XmlDocumentIndex::parse(b"<root>\xff</root>"), None);
    }

    #[test]
    fn a_number_reads_only_from_numeric_text() {
        let index = feed();

        assert_eq!(index.get_f64("//ttl"), Some(15.0));
        assert_eq!(index.get_f64("//title"), None, "text is no number");
        assert_eq!(index.get_f64("//ratio"), None, "NaN reads as absent");
    }
}
