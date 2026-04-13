// Copyright (C) 2026  Braiins Systems s.r.o.

//! Owned XML lookup index for the WASM host API.

use std::collections::HashMap;

use anyhow::Result;

/// Owned lookup data for one parsed XML document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct XmlDocumentIndex {
    first_text_by_local_name: HashMap<String, String>,
    first_attrs_by_local_name: HashMap<String, HashMap<String, String>>,
}

impl XmlDocumentIndex {
    /// Parse XML once and build the owned lookup index used by the host API.
    pub(crate) fn from_xml(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml)?;
        Ok(Self::from_document(&doc))
    }

    /// Resolve a supported host XML path to an indexed string result.
    #[must_use]
    pub(crate) fn get_str(&self, path: &str) -> Option<&str> {
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

    fn from_document(doc: &roxmltree::Document<'_>) -> Self {
        let mut first_text_by_local_name = HashMap::new();
        let mut first_attrs_by_local_name = HashMap::new();

        for node in doc.descendants().filter(roxmltree::Node::is_element) {
            let local_name = node.tag_name().name().to_owned();

            first_attrs_by_local_name.entry(local_name.clone()).or_insert_with(|| {
                node.attributes()
                    .map(|attr| (attr.name().to_owned(), attr.value().to_owned()))
                    .collect()
            });

            if let Some(text) = extract_text_children(node) {
                first_text_by_local_name
                    .entry(local_name)
                    .or_insert(text);
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
    Attribute { local_name: &'a str, attr_name: &'a str },
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

    const FEED_XML: &str = r#"
        <rss xmlns:dc="http://purl.org/dc/elements/1.1/">
            <channel>
                <item>
                    <title></title>
                    <dc:title>Launch</dc:title>
                    <res protocolInfo="http-get:*:audio/mpeg:*" duration="00:01:02" />
                    <ttl>15</ttl>
                    <title>Fallback</title>
                </item>
            </channel>
        </rss>
    "#;

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
        let index =
            XmlDocumentIndex::from_xml(FEED_XML).expect("BUG: test XML should build an index");

        assert_eq!(index.get_str("//title"), Some("Launch"));
        assert_eq!(index.get_str("//dc:title"), Some("Launch"));
        assert_eq!(index.get_str("//missing"), None);
        assert_eq!(lookup_text(&index, "title"), Some("Launch"));
        assert_eq!(lookup_text(&index, "ttl"), Some("15"));
    }

    #[test]
    fn xml_index_matches_attribute_lookup_semantics() {
        let index =
            XmlDocumentIndex::from_xml(FEED_XML).expect("BUG: test XML should build an index");

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
        let xml = r#"
            <root>
                <entry />
                <entry duration="10" />
            </root>
        "#;

        let index = XmlDocumentIndex::from_xml(xml).expect("BUG: test XML should build an index");

        assert_eq!(index.get_str("//entry/@duration"), None);
        assert_eq!(lookup_attr(&index, "entry", "duration"), None);
    }

    #[test]
    fn xml_index_rejects_invalid_xml() {
        assert!(XmlDocumentIndex::from_xml("<root>").is_err());
    }
}
