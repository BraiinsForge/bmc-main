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

//! Typed access shim used by widget-side `manifest_params.rs` modules emitted
//! by `bmc-widget-codegen`. The generated module wraps the dynamic [`Params`]
//! snapshot into a struct whose fields are named and typed against the manifest,
//! then reaches back into this module for the actual per-value reads.
//!
//! Splitting the read logic into the SDK keeps each generated `manifest_params.rs`
//! small and uniform — the heavy lifting (panic-message format, optional-vs-required
//! dispatch, snapshot integration) lives once here instead of being re-emitted per
//! widget. The generated code is then mostly a list of typed key names + a thin
//! `from_snapshot` body that delegates back through [`ParamRead`].
//!
//! ## Required vs optional
//!
//! [`ParamRead::read_required`] panics when the snapshot is missing or null for the key.
//! Saving a scene stores every declared key, defaults included,
//! but nothing fills in a key or list-row field the widget later requires:
//! params stored before then lack it until the widget migrates them.
//!
//! [`ParamRead::read_optional`] returns `None` for missing or null entries.
//!
//! ## Values of another type
//!
//! The host checks values against the manifest when a scene is saved, not when it loads,
//! so a value stored before the widget changed a param's type or options reaches it as stored.
//! Required and optional reads alike panic on a value or list item of another type,
//! or an enum value outside its options, rather than skip it.
//!
//! ## Enums
//!
//! Manifest `enum_values` types implement [`ValueRead`], and through it [`ParamRead`],
//! via the [`crate::impl_manifest_str_enum!`], [`crate::impl_manifest_i32_enum!`]
//! and [`crate::impl_manifest_f64_enum!`] macros below. Each macro expects the enum
//! to already provide an inherent `fn from_manifest_value(...) -> Option<Self>`
//! — the codegen emits both alongside the macro invocation.

use super::{Object, Params, Value};

/// Materialise a typed value out of a dynamic [`Params`] snapshot.
///
/// Implemented for every [`ValueRead`] type and for a `Vec` of one.
pub trait ParamRead: Sized {
    /// Read a required key. Panics when the snapshot is missing or null for `key`:
    /// params stored before the widget required it lack it until migrated.
    #[must_use]
    fn read_required(snap: &Params, key: &str) -> Self {
        Self::read_optional(snap, key).unwrap_or_else(|| {
            panic!(
                "required param `{key}` missing from the snapshot: \
                params stored before the widget required it lack it until migrated"
            )
        })
    }

    /// Read an optional key. Returns `None` for missing or null entries.
    fn read_optional(snap: &Params, key: &str) -> Option<Self>;
}

impl<T: ValueRead> ParamRead for T {
    fn read_optional(snap: &Params, key: &str) -> Option<Self> {
        read_present(snap, key, |value| T::from_value(value).ok())
    }
}

/// `None` when `key` is missing or null; panics on a value `read` refuses.
fn read_present<'s, T>(
    snap: &'s Params,
    key: &str,
    read: impl FnOnce(Value<'s>) -> Option<T>,
) -> Option<T> {
    let value = snap.get(key)?;
    if matches!(value, Value::Null) {
        return None;
    }
    Some(read(value).unwrap_or_else(|| {
        panic!(
            "param `{key}` does not match the manifest type: \
            params stored before the widget changed it keep the old value until migrated"
        )
    }))
}

/// Why a [`Value`] did not read as the type asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadError {
    /// A value of another type, or an enum value outside its options.
    Mismatch,
    /// A row lacks this required field, or holds it as null.
    MissingField(String),
}

/// Materialise a typed value out of one [`Value`].
pub trait ValueRead: Sized {
    /// [`ReadError::Mismatch`] for a null or a value of another type.
    /// Read a row's fields through [`required_field`] and [`optional_field`],
    /// which handle absence and null.
    fn from_value(value: Value<'_>) -> Result<Self, ReadError>;
}

impl ValueRead for String {
    fn from_value(value: Value<'_>) -> Result<Self, ReadError> {
        value.as_str().map(str::to_owned).ok_or(ReadError::Mismatch)
    }
}

impl ValueRead for i32 {
    fn from_value(value: Value<'_>) -> Result<Self, ReadError> {
        value.as_i32().ok_or(ReadError::Mismatch)
    }
}

impl ValueRead for f64 {
    fn from_value(value: Value<'_>) -> Result<Self, ReadError> {
        value.as_f64().ok_or(ReadError::Mismatch)
    }
}

impl ValueRead for bool {
    fn from_value(value: Value<'_>) -> Result<Self, ReadError> {
        value.as_bool().ok_or(ReadError::Mismatch)
    }
}

impl<T: ValueRead> ParamRead for Vec<T> {
    fn read_optional(snap: &Params, key: &str) -> Option<Self> {
        let list = read_present(snap, key, |value| value.as_list())?;
        Some(
            list.iter()
                .enumerate()
                .map(|(i, item)| match T::from_value(item) {
                    Ok(item) => item,
                    Err(ReadError::Mismatch) => panic!(
                        "param `{key}` item {i} does not match the manifest item type: \
                        items stored before the widget changed it keep the old value until migrated"
                    ),
                    Err(ReadError::MissingField(field)) => panic!(
                        "required field `{field}` of param `{key}` item {i} is missing: \
                        rows stored before the widget required it lack it until migrated"
                    ),
                })
                .collect(),
        )
    }
}

/// A required field of an object item; absent or null is [`ReadError::MissingField`].
pub fn required_field<T: ValueRead>(row: &Object<'_>, key: &str) -> Result<T, ReadError> {
    match row.get(key) {
        None | Some(Value::Null) => Err(ReadError::MissingField(key.to_owned())),
        Some(value) => T::from_value(value),
    }
}

/// An optional field of an object item: `None` when absent or null.
pub fn optional_field<T: ValueRead>(row: &Object<'_>, key: &str) -> Result<Option<T>, ReadError> {
    match row.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => T::from_value(value).map(Some),
    }
}

/// Implement [`ValueRead`], and through it [`ParamRead`], for a manifest string-enum.
///
/// The enum must provide an inherent `fn from_manifest_value(s: &str) -> Option<Self>`
/// — the codegen emits both this macro call and the function next to each other.
#[macro_export]
macro_rules! impl_manifest_str_enum {
    ($t:ty) => {
        impl $crate::params::typed::ValueRead for $t {
            fn from_value(
                value: $crate::params::Value<'_>,
            ) -> Result<Self, $crate::params::typed::ReadError> {
                value
                    .as_str()
                    .and_then(<$t>::from_manifest_value)
                    .ok_or($crate::params::typed::ReadError::Mismatch)
            }
        }
    };
}

/// Implement [`ValueRead`], and through it [`ParamRead`],
/// for a manifest integer-enum (`enum_values` of `i32`).
#[macro_export]
macro_rules! impl_manifest_i32_enum {
    ($t:ty) => {
        impl $crate::params::typed::ValueRead for $t {
            fn from_value(
                value: $crate::params::Value<'_>,
            ) -> Result<Self, $crate::params::typed::ReadError> {
                value
                    .as_i32()
                    .and_then(<$t>::from_manifest_value)
                    .ok_or($crate::params::typed::ReadError::Mismatch)
            }
        }
    };
}

/// Implement [`ValueRead`], and through it [`ParamRead`],
/// for a manifest double-enum (`enum_values` of `f64`).
#[macro_export]
macro_rules! impl_manifest_f64_enum {
    ($t:ty) => {
        impl $crate::params::typed::ValueRead for $t {
            fn from_value(
                value: $crate::params::Value<'_>,
            ) -> Result<Self, $crate::params::typed::ReadError> {
                value
                    .as_f64()
                    .and_then(<$t>::from_manifest_value)
                    .ok_or($crate::params::typed::ReadError::Mismatch)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::super::Params;
    use super::*;
    use bmc_wasm_protocol::params::kind;

    /// Build a `Params` from a tiny inline byte buffer for testing.
    /// Layout matches `params.rs` (count header + per-entry tag/key/payload).
    fn build(entries: &[(&str, Entry<'_>)]) -> Params {
        let mut buf = Vec::new();
        write_entries(&mut buf, entries);
        Params::from_bytes(buf)
    }

    fn write_entries(buf: &mut Vec<u8>, entries: &[(&str, Entry<'_>)]) {
        let count = u32::try_from(entries.len()).expect("BUG: test entry count fits u32");
        buf.extend_from_slice(&count.to_le_bytes());
        for (key, ent) in entries {
            let key_bytes = key.as_bytes();
            let key_len = u16::try_from(key_bytes.len()).expect("BUG: test key under 64 KiB");
            buf.push(ent.kind());
            buf.extend_from_slice(&key_len.to_le_bytes());
            buf.extend_from_slice(key_bytes);
            ent.write_payload(buf);
        }
    }

    enum Entry<'a> {
        Str(&'a str),
        I32(i32),
        F64(f64),
        Bool(bool),
        Null,
        List(&'a [Entry<'a>]),
        Object(&'a [(&'a str, Entry<'a>)]),
    }

    impl Entry<'_> {
        fn kind(&self) -> u8 {
            match self {
                Entry::Str(_) => kind::STR,
                Entry::I32(_) => kind::I32,
                Entry::F64(_) => kind::F64,
                Entry::Bool(_) => kind::BOOL,
                Entry::Null => kind::NULL,
                Entry::List(_) => kind::LIST,
                Entry::Object(_) => kind::OBJECT,
            }
        }

        fn write_payload(&self, buf: &mut Vec<u8>) {
            match self {
                Entry::Str(v) => {
                    let vb = v.as_bytes();
                    let val_len = u32::try_from(vb.len()).expect("BUG: test value under 4 GiB");
                    buf.extend_from_slice(&val_len.to_le_bytes());
                    buf.extend_from_slice(vb);
                }
                Entry::I32(v) => buf.extend_from_slice(&v.to_le_bytes()),
                Entry::F64(v) => buf.extend_from_slice(&v.to_le_bytes()),
                Entry::Bool(v) => buf.push(u8::from(*v)),
                Entry::Null => {}
                Entry::List(items) => {
                    let count = u32::try_from(items.len()).expect("BUG: test list fits u32");
                    buf.extend_from_slice(&count.to_le_bytes());
                    for item in *items {
                        buf.push(item.kind());
                        item.write_payload(buf);
                    }
                }
                Entry::Object(fields) => write_entries(buf, fields),
            }
        }
    }

    #[test]
    fn required_primitives_round_trip() {
        let p = build(&[
            ("s", Entry::Str("hi")),
            ("i", Entry::I32(7)),
            ("f", Entry::F64(1.5)),
            ("b", Entry::Bool(true)),
        ]);
        assert_eq!(<String as ParamRead>::read_required(&p, "s"), "hi");
        assert_eq!(<i32 as ParamRead>::read_required(&p, "i"), 7);
        assert!((<f64 as ParamRead>::read_required(&p, "f") - 1.5).abs() < f64::EPSILON);
        assert!(<bool as ParamRead>::read_required(&p, "b"));
    }

    #[test]
    fn optional_returns_none_for_null_entries() {
        let p = build(&[("s", Entry::Null), ("i", Entry::Null)]);
        assert!(<String as ParamRead>::read_optional(&p, "s").is_none());
        assert!(<i32 as ParamRead>::read_optional(&p, "i").is_none());
    }

    #[test]
    fn optional_returns_none_for_missing_keys() {
        let p = build(&[]);
        assert!(<bool as ParamRead>::read_optional(&p, "absent").is_none());
        assert!(<f64 as ParamRead>::read_optional(&p, "absent").is_none());
    }

    #[test]
    #[should_panic(expected = "required param `missing` missing from the snapshot")]
    fn required_panics_on_missing() {
        let p = build(&[]);
        let _: String = ParamRead::read_required(&p, "missing");
    }

    #[test]
    #[should_panic(expected = "param `s` does not match the manifest type: \
                               params stored before the widget changed it keep the old value")]
    fn required_panics_on_a_value_of_another_type() {
        let p = build(&[("s", Entry::I32(1))]);
        let _: String = ParamRead::read_required(&p, "s");
    }

    #[test]
    #[should_panic(expected = "param `s` does not match the manifest type")]
    fn optional_panics_on_a_value_of_another_type() {
        let p = build(&[("s", Entry::I32(1))]);
        let _ = <String as ParamRead>::read_optional(&p, "s");
    }

    // ── List coverage ───────────────────────────────────────────────

    #[test]
    fn list_reads_its_items_in_order() {
        let p = build(&[
            ("s", Entry::List(&[Entry::Str("NVDA"), Entry::Str("AAPL")])),
            ("i", Entry::List(&[Entry::I32(3), Entry::I32(1)])),
            ("e", Entry::List(&[])),
        ]);
        assert_eq!(
            <Vec<String> as ParamRead>::read_required(&p, "s"),
            ["NVDA", "AAPL"]
        );
        assert_eq!(<Vec<i32> as ParamRead>::read_required(&p, "i"), [3, 1]);
        assert!(<Vec<bool> as ParamRead>::read_required(&p, "e").is_empty());
    }

    #[test]
    #[should_panic(expected = "param `l` item 1 does not match the manifest item type: \
                               items stored before the widget changed it keep the old value")]
    fn list_item_of_another_type_panics() {
        let p = build(&[("l", Entry::List(&[Entry::I32(1), Entry::Str("two")]))]);
        let _ = <Vec<i32> as ParamRead>::read_required(&p, "l");
    }

    #[test]
    #[should_panic(expected = "param `l` does not match the manifest type")]
    fn list_param_holding_no_list_panics() {
        let p = build(&[("l", Entry::I32(1))]);
        let _ = <Vec<i32> as ParamRead>::read_required(&p, "l");
    }

    // ── Object-item coverage ────────────────────────────────────────

    #[derive(Debug, PartialEq)]
    struct Link {
        label: String,
        url: Option<String>,
    }

    impl ValueRead for Link {
        fn from_value(value: Value<'_>) -> Result<Self, ReadError> {
            let row = value.as_object().ok_or(ReadError::Mismatch)?;
            Ok(Self {
                label: required_field(&row, "label")?,
                url: optional_field(&row, "url")?,
            })
        }
    }

    #[test]
    fn object_rows_read_their_fields() {
        let p = build(&[(
            "links",
            Entry::List(&[
                Entry::Object(&[
                    ("label", Entry::Str("Pool")),
                    ("url", Entry::Str("https://x")),
                ]),
                Entry::Object(&[("label", Entry::Str("Home")), ("url", Entry::Null)]),
            ]),
        )]);
        assert_eq!(
            <Vec<Link> as ParamRead>::read_required(&p, "links"),
            [
                Link {
                    label: "Pool".into(),
                    url: Some("https://x".into()),
                },
                Link {
                    label: "Home".into(),
                    url: None,
                },
            ]
        );
    }

    #[test]
    #[should_panic(
        expected = "required field `label` of param `links` item 0 is missing: \
                               rows stored before the widget required it"
    )]
    fn object_row_without_a_required_field_names_it() {
        let p = build(&[(
            "links",
            Entry::List(&[Entry::Object(&[("url", Entry::Str("https://x"))])]),
        )]);
        let _ = <Vec<Link> as ParamRead>::read_required(&p, "links");
    }

    #[test]
    #[should_panic(expected = "required field `label` of param `links` item 0 is missing")]
    fn object_row_with_a_null_required_field_names_it() {
        let p = build(&[(
            "links",
            Entry::List(&[Entry::Object(&[("label", Entry::Null)])]),
        )]);
        let _ = <Vec<Link> as ParamRead>::read_required(&p, "links");
    }

    #[test]
    #[should_panic(expected = "param `links` item 0 does not match the manifest item type")]
    fn object_row_with_a_required_field_of_another_type_panics() {
        let p = build(&[(
            "links",
            Entry::List(&[Entry::Object(&[("label", Entry::I32(1))])]),
        )]);
        let _ = <Vec<Link> as ParamRead>::read_required(&p, "links");
    }

    #[test]
    #[should_panic(expected = "param `links` item 0 does not match the manifest item type")]
    fn object_row_with_an_optional_field_of_another_type_panics() {
        let p = build(&[(
            "links",
            Entry::List(&[Entry::Object(&[
                ("label", Entry::Str("Pool")),
                ("url", Entry::I32(1)),
            ])]),
        )]);
        let _ = <Vec<Link> as ParamRead>::read_required(&p, "links");
    }

    // ── String-enum macro coverage ──────────────────────────────────

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Color {
        Red,
        Blue,
    }

    impl Color {
        fn from_manifest_value(s: &str) -> Option<Self> {
            match s {
                "red" => Some(Self::Red),
                "blue" => Some(Self::Blue),
                _ => None,
            }
        }
    }

    crate::impl_manifest_str_enum!(Color);

    #[test]
    fn str_enum_required_reads_match() {
        let p = build(&[("c", Entry::Str("blue"))]);
        assert_eq!(<Color as ParamRead>::read_required(&p, "c"), Color::Blue);
    }

    #[test]
    fn str_enum_optional_absent_is_none() {
        let p = build(&[("c", Entry::Null)]);
        assert!(<Color as ParamRead>::read_optional(&p, "c").is_none());
    }

    #[test]
    #[should_panic(expected = "param `c` does not match the manifest type")]
    fn str_enum_panics_on_unknown_value() {
        let p = build(&[("c", Entry::Str("green"))]);
        let _ = <Color as ParamRead>::read_required(&p, "c");
    }

    #[test]
    #[should_panic(expected = "param `c` does not match the manifest type")]
    fn str_enum_optional_panics_on_unknown_value() {
        let p = build(&[("c", Entry::Str("green"))]);
        let _ = <Color as ParamRead>::read_optional(&p, "c");
    }

    #[test]
    fn str_enum_list_reads_each_item() {
        let p = build(&[("c", Entry::List(&[Entry::Str("red"), Entry::Str("blue")]))]);
        assert_eq!(
            <Vec<Color> as ParamRead>::read_required(&p, "c"),
            [Color::Red, Color::Blue]
        );
    }

    #[test]
    #[should_panic(expected = "param `c` item 0 does not match the manifest item type")]
    fn str_enum_list_panics_on_an_unknown_item() {
        let p = build(&[("c", Entry::List(&[Entry::Str("green")]))]);
        let _ = <Vec<Color> as ParamRead>::read_required(&p, "c");
    }

    // ── i32-enum macro coverage ─────────────────────────────────────

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Rank {
        One,
        Two,
    }

    impl Rank {
        fn from_manifest_value(v: i32) -> Option<Self> {
            match v {
                1 => Some(Self::One),
                2 => Some(Self::Two),
                _ => None,
            }
        }
    }

    crate::impl_manifest_i32_enum!(Rank);

    #[test]
    fn i32_enum_round_trip() {
        let p = build(&[("r", Entry::I32(2))]);
        assert_eq!(<Rank as ParamRead>::read_required(&p, "r"), Rank::Two);
    }

    // ── f64-enum macro coverage ─────────────────────────────────────

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Gamma {
        Linear,
        Srgb,
    }

    impl Gamma {
        fn from_manifest_value(v: f64) -> Option<Self> {
            if (v - 1.0).abs() < f64::EPSILON {
                Some(Self::Linear)
            } else if (v - 2.2).abs() < f64::EPSILON {
                Some(Self::Srgb)
            } else {
                None
            }
        }
    }

    crate::impl_manifest_f64_enum!(Gamma);

    #[test]
    fn f64_enum_epsilon_match() {
        let p = build(&[("g", Entry::F64(2.2))]);
        assert_eq!(<Gamma as ParamRead>::read_required(&p, "g"), Gamma::Srgb);
    }
}
