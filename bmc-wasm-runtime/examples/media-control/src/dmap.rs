// Copyright (C) 2026  Braiins Systems s.r.o.

//! DMAP (Digital Media Access Protocol) binary parser.
//!
//! DMAP is a TLV format used by iTunes/Music.app (DACP protocol):
//! `[4-byte ASCII tag][4-byte BE u32 length][data]`
//!
//! Only the ~25 content codes relevant to DACP media control are hardcoded.

/// Parsed DMAP node.
#[derive(Debug)]
pub struct Node<'a> {
    pub tag: [u8; 4],
    pub value: Value<'a>,
}

/// DMAP value types.
#[derive(Debug)]
#[allow(dead_code)]
pub enum Value<'a> {
    U8(u8),
    U32(u32),
    U64(u64),
    Str(&'a str),
    Data(&'a [u8]),
    Container(Vec<Node<'a>>),
}

/// Expected type for a content code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentType {
    U8,
    U32,
    U64,
    Str,
    Container,
    Data,
}

/// Look up the expected type for a DMAP content code.
///
/// Only DACP-relevant codes are listed; unknown tags default to `Data`.
fn content_type(tag: [u8; 4]) -> ContentType {
    match &tag {
        // Containers
        b"mlog" | b"mccr" | b"cmst" | b"mdcl" | b"cmpa" | b"mlcl" | b"mlit" | b"msrv" | b"aply"
        | b"casp" | b"cmgt" => ContentType::Container,
        // Strings
        b"cann" | b"cana" | b"canl" | b"cang" | b"minm" | b"cmnm" | b"cmty" => ContentType::Str,
        // u32
        b"mstt" | b"cmvo" | b"cast" | b"cant" | b"mlid" | b"miid" | b"cmsr" | b"ceGS" => {
            ContentType::U32
        }
        // u8
        b"caps" | b"cash" | b"carp" | b"caas" | b"cavc" => ContentType::U8,
        // u64
        b"cmpg" => ContentType::U64,
        // Everything else
        _ => ContentType::Data,
    }
}

/// Parse a DMAP binary blob into a list of nodes.
pub fn parse(data: &[u8]) -> Vec<Node<'_>> {
    let mut nodes = Vec::new();
    let mut pos = 0;
    while pos + 8 <= data.len() {
        let tag = [data[pos], data[pos + 1], data[pos + 2], data[pos + 3]];
        let len = u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
            as usize;
        pos += 8;
        if pos + len > data.len() {
            break;
        }
        let chunk = &data[pos..pos + len];
        let value = match content_type(tag) {
            ContentType::Container => Value::Container(parse(chunk)),
            ContentType::Str => Value::Str(core::str::from_utf8(chunk).unwrap_or("")),
            ContentType::U8 if len >= 1 => Value::U8(chunk[0]),
            ContentType::U32 if len >= 4 => {
                Value::U32(u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            }
            ContentType::U64 if len >= 8 => Value::U64(u64::from_be_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ])),
            _ => Value::Data(chunk),
        };
        nodes.push(Node { tag, value });
        pos += len;
    }
    nodes
}

/// Find the first node with a matching tag (recursive descent into containers).
pub fn find<'a>(nodes: &'a [Node<'a>], tag: [u8; 4]) -> Option<&'a Node<'a>> {
    for node in nodes {
        if node.tag == tag {
            return Some(node);
        }
        if let Value::Container(ref children) = node.value {
            if let Some(found) = find(children, tag) {
                return Some(found);
            }
        }
    }
    None
}

/// Extract a `u32` value by tag.
pub fn find_u32(nodes: &[Node<'_>], tag: [u8; 4]) -> Option<u32> {
    find(nodes, tag).and_then(|n| match n.value {
        Value::U32(v) => Some(v),
        Value::U8(v) => Some(u32::from(v)),
        _ => None,
    })
}

/// Extract a `u64` value by tag.
#[allow(dead_code)]
pub fn find_u64(nodes: &[Node<'_>], tag: [u8; 4]) -> Option<u64> {
    find(nodes, tag).and_then(|n| match n.value {
        Value::U64(v) => Some(v),
        Value::U32(v) => Some(u64::from(v)),
        _ => None,
    })
}

/// Extract a string value by tag.
pub fn find_str<'a>(nodes: &'a [Node<'a>], tag: [u8; 4]) -> Option<&'a str> {
    find(nodes, tag).and_then(|n| match n.value {
        Value::Str(s) => Some(s),
        _ => None,
    })
}

/// Extract a `u8` value by tag.
pub fn find_u8(nodes: &[Node<'_>], tag: [u8; 4]) -> Option<u8> {
    find(nodes, tag).and_then(|n| match n.value {
        Value::U8(v) => Some(v),
        _ => None,
    })
}
