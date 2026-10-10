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

//! Wire format and reader for widget params snapshots.
//!
//! The host encoder (`bmc-wasm-runtime::runtime::imports::params`) packs the layout below
//! and [`Params`] reads it back, so both sides build on one definition of the format.
//! `bmc-wasm-sdk::params` re-exports the reader and adds the host-fed snapshot lifecycle.
//!
//! A [`Params`] is an owned snapshot of every parameter the host delivered
//! for a widget instance. Values are read through the typed accessors
//! ([`Params::get_str`], [`Params::get_i32`], [`Params::get_f64`],
//! [`Params::get_bool`], [`Params::get_list`], [`Params::get_object`]),
//! each returning `None` for a missing key, a null, or a value of another kind.
//!
//! The parser is lazy, so an accessor only walks the buffer
//! as far as it needs to find the requested key.
//!
//! ## Wire format
//!
//! Snapshots are a packed byte buffer in little-endian order.
//! The format mirrors the `ParamValue` value space:
//!
//! ```text
//! u32  count
//! for each entry:
//!   u8   kind       0 = str, 1 = i32, 2 = f64, 3 = bool, 4 = null, 5 = list, 6 = object
//!   u16  key_len
//!   key_len bytes utf-8
//!   variant payload:
//!     str    → u32 len; len bytes utf-8
//!     i32    → 4 bytes LE
//!     f64    → 8 bytes LE
//!     bool   → 1 byte
//!     null   → no bytes
//!     list   → u32 count; count × (u8 kind; variant payload)
//!     object → u32 count; count × entry, laid out as above
//! ```
//!
//! Null entries are packed for every manifest-declared key that has no resolved value.
//! The snapshot's shape is faithful to the manifest. The typed accessors return `None`
//! for null entries (same as for missing keys); [`Params::keys`] still yields them
//! so callers iterating the full set see the full manifest.

/// Wire-format kind discriminators for `ParamValue` variants.
///
/// One byte per entry, preceding the variant-specific payload.
pub mod kind {
    pub const STR: u8 = 0;
    pub const I32: u8 = 1;
    pub const F64: u8 = 2;
    pub const BOOL: u8 = 3;
    pub const NULL: u8 = 4;
    pub const LIST: u8 = 5;
    pub const OBJECT: u8 = 6;
}

/// Owned snapshot of the host-delivered params for a widget instance.
///
/// Cheap to [`Clone`] — the underlying storage is a single `Vec<u8>`,
/// so a widget can hold the current and the previous snapshot side by side
/// for React-style "did this param change since last render" checks.
#[derive(Clone, Default, Debug)]
pub struct Params {
    /// Packed bytes in the wire format above.
    bytes: Vec<u8>,
}

impl Params {
    /// Build a `Params` from an owned packed-byte buffer.
    ///
    /// The host owns the wire layout; a malformed buffer here means the host messed up,
    /// not the widget. The constructor accepts the bytes unconditionally — accessors
    /// and iterators stop on the first parse error and yield no further entries.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Returns `true` when the snapshot carries no entries (count == 0,
    /// or the buffer is too short to even read the count header).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Number of entries the snapshot's header claims.
    /// Reads the first 4 LE bytes; returns 0 if the buffer is shorter than the header itself.
    fn count(&self) -> u32 {
        let Some(head) = self.bytes.first_chunk::<4>() else {
            return 0;
        };
        u32::from_le_bytes(*head)
    }

    /// Returns the value for `key`, or `None` if the key is missing.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Value<'_>> {
        self.entries().find(|e| e.key == key).map(|e| e.value)
    }

    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    #[must_use]
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key)?.as_i32()
    }

    #[must_use]
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    #[must_use]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    #[must_use]
    pub fn get_list(&self, key: &str) -> Option<List<'_>> {
        self.get(key)?.as_list()
    }

    #[must_use]
    pub fn get_object(&self, key: &str) -> Option<Object<'_>> {
        self.get(key)?.as_object()
    }

    /// Iterator over every key the snapshot carries, including keys whose value is `null`.
    /// Order matches the host's serialisation order, which is alphabetical by key
    /// for deterministic snapshot equality.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries().map(|e| e.key)
    }

    /// Internal walker over decoded entries.
    /// Stops at the first parse error.
    fn entries(&self) -> EntryIter<'_> {
        EntryIter::new(self.bytes.get(4..).unwrap_or_default(), self.count())
    }
}

#[derive(Debug)]
struct Entry<'a> {
    key: &'a str,
    value: Value<'a>,
}

/// One param value, borrowed from the snapshot's bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Str(&'a str),
    I32(i32),
    F64(f64),
    Bool(bool),
    Null,
    List(List<'a>),
    Object(Object<'a>),
}

impl<'a> Value<'a> {
    #[must_use]
    pub fn as_str(&self) -> Option<&'a str> {
        if let Self::Str(s) = *self {
            Some(s)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_i32(&self) -> Option<i32> {
        if let Self::I32(v) = *self {
            Some(v)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        if let Self::F64(v) = *self {
            Some(v)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = *self {
            Some(b)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_list(&self) -> Option<List<'a>> {
        if let Self::List(list) = *self {
            Some(list)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_object(&self) -> Option<Object<'a>> {
        if let Self::Object(object) = *self {
            Some(object)
        } else {
            None
        }
    }
}

/// A list param's items, decoded as they are iterated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct List<'a> {
    /// `count` items, each a kind byte and its payload.
    bytes: &'a [u8],
    count: u32,
}

impl<'a> List<'a> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.count as usize
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub fn iter(&self) -> ListIter<'a> {
        ListIter {
            cursor: Cursor::new(self.bytes),
            remaining: self.count,
        }
    }
}

impl<'a> IntoIterator for &List<'a> {
    type Item = Value<'a>;
    type IntoIter = ListIter<'a>;

    fn into_iter(self) -> ListIter<'a> {
        self.iter()
    }
}

/// Iterator over a [`List`]'s items.
/// Stops at the first malformed item.
#[derive(Debug, Clone)]
pub struct ListIter<'a> {
    cursor: Cursor<'a>,
    remaining: u32,
}

impl<'a> Iterator for ListIter<'a> {
    type Item = Value<'a>;

    fn next(&mut self) -> Option<Value<'a>> {
        if self.remaining == 0 {
            return None;
        }
        let item = self.cursor.u8().and_then(|kind| self.cursor.value(kind));
        self.remaining = if item.is_some() {
            self.remaining - 1
        } else {
            0
        };
        item
    }
}

/// An object param's fields, decoded on lookup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Object<'a> {
    /// `count` entries, laid out like the snapshot's own.
    bytes: &'a [u8],
    count: u32,
}

impl<'a> Object<'a> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.count as usize
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<Value<'a>> {
        EntryIter::new(self.bytes, self.count)
            .find(|e| e.key == key)
            .map(|e| e.value)
    }
}

/// Walker over `count` packed entries.
/// Stops at the first malformed entry.
struct EntryIter<'a> {
    cursor: Cursor<'a>,
    remaining: u32,
}

impl<'a> EntryIter<'a> {
    fn new(bytes: &'a [u8], count: u32) -> Self {
        Self {
            cursor: Cursor::new(bytes),
            remaining: count,
        }
    }
}

impl<'a> Iterator for EntryIter<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Entry<'a>> {
        if self.remaining == 0 {
            return None;
        }
        let entry = self.cursor.entry();
        self.remaining = if entry.is_some() {
            self.remaining - 1
        } else {
            0
        };
        entry
    }
}

/// A read position in packed bytes.
/// Every read returns `None` rather than panicking
/// on a buffer too short or malformed for it.
#[derive(Debug, Clone)]
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(len)?;
        let taken = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(taken)
    }

    fn take_array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.first_chunk::<N>().copied()
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn len_u32(&mut self) -> Option<usize> {
        usize::try_from(u32::from_le_bytes(self.take_array()?)).ok()
    }

    fn str(&mut self, len: usize) -> Option<&'a str> {
        core::str::from_utf8(self.take(len)?).ok()
    }

    fn entry(&mut self) -> Option<Entry<'a>> {
        let kind = self.u8()?;
        let key_len = usize::from(u16::from_le_bytes(self.take_array()?));
        let key = self.str(key_len)?;
        let value = self.value(kind)?;
        Some(Entry { key, value })
    }

    fn value(&mut self, kind: u8) -> Option<Value<'a>> {
        let value = match kind {
            kind::STR => {
                let len = self.len_u32()?;
                Value::Str(self.str(len)?)
            }
            kind::I32 => Value::I32(i32::from_le_bytes(self.take_array()?)),
            kind::F64 => Value::F64(f64::from_le_bytes(self.take_array()?)),
            kind::BOOL => Value::Bool(self.u8()? != 0),
            kind::NULL => Value::Null,
            kind::LIST => {
                let count = u32::from_le_bytes(self.take_array()?);
                let start = self.offset;
                for _ in 0..count {
                    let item_kind = self.u8()?;
                    self.value(item_kind)?;
                }
                Value::List(List {
                    bytes: self.bytes.get(start..self.offset)?,
                    count,
                })
            }
            kind::OBJECT => {
                let count = u32::from_le_bytes(self.take_array()?);
                let start = self.offset;
                for _ in 0..count {
                    self.entry()?;
                }
                Value::Object(Object {
                    bytes: self.bytes.get(start..self.offset)?,
                    count,
                })
            }
            _ => return None,
        };
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a packed buffer from an iterator of (key, value) pairs
    /// in the same wire format the host will produce.
    ///
    /// Used by the unit tests to exercise the parser end-to-end
    /// without the host plumbing.
    struct PackedBuilder {
        out: Vec<u8>,
        count: u32,
    }

    impl PackedBuilder {
        fn new() -> Self {
            // Reserve space for the count header.
            Self {
                out: vec![0; 4],
                count: 0,
            }
        }

        fn push_key(&mut self, kind: u8, key: &str) {
            self.count += 1;
            self.out.push(kind);
            let key_len = u16::try_from(key.len())
                .expect("BUG: test fixtures always use keys well under 64 KiB");
            self.out.extend_from_slice(&key_len.to_le_bytes());
            self.out.extend_from_slice(key.as_bytes());
        }

        fn str(mut self, key: &str, value: &str) -> Self {
            self.push_key(kind::STR, key);
            let len = u32::try_from(value.len())
                .expect("BUG: test fixtures always use values well under 4 GiB");
            self.out.extend_from_slice(&len.to_le_bytes());
            self.out.extend_from_slice(value.as_bytes());
            self
        }

        fn i32(mut self, key: &str, value: i32) -> Self {
            self.push_key(kind::I32, key);
            self.out.extend_from_slice(&value.to_le_bytes());
            self
        }

        fn f64(mut self, key: &str, value: f64) -> Self {
            self.push_key(kind::F64, key);
            self.out.extend_from_slice(&value.to_le_bytes());
            self
        }

        fn bool(mut self, key: &str, value: bool) -> Self {
            self.push_key(kind::BOOL, key);
            self.out.push(u8::from(value));
            self
        }

        fn null(mut self, key: &str) -> Self {
            self.push_key(kind::NULL, key);
            self
        }

        fn i32_list(mut self, key: &str, items: &[i32]) -> Self {
            self.push_key(kind::LIST, key);
            let count =
                u32::try_from(items.len()).expect("BUG: test fixtures always use short lists");
            self.out.extend_from_slice(&count.to_le_bytes());
            for item in items {
                self.out.push(kind::I32);
                self.out.extend_from_slice(&item.to_le_bytes());
            }
            self
        }

        fn build(mut self) -> Vec<u8> {
            let head = self.count.to_le_bytes();
            self.out[0..4].copy_from_slice(&head);
            self.out
        }
    }

    #[test]
    fn default_params_is_empty() {
        let p = Params::default();
        assert!(p.is_empty());
        assert_eq!(p.keys().count(), 0);
        assert_eq!(p.get_str("x"), None);
        assert_eq!(p.get_i32("x"), None);
        assert_eq!(p.get_f64("x"), None);
        assert_eq!(p.get_bool("x"), None);
    }

    #[test]
    fn empty_header_is_empty() {
        let p = Params::from_bytes(0_u32.to_le_bytes().to_vec());
        assert!(p.is_empty());
    }

    #[test]
    fn each_scalar_kind_round_trips() {
        let bytes = PackedBuilder::new()
            .str("label", "hello")
            .i32("count", -7)
            .f64("ratio", 2.5)
            .bool("active", true)
            .null("optional")
            .build();

        let p = Params::from_bytes(bytes);

        assert!(!p.is_empty());
        assert_eq!(p.get_str("label"), Some("hello"));
        assert_eq!(p.get_i32("count"), Some(-7));
        assert_eq!(p.get_f64("ratio"), Some(2.5));
        assert_eq!(p.get_bool("active"), Some(true));
    }

    #[test]
    fn null_entries_are_visible_to_keys_but_invisible_to_typed_accessors() {
        let bytes = PackedBuilder::new()
            .null("optional")
            .str("present", "yes")
            .build();
        let p = Params::from_bytes(bytes);

        let keys: Vec<&str> = p.keys().collect();
        assert_eq!(keys, vec!["optional", "present"]);

        assert_eq!(p.get_str("optional"), None);
        assert_eq!(p.get_i32("optional"), None);
        assert_eq!(p.get_f64("optional"), None);
        assert_eq!(p.get_bool("optional"), None);
        assert_eq!(p.get_str("present"), Some("yes"));
    }

    #[test]
    fn wrong_kind_returns_none() {
        let bytes = PackedBuilder::new()
            .str("label", "hello")
            .i32("count", 42)
            .build();
        let p = Params::from_bytes(bytes);

        assert_eq!(p.get_i32("label"), None);
        assert_eq!(p.get_str("count"), None);
        assert_eq!(p.get_bool("label"), None);
        assert_eq!(p.get_f64("count"), None);
    }

    #[test]
    fn missing_key_returns_none() {
        let bytes = PackedBuilder::new().str("present", "yes").build();
        let p = Params::from_bytes(bytes);
        assert_eq!(p.get_str("absent"), None);
        assert_eq!(p.get_i32("absent"), None);
        assert_eq!(p.get_f64("absent"), None);
        assert_eq!(p.get_bool("absent"), None);
    }

    #[test]
    fn keys_iterates_in_packed_order() {
        let bytes = PackedBuilder::new()
            .str("z", "")
            .str("a", "")
            .str("m", "")
            .build();
        let p = Params::from_bytes(bytes);
        let keys: Vec<&str> = p.keys().collect();
        assert_eq!(keys, vec!["z", "a", "m"]);
    }

    #[test]
    fn truncated_buffer_stops_parser_gracefully() {
        let mut bytes = PackedBuilder::new().str("label", "hello").build();
        // Drop the last 3 bytes of the value — the parser should not panic.
        bytes.truncate(bytes.len() - 3);
        let p = Params::from_bytes(bytes);
        assert_eq!(p.get_str("label"), None);
    }

    #[test]
    fn a_list_reads_item_by_item_and_is_stepped_over_whole() {
        let bytes = PackedBuilder::new()
            .i32_list("nums", &[1, 2, 3])
            .str("after", "yes")
            .build();
        let p = Params::from_bytes(bytes);

        let list = p.get_list("nums").expect("BUG: nums is a list");
        assert_eq!(list.len(), 3);
        let nums: Vec<i32> = list.iter().filter_map(|v| v.as_i32()).collect();
        assert_eq!(nums, [1, 2, 3]);
        assert_eq!(
            p.get_str("after"),
            Some("yes"),
            "the parser must step over the whole list to reach the next entry"
        );
    }

    #[test]
    fn a_truncated_list_stops_parser_gracefully() {
        let mut bytes = PackedBuilder::new().i32_list("nums", &[1, 2, 3]).build();
        bytes.truncate(bytes.len() - 2);
        let p = Params::from_bytes(bytes);
        assert_eq!(p.get_list("nums"), None);
    }

    #[test]
    fn unknown_kind_stops_parser_gracefully() {
        // Count = 1, kind = 99 (unknown).
        let bytes = vec![1, 0, 0, 0, 99, 1, 0, b'x'];
        let p = Params::from_bytes(bytes);
        assert_eq!(p.keys().count(), 0);
    }

    #[test]
    fn non_utf8_key_stops_parser_gracefully() {
        // Count = 1, kind = STR, key_len = 1, key = 0xFF (invalid UTF-8).
        let bytes = vec![1, 0, 0, 0, kind::STR, 1, 0, 0xFF, 0, 0, 0, 0];
        let p = Params::from_bytes(bytes);
        assert_eq!(p.keys().count(), 0);
    }

    #[test]
    fn clone_is_cheap_and_preserves_content() {
        let bytes = PackedBuilder::new()
            .str("label", "hello")
            .i32("count", 42)
            .build();
        let p = Params::from_bytes(bytes);
        let p2 = p.clone();
        assert_eq!(p.get_str("label"), p2.get_str("label"));
        assert_eq!(p.get_i32("count"), p2.get_i32("count"));
    }
}
