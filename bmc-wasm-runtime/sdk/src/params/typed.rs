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
//! [`ParamRead::read_required`] panics with a `BUG:` message when the manifest
//! declares a non-optional key but the host snapshot is missing or null for it.
//! That mirrors the contract the compositor's instance-validation pass enforces
//! (it injects manifest defaults for any required key the operator didn't set),
//! so a `None` here would indicate a host-side bug the widget should not paper over.
//!
//! [`ParamRead::read_optional`] returns `None` for missing or null entries.
//!
//! ## Enums
//!
//! Manifest `enum_values` types implement [`ParamRead`] via the
//! [`crate::impl_manifest_str_enum!`], [`crate::impl_manifest_i32_enum!`] and
//! [`crate::impl_manifest_f64_enum!`] macros below. Each macro expects the enum
//! to already provide an inherent `fn from_manifest_value(...) -> Option<Self>` —
//! the codegen emits both alongside the macro invocation.

use super::Params;

/// Materialise a typed value out of a dynamic [`Params`] snapshot.
///
/// Implemented in this module for `String`, `i32`, `f64`, `bool`; widget-side
/// codegen plugs in manifest `enum_values` types via the
/// `impl_manifest_{str,i32,f64}_enum!` macros.
pub trait ParamRead: Sized {
    /// Read a required key. Panics (BUG:) when the host snapshot is missing or null
    /// for `key`, since the compositor's validator should always inject the manifest
    /// default for required keys.
    fn read_required(snap: &Params, key: &str) -> Self;

    /// Read an optional key. Returns `None` for missing or null entries.
    fn read_optional(snap: &Params, key: &str) -> Option<Self>;
}

impl ParamRead for String {
    fn read_required(snap: &Params, key: &str) -> Self {
        snap.get_str(key)
            .unwrap_or_else(|| panic!("BUG: required param `{key}` missing from snapshot"))
            .to_owned()
    }

    fn read_optional(snap: &Params, key: &str) -> Option<Self> {
        snap.get_str(key).map(str::to_owned)
    }
}

impl ParamRead for i32 {
    fn read_required(snap: &Params, key: &str) -> Self {
        snap.get_i32(key)
            .unwrap_or_else(|| panic!("BUG: required param `{key}` missing from snapshot"))
    }

    fn read_optional(snap: &Params, key: &str) -> Option<Self> {
        snap.get_i32(key)
    }
}

impl ParamRead for f64 {
    fn read_required(snap: &Params, key: &str) -> Self {
        snap.get_f64(key)
            .unwrap_or_else(|| panic!("BUG: required param `{key}` missing from snapshot"))
    }

    fn read_optional(snap: &Params, key: &str) -> Option<Self> {
        snap.get_f64(key)
    }
}

impl ParamRead for bool {
    fn read_required(snap: &Params, key: &str) -> Self {
        snap.get_bool(key)
            .unwrap_or_else(|| panic!("BUG: required param `{key}` missing from snapshot"))
    }

    fn read_optional(snap: &Params, key: &str) -> Option<Self> {
        snap.get_bool(key)
    }
}

/// Implement [`ParamRead`] for a manifest string-enum.
///
/// The enum must provide an inherent `fn from_manifest_value(s: &str) -> Option<Self>`
/// — the codegen emits both this macro call and the function next to each other.
#[macro_export]
macro_rules! impl_manifest_str_enum {
    ($t:ty) => {
        impl $crate::params::typed::ParamRead for $t {
            fn read_required(snap: &$crate::params::Params, key: &str) -> Self {
                let s = snap
                    .get_str(key)
                    .unwrap_or_else(|| panic!("BUG: required param `{key}` missing from snapshot"));
                <$t>::from_manifest_value(s).unwrap_or_else(|| {
                    // Value is not interpolated into the panic message —
                    // strings can be operator-typed in future, so the format
                    // avoids leaking the value into trap logs by default.
                    // Inspect the snapshot directly if a value is needed.
                    panic!("BUG: required param `{key}` value not in manifest enum_values")
                })
            }

            fn read_optional(snap: &$crate::params::Params, key: &str) -> Option<Self> {
                let s = snap.get_str(key)?;
                Some(<$t>::from_manifest_value(s).unwrap_or_else(|| {
                    panic!("BUG: optional param `{key}` value not in manifest enum_values")
                }))
            }
        }
    };
}

/// Implement [`ParamRead`] for a manifest integer-enum (`enum_values` of `i32`).
#[macro_export]
macro_rules! impl_manifest_i32_enum {
    ($t:ty) => {
        impl $crate::params::typed::ParamRead for $t {
            fn read_required(snap: &$crate::params::Params, key: &str) -> Self {
                let v = snap
                    .get_i32(key)
                    .unwrap_or_else(|| panic!("BUG: required param `{key}` missing from snapshot"));
                <$t>::from_manifest_value(v).unwrap_or_else(|| {
                    panic!("BUG: required param `{key}` has value {v} not in manifest enum_values")
                })
            }

            fn read_optional(snap: &$crate::params::Params, key: &str) -> Option<Self> {
                let v = snap.get_i32(key)?;
                Some(<$t>::from_manifest_value(v).unwrap_or_else(|| {
                    panic!("BUG: optional param `{key}` has value {v} not in manifest enum_values")
                }))
            }
        }
    };
}

/// Implement [`ParamRead`] for a manifest double-enum (`enum_values` of `f64`).
#[macro_export]
macro_rules! impl_manifest_f64_enum {
    ($t:ty) => {
        impl $crate::params::typed::ParamRead for $t {
            fn read_required(snap: &$crate::params::Params, key: &str) -> Self {
                let v = snap
                    .get_f64(key)
                    .unwrap_or_else(|| panic!("BUG: required param `{key}` missing from snapshot"));
                <$t>::from_manifest_value(v).unwrap_or_else(|| {
                    panic!("BUG: required param `{key}` has value {v} not in manifest enum_values")
                })
            }

            fn read_optional(snap: &$crate::params::Params, key: &str) -> Option<Self> {
                let v = snap.get_f64(key)?;
                Some(<$t>::from_manifest_value(v).unwrap_or_else(|| {
                    panic!("BUG: optional param `{key}` has value {v} not in manifest enum_values")
                }))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::super::Params;
    use super::*;

    /// Build a `Params` from a tiny inline byte buffer for testing.
    /// Layout matches `params.rs` (count header + per-entry tag/key/payload).
    fn build(entries: &[(&str, Entry<'_>)]) -> Params {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (key, ent) in entries {
            let key_bytes = key.as_bytes();
            let key_len = u16::try_from(key_bytes.len()).expect("BUG: test key under 64 KiB");
            match ent {
                Entry::Str(v) => {
                    buf.push(0);
                    buf.extend_from_slice(&key_len.to_le_bytes());
                    buf.extend_from_slice(key_bytes);
                    let vb = v.as_bytes();
                    buf.extend_from_slice(&(vb.len() as u32).to_le_bytes());
                    buf.extend_from_slice(vb);
                }
                Entry::I32(v) => {
                    buf.push(1);
                    buf.extend_from_slice(&key_len.to_le_bytes());
                    buf.extend_from_slice(key_bytes);
                    buf.extend_from_slice(&v.to_le_bytes());
                }
                Entry::F64(v) => {
                    buf.push(2);
                    buf.extend_from_slice(&key_len.to_le_bytes());
                    buf.extend_from_slice(key_bytes);
                    buf.extend_from_slice(&v.to_le_bytes());
                }
                Entry::Bool(v) => {
                    buf.push(3);
                    buf.extend_from_slice(&key_len.to_le_bytes());
                    buf.extend_from_slice(key_bytes);
                    buf.push(u8::from(*v));
                }
                Entry::Null => {
                    buf.push(4);
                    buf.extend_from_slice(&key_len.to_le_bytes());
                    buf.extend_from_slice(key_bytes);
                }
            }
        }
        Params::from_bytes(buf)
    }

    enum Entry<'a> {
        Str(&'a str),
        I32(i32),
        F64(f64),
        Bool(bool),
        Null,
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
    #[should_panic(expected = "BUG: required param `missing` missing from snapshot")]
    fn required_panics_on_missing() {
        let p = build(&[]);
        let _: String = ParamRead::read_required(&p, "missing");
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
    #[should_panic(expected = "BUG: required param `c` value not in manifest enum_values")]
    fn str_enum_panics_on_unknown_value() {
        let p = build(&[("c", Entry::Str("green"))]);
        let _ = <Color as ParamRead>::read_required(&p, "c");
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
