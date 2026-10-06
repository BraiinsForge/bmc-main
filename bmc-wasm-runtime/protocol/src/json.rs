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

//! How a widget reads JSON: the host answers for a wasm guest,
//! and the native SDK reads the same way through these functions.

/// What a JSON value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonKind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

impl JsonKind {
    /// The host's tag: `0` null through `5` object; a negative tag means absent.
    #[must_use]
    pub const fn to_wire(self) -> i32 {
        match self {
            Self::Null => 0,
            Self::Bool => 1,
            Self::Number => 2,
            Self::String => 3,
            Self::Array => 4,
            Self::Object => 5,
        }
    }

    #[must_use]
    pub const fn from_wire(tag: i32) -> Option<Self> {
        match tag {
            0 => Some(Self::Null),
            1 => Some(Self::Bool),
            2 => Some(Self::Number),
            3 => Some(Self::String),
            4 => Some(Self::Array),
            5 => Some(Self::Object),
            _ => None,
        }
    }
}

#[cfg(feature = "host")]
pub use read::{Value, bool_at, f64_at, i64_at, kind_at, len_at, parse, str_at};

/// Paths are RFC 6901 JSON Pointers; every reader answers `None` where the path finds nothing.
#[cfg(feature = "host")]
mod read {
    pub use serde_json::Value;

    use super::JsonKind;

    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Value> {
        serde_json::from_slice(bytes).ok()
    }

    #[must_use]
    pub fn str_at<'doc>(doc: &'doc Value, path: &str) -> Option<&'doc str> {
        doc.pointer(path)?.as_str()
    }

    /// `None` for a number that is a float or does not fit an `i64`.
    #[must_use]
    pub fn i64_at(doc: &Value, path: &str) -> Option<i64> {
        doc.pointer(path)?.as_i64()
    }

    #[must_use]
    pub fn f64_at(doc: &Value, path: &str) -> Option<f64> {
        doc.pointer(path)?.as_f64()
    }

    #[must_use]
    pub fn bool_at(doc: &Value, path: &str) -> Option<bool> {
        doc.pointer(path)?.as_bool()
    }

    #[must_use]
    pub fn kind_at(doc: &Value, path: &str) -> Option<JsonKind> {
        Some(match doc.pointer(path)? {
            Value::Null => JsonKind::Null,
            Value::Bool(_) => JsonKind::Bool,
            Value::Number(_) => JsonKind::Number,
            Value::String(_) => JsonKind::String,
            Value::Array(_) => JsonKind::Array,
            Value::Object(_) => JsonKind::Object,
        })
    }

    /// Element count of an array or object; `None` for anything else.
    #[must_use]
    pub fn len_at(doc: &Value, path: &str) -> Option<usize> {
        match doc.pointer(path)? {
            Value::Array(items) => Some(items.len()),
            Value::Object(entries) => Some(entries.len()),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
        }
    }
}

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;

    fn doc() -> Value {
        serde_json::json!({
            "name": "s21",
            "count": 3,
            "ratio": 0.5,
            "huge": 18_446_744_073_709_551_615_u64,
            "flag": true,
            "nothing": null,
            "rows": [1, 2],
            "empty": {},
        })
    }

    #[test]
    fn a_wire_tag_round_trips_every_kind() {
        for kind in [
            JsonKind::Null,
            JsonKind::Bool,
            JsonKind::Number,
            JsonKind::String,
            JsonKind::Array,
            JsonKind::Object,
        ] {
            assert_eq!(JsonKind::from_wire(kind.to_wire()), Some(kind));
        }
        assert_eq!(JsonKind::from_wire(-1), None, "a negative tag is absence");
    }

    #[test]
    fn malformed_bytes_parse_to_nothing() {
        assert_eq!(parse(b"{\"unterminated\": "), None);
    }

    #[test]
    fn a_reader_answers_only_its_own_type() {
        let doc = doc();
        assert_eq!(str_at(&doc, "/name"), Some("s21"));
        assert_eq!(str_at(&doc, "/count"), None, "a number is not a string");
        assert_eq!(bool_at(&doc, "/flag"), Some(true));
        assert_eq!(bool_at(&doc, "/nothing"), None, "null is not a bool");
    }

    #[test]
    fn an_integer_reader_refuses_floats_and_overflow() {
        let doc = doc();
        assert_eq!(i64_at(&doc, "/count"), Some(3));
        assert_eq!(i64_at(&doc, "/ratio"), None);
        assert_eq!(i64_at(&doc, "/huge"), None);
        assert_eq!(
            f64_at(&doc, "/count"),
            Some(3.0),
            "an integer reads as a float"
        );
    }

    #[test]
    fn absence_is_told_apart_from_emptiness() {
        let doc = doc();
        assert_eq!(kind_at(&doc, "/empty"), Some(JsonKind::Object));
        assert_eq!(len_at(&doc, "/empty"), Some(0));
        assert_eq!(len_at(&doc, "/rows"), Some(2));
        assert_eq!(len_at(&doc, "/name"), None, "a string has no element count");
        assert_eq!(kind_at(&doc, "/absent"), None);
    }
}
