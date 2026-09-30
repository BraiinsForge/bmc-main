// Copyright (C) 2025  Braiins Systems s.r.o.
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

//! Schema-driven form field vocabulary — `ParamDefinition`/`ParamKind`, the value space `ParamValue`,
//! keyed by `ParamKey` — shared by the widget manifest's `params` and the credential-type `fields`.
//!
//! JSON-Schema-expressible constraints ride on `schemars` attributes; cross-field invariants
//! (`default_value` in `[min, max]` / in `enum_values`, `±0.0` enum collision) live in
//! [`ParamDefinition::validate`].

use std::collections::BTreeMap;
use std::marker::PhantomData;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub mod credential;
mod validate;

pub use validate::{MissingValues, Violation, validate_values};

/// Maximum byte length of a [`ParamKey`]. Under the wire-format `u16` length field, so the
/// encoder's `u16::try_from` is statically infallible.
pub const MAX_PARAM_KEY_LENGTH: usize = 64;

/// The most bytes a string value may hold,
/// checked when a value is written and when the widget host reads the params it is sent.
pub const MAX_PARAM_STRING_LENGTH: usize = 1024;

/// The most `max_items` an array field may declare:
/// a limit for the operator form, which edits every item as a row.
pub const MAX_ARRAY_ITEMS: usize = 100;

/// Errors from the field-schema validators; wrapped by the manifest / credential-type error types.
#[derive(Debug, Error)]
pub enum FieldSchemaError {
    /// A field failed a cross-field invariant JSON Schema cannot express.
    #[error("parameter {name:?}: {reason}")]
    InvalidParam { name: String, reason: String },

    /// Duplicate field key (the deserializer rejects these; kept for programmatic builders).
    #[error("duplicate parameter key: {0:?}")]
    DuplicateParamKey(String),
}

/// A field key inside a schema's field map (a manifest's `params`, a credential type's `fields`).
/// Identifier-shaped `^[A-Za-z][A-Za-z0-9_-]*$`, capped at [`MAX_PARAM_KEY_LENGTH`] bytes, so it is
/// safe to reuse as a generated Rust field name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(transparent)]
pub struct ParamKey(
    #[schemars(regex(pattern = r"^[A-Za-z][A-Za-z0-9_\-]*$"), length(max = MAX_PARAM_KEY_LENGTH))]
    String,
);

impl ParamKey {
    /// Construct a `ParamKey`, applying the same rules as `Deserialize`; returns the input on failure.
    pub fn try_new(s: String) -> Result<Self, String> {
        if Self::is_valid(&s) {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }

    fn is_valid(s: &str) -> bool {
        if s.len() > MAX_PARAM_KEY_LENGTH {
            return false;
        }
        let mut bytes = s.bytes();
        let first_ok = matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic());
        let rest_ok = bytes.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        first_ok && rest_ok
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ParamKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::try_new(s).map_err(|s| D::Error::custom(format!("invalid param key {s:?}")))
    }
}

impl std::borrow::Borrow<str> for ParamKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A single entry in a `ParamKind::String` `enum_values` list.
/// The `value` is what the host stores and the widget receives;
/// the `label` is the operator-facing string shown in the config UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StringOption {
    /// Wire value stored for this option.
    /// Must be unique within the surrounding `enum_values` array and non-empty after trim.
    /// Capped at [`MAX_PARAM_STRING_LENGTH`] bytes.
    #[schemars(length(max = MAX_PARAM_STRING_LENGTH))]
    pub value: String,
    /// Human-readable label shown in the operator UI.
    pub label: String,
}

/// A single entry in a `ParamKind::Double` `enum_values` list.
/// The `value` is the f64 selected; `label` is the operator-facing string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DoubleOption {
    /// Wire value stored for this option.
    /// Must be finite and unique within the surrounding `enum_values` array after canonicalising
    /// `+0.0` and `-0.0` to the same bit pattern.
    pub value: f64,
    /// Human-readable label shown in the operator UI.
    pub label: String,
}

/// A single entry in a `ParamKind::Integer` `enum_values` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IntegerOption {
    /// Wire value stored for this option.
    /// Must be unique within the surrounding `enum_values` array.
    pub value: i32,
    /// Human-readable label shown in the operator UI.
    pub label: String,
}

/// Optional structural hint on a `ParamKind::String`, instructing
/// the operator UI to render a specialised input (date picker, URI validator, etc.) instead of a free-form text field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StringFormat {
    /// ISO 8601 date (no time component).
    Date,
    /// ISO 8601 time of day.
    Time,
    /// RFC 5322 email address.
    Email,
    /// RFC 3986 URI.
    Uri,
    /// Sensitive value the UI must mask (render a password input, never echo the value back).
    Password,
}

/// Typed value for a stored field: null, bool, i32, finite f64, string, and lists and objects of those.
/// Both the compositor's in-memory form and the wire shape sent to widgets.
/// Which shapes a field accepts is its [`ParamKind`]'s call, enforced by [`validate_values`].
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ParamValue {
    /// Absence of a value. Sent for optional params the operator cleared.
    Null,
    /// A boolean.
    Boolean(bool),
    /// An i32 — the integer width the manifest declares.
    Integer(i32),
    /// A finite f64. NaN / ±infinity are rejected at parse time.
    Double(f64),
    /// A UTF-8 string.
    String(String),
    /// An ordered list of values.
    List(Vec<ParamValue>),
    /// Values keyed by field name.
    Object(BTreeMap<ParamKey, ParamValue>),
}

fn deserialize_finite_f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(D::Error::custom(format!(
            "ParamValue::Double must be finite (got {v})"
        )))
    }
}

impl<'de> Deserialize<'de> for ParamValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        ParamValueRepr::deserialize(d)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// A [`ParamValue`] as read, its object keys unchecked:
/// the untagged parse would swallow a repeated or malformed key into a vague mismatch.
#[derive(Deserialize)]
#[serde(
    untagged,
    expecting = "null, a boolean, a number, text, a list or an object"
)]
enum ParamValueRepr {
    Null,
    Boolean(bool),
    Integer(i32),
    #[serde(deserialize_with = "deserialize_finite_f64")]
    Double(f64),
    String(String),
    List(Vec<ParamValueRepr>),
    #[serde(deserialize_with = "deserialize_entries")]
    Object(Vec<(String, ParamValueRepr)>),
}

fn deserialize_entries<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Vec<(String, ParamValueRepr)>, D::Error> {
    struct Entries;

    impl<'de> Visitor<'de> for Entries {
        type Value = Vec<(String, ParamValueRepr)>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<Self::Value, M::Error> {
            let mut entries = Vec::with_capacity(access.size_hint().unwrap_or(0));
            while let Some(entry) = access.next_entry()? {
                entries.push(entry);
            }
            Ok(entries)
        }
    }

    d.deserialize_map(Entries)
}

impl TryFrom<ParamValueRepr> for ParamValue {
    type Error = String;

    fn try_from(repr: ParamValueRepr) -> Result<Self, String> {
        Ok(match repr {
            ParamValueRepr::Null => Self::Null,
            ParamValueRepr::Boolean(b) => Self::Boolean(b),
            ParamValueRepr::Integer(i) => Self::Integer(i),
            ParamValueRepr::Double(d) => Self::Double(d),
            ParamValueRepr::String(s) => Self::String(s),
            ParamValueRepr::List(items) => Self::List(
                items
                    .into_iter()
                    .map(Self::try_from)
                    .collect::<Result<_, _>>()?,
            ),
            ParamValueRepr::Object(entries) => {
                let mut fields = BTreeMap::new();
                for (key, value) in entries {
                    let key = ParamKey::try_new(key)
                        .map_err(|key| format!("invalid object key {key:?}"))?;
                    if fields.contains_key(&key) {
                        return Err(format!("duplicate object key {:?}", key.as_str()));
                    }
                    fields.insert(key, Self::try_from(value)?);
                }
                Self::Object(fields)
            }
        })
    }
}

impl ParamValue {
    /// JSON projection for the wayland boundary — bare values, not the internally-tagged form.
    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            ParamValue::Null => serde_json::Value::Null,
            ParamValue::Boolean(b) => serde_json::Value::Bool(*b),
            ParamValue::Integer(i) => serde_json::Value::Number((*i).into()),
            ParamValue::Double(d) => serde_json::Number::from_f64(*d)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            ParamValue::String(s) => serde_json::Value::String(s.clone()),
            ParamValue::List(items) => {
                serde_json::Value::Array(items.iter().map(ParamValue::to_json_value).collect())
            }
            ParamValue::Object(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.as_str().to_owned(), value.to_json_value()))
                    .collect(),
            ),
        }
    }

    /// Build the default value for a field; optional fields without a default yield `Null`.
    #[must_use]
    pub fn from_param_kind_default(kind: &ParamKind) -> Self {
        match kind.shape() {
            Shape::Scalar(scalar) => Self::from_scalar_default(scalar),
            Shape::Array(ArrayParam { default_value, .. }) => {
                ParamValue::List(default_value.clone())
            }
        }
    }

    #[must_use]
    pub fn from_scalar_default(scalar: Scalar<'_>) -> Self {
        match scalar {
            Scalar::String(StringParam { default_value, .. })
            | Scalar::Timezone(TimezoneParam { default_value, .. }) => default_value
                .clone()
                .map_or(ParamValue::Null, ParamValue::String),
            Scalar::Double(DoubleParam { default_value, .. }) => {
                default_value.map_or(ParamValue::Null, ParamValue::Double)
            }
            Scalar::Integer(IntegerParam { default_value, .. }) => {
                default_value.map_or(ParamValue::Null, ParamValue::Integer)
            }
            Scalar::Boolean(BooleanParam { default_value }) => {
                default_value.map_or(ParamValue::Null, ParamValue::Boolean)
            }
        }
    }
}

/// Reasons the wayland-side JSON-to-[`ParamValue`] conversion can fail.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParamValueConversionError {
    /// An object key that is not a valid [`ParamKey`].
    #[error("invalid object key {0:?}")]
    InvalidObjectKey(String),
    /// A number representable as neither i32 nor f64 (unreachable via `serde_json`, kept for exhaustiveness).
    #[error("number is not representable as i32 or f64")]
    UnrepresentableNumber,
    /// NaN or ±infinity (a hand-built `serde_json::Value` can carry these).
    #[error("number is not finite")]
    NonFiniteNumber,
    /// String value exceeded [`MAX_PARAM_STRING_LENGTH`].
    #[error("string value exceeds max length of {max} bytes (got {len})")]
    StringTooLong { len: usize, max: usize },
}

/// Inverse of [`ParamValue::to_json_value`] — re-types wayland-edge JSON into a [`ParamValue`],
/// erroring on non-finite numbers, over-long strings and object keys that are not a [`ParamKey`].
impl TryFrom<&serde_json::Value> for ParamValue {
    type Error = ParamValueConversionError;

    fn try_from(value: &serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Null => Ok(ParamValue::Null),
            serde_json::Value::Bool(b) => Ok(ParamValue::Boolean(*b)),
            serde_json::Value::String(s) => {
                if s.len() > MAX_PARAM_STRING_LENGTH {
                    Err(ParamValueConversionError::StringTooLong {
                        len: s.len(),
                        max: MAX_PARAM_STRING_LENGTH,
                    })
                } else {
                    Ok(ParamValue::String(s.clone()))
                }
            }
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    if let Ok(i32_val) = i32::try_from(i) {
                        Ok(ParamValue::Integer(i32_val))
                    } else {
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "widening i64 outside i32 range into f64 — caller has already validated finiteness at the JSON parser layer"
                        )]
                        Ok(ParamValue::Double(i as f64))
                    }
                } else if let Some(f) = n.as_f64() {
                    if f.is_finite() {
                        Ok(ParamValue::Double(f))
                    } else {
                        Err(ParamValueConversionError::NonFiniteNumber)
                    }
                } else {
                    Err(ParamValueConversionError::UnrepresentableNumber)
                }
            }
            serde_json::Value::Array(items) => Ok(ParamValue::List(
                items
                    .iter()
                    .map(ParamValue::try_from)
                    .collect::<Result<_, _>>()?,
            )),
            serde_json::Value::Object(fields) => Ok(ParamValue::Object(
                fields
                    .iter()
                    .map(|(key, value)| {
                        let key = ParamKey::try_new(key.clone())
                            .map_err(ParamValueConversionError::InvalidObjectKey)?;
                        Ok((key, ParamValue::try_from(value)?))
                    })
                    .collect::<Result<_, _>>()?,
            )),
        }
    }
}

/// As [`deserialize_unique_params`], for any value type. `what` names the key kind in the error:
/// `"param key"` yields `duplicate param key "theme"`.
pub fn deserialize_unique_keyed<'de, D, V>(
    deserializer: D,
    what: &'static str,
) -> Result<IndexMap<ParamKey, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct UniqueMapVisitor<V> {
        what: &'static str,
        value: PhantomData<V>,
    }

    impl<'de, V: Deserialize<'de>> Visitor<'de> for UniqueMapVisitor<V> {
        type Value = IndexMap<ParamKey, V>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "a map with unique {}s", self.what)
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut map = IndexMap::with_capacity(access.size_hint().unwrap_or(0));
            while let Some((key, value)) = access.next_entry::<ParamKey, V>()? {
                if map.contains_key(&key) {
                    return Err(M::Error::custom(format!(
                        "duplicate {} {:?}",
                        self.what,
                        key.as_str()
                    )));
                }
                map.insert(key, value);
            }
            Ok(map)
        }
    }

    deserializer.deserialize_map(UniqueMapVisitor {
        what,
        value: PhantomData,
    })
}

/// Deserialize a field map, rejecting duplicate keys instead of silently keeping the last.
pub fn deserialize_unique_params<'de, D>(
    deserializer: D,
) -> Result<IndexMap<ParamKey, ParamDefinition>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_unique_keyed(deserializer, "param key")
}

/// Per-field declaration inside a schema's field map.
/// The `kind` field carries the value-type-specific options (`enum_values`, `min`, `max`, etc.)
/// via a serde-flattened tagged enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParamDefinition {
    /// Human-readable field name, shown in the operator UI.
    pub name: String,
    /// Optional one-line field description, shown in the operator UI as help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the operator can leave this field unset.
    /// A required field must declare a `default_value`, except an `array`, whose omitted default is the empty list.
    /// The `default_value` is seeded at widget creation, and an unset optional field is delivered as `Null`.
    #[serde(
        default,
        rename = "optional",
        skip_serializing_if = "core::ops::Not::not"
    )]
    pub is_optional: bool,
    /// Value-kind-specific shape — discriminated on `type`.
    #[serde(flatten)]
    pub kind: ParamKind,
}

/// Tagged enum carrying the value-type-specific shape of a [`ParamDefinition`].
/// The discriminator field is `type`; variant names are lowercased on the wire
/// (`"string"`, `"double"`, `"integer"`, `"boolean"`, `"timezone"`, `"array"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ParamKind {
    /// A UTF-8 string.
    String(StringParam),
    /// A finite f64 — JSON Schema "number".
    Double(DoubleParam),
    /// A 32-bit signed integer — JSON Schema "integer" with i32 range.
    Integer(IntegerParam),
    /// A boolean.
    Boolean(BooleanParam),
    /// An IANA timezone identifier. Wire form is a string;
    /// the dedicated variant lets the operator UI render a zone picker instead of a free-form text input.
    Timezone(TimezoneParam),
    /// An ordered list the operator can add to, remove from and reorder.
    Array(ArrayParam),
}

/// The options of a [`ParamKind::Array`] field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct ArrayParam {
    /// What every item is. A newly added item starts at the item's `default_value`,
    /// or an object item at each field's; neither is required.
    pub items: ItemKind,
    /// Fewest items the list may hold.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(range(max = MAX_ARRAY_ITEMS))]
    pub min_items: usize,
    /// Most items the list may hold, capped at [`MAX_ARRAY_ITEMS`].
    #[schemars(range(min = 1, max = MAX_ARRAY_ITEMS))]
    pub max_items: usize,
    /// Items seeded at widget creation; must fit `min_items..=max_items`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_value: Vec<ParamValue>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands the predicate a reference"
)]
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The kind of an [`ArrayParam`]'s items, tagged like [`ParamKind`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ItemKind {
    /// A UTF-8 string.
    String(StringParam),
    /// A finite f64.
    Double(DoubleParam),
    /// A 32-bit signed integer.
    Integer(IntegerParam),
    /// A boolean.
    Boolean(BooleanParam),
    /// An IANA timezone identifier.
    Timezone(TimezoneParam),
    /// A row of named scalar fields.
    Object(ObjectParam),
}

/// The fields of an [`ItemKind::Object`] item, in display order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectParam {
    /// The row's fields, keyed like params.
    #[serde(deserialize_with = "deserialize_unique_fields")]
    pub fields: IndexMap<ParamKey, ObjectField>,
}

fn deserialize_unique_fields<'de, D>(
    deserializer: D,
) -> Result<IndexMap<ParamKey, ObjectField>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_unique_keyed(deserializer, "field key")
}

/// One field of an object item; always a scalar, so objects never nest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectField {
    /// Human-readable field name, shown in the operator UI.
    pub name: String,
    /// Optional one-line field description, shown in the operator UI as help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the operator can leave this field unset; an unset field is delivered as `Null` in its row.
    #[serde(
        default,
        rename = "optional",
        skip_serializing_if = "core::ops::Not::not"
    )]
    pub is_optional: bool,
    /// Value-kind-specific shape — discriminated on `type`.
    #[serde(flatten)]
    pub kind: ScalarKind,
}

/// The kind of an [`ObjectField`], tagged like [`ParamKind`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ScalarKind {
    /// A UTF-8 string.
    String(StringParam),
    /// A finite f64.
    Double(DoubleParam),
    /// A 32-bit signed integer.
    Integer(IntegerParam),
    /// A boolean.
    Boolean(BooleanParam),
    /// An IANA timezone identifier.
    Timezone(TimezoneParam),
}

/// A scalar field's options, borrowed from whichever kind holds them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scalar<'a> {
    String(&'a StringParam),
    Double(&'a DoubleParam),
    Integer(&'a IntegerParam),
    Boolean(&'a BooleanParam),
    Timezone(&'a TimezoneParam),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape<'a> {
    Scalar(Scalar<'a>),
    Array(&'a ArrayParam),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemShape<'a> {
    Scalar(Scalar<'a>),
    Object(&'a ObjectParam),
}

/// The options of a [`ParamKind::String`] field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct StringParam {
    /// Optional structural hint to the operator UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<StringFormat>,
    /// Optional closed set of allowed values.
    /// When non-empty, the `default_value` must be one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<StringOption>,
    /// The starting value, as the param, list item or object field holding these options defines it.
    /// Capped at [`MAX_PARAM_STRING_LENGTH`] bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = MAX_PARAM_STRING_LENGTH))]
    pub default_value: Option<String>,
    /// Example text shown in the empty input, such as "e.g. BTC or AAPL"; never a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// The options of a [`ParamKind::Double`] field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct DoubleParam {
    /// Inclusive lower bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Inclusive upper bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// UI step granularity. Strictly positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0.0))]
    pub step: Option<f64>,
    /// Optional closed set of allowed values.
    /// When non-empty, the `default_value` must be one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<DoubleOption>,
    /// The starting value, as the param, list item or object field holding these options defines it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<f64>,
    /// Example text shown in the empty input, such as "e.g. 0.5"; never a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// The options of a [`ParamKind::Integer`] field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct IntegerParam {
    /// Inclusive lower bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i32>,
    /// Inclusive upper bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i32>,
    /// UI step granularity. Strictly positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub step: Option<i32>,
    /// Optional closed set of allowed values.
    /// When non-empty, the `default_value` must be one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<IntegerOption>,
    /// The starting value, as the param, list item or object field holding these options defines it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<i32>,
    /// Example text shown in the empty input, such as "e.g. 42"; never a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// The options of a [`ParamKind::Boolean`] field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct BooleanParam {
    /// The starting value, as the param, list item or object field holding these options defines it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<bool>,
}

/// The options of a [`ParamKind::Timezone`] field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct TimezoneParam {
    /// The starting zone, as the param, list item or object field holding these options defines it.
    /// Capped at [`MAX_PARAM_STRING_LENGTH`] bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = MAX_PARAM_STRING_LENGTH))]
    pub default_value: Option<String>,
    /// Example text shown in the empty input, such as "e.g. Europe/Prague"; never a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

impl ParamDefinition {
    /// Stores a list's default as the validator projects it,
    /// so it reaches every consumer in the shape an operator's list would.
    /// A default the validator refuses stays as written, for [`Self::validate`] to report.
    pub fn normalize(&mut self) {
        if let ParamKind::Array(array) = &mut self.kind
            && let Ok(items) = array.projected_default()
        {
            array.default_value = items;
        }
    }

    /// Also refuses a list default that [`Self::normalize`] would still change.
    pub fn validate(&self, name: &str) -> Result<(), FieldSchemaError> {
        let invalid = |reason: String| FieldSchemaError::InvalidParam {
            name: name.to_owned(),
            reason,
        };

        if self.is_optional && matches!(self.kind, ParamKind::Array(_)) {
            return Err(invalid(
                "array params cannot be optional; min_items: 0 allows an empty list".into(),
            ));
        }
        if !self.is_optional && !self.kind.has_default_value() {
            return Err(invalid("required param needs default_value".into()));
        }
        self.kind.validate(name)
    }
}

impl ParamKind {
    #[must_use]
    pub fn shape(&self) -> Shape<'_> {
        match self {
            ParamKind::String(p) => Shape::Scalar(Scalar::String(p)),
            ParamKind::Double(p) => Shape::Scalar(Scalar::Double(p)),
            ParamKind::Integer(p) => Shape::Scalar(Scalar::Integer(p)),
            ParamKind::Boolean(p) => Shape::Scalar(Scalar::Boolean(p)),
            ParamKind::Timezone(p) => Shape::Scalar(Scalar::Timezone(p)),
            ParamKind::Array(p) => Shape::Array(p),
        }
    }

    fn has_default_value(&self) -> bool {
        match self {
            ParamKind::String(StringParam { default_value, .. })
            | ParamKind::Timezone(TimezoneParam { default_value, .. }) => default_value.is_some(),
            ParamKind::Double(DoubleParam { default_value, .. }) => default_value.is_some(),
            ParamKind::Integer(IntegerParam { default_value, .. }) => default_value.is_some(),
            ParamKind::Boolean(BooleanParam { default_value }) => default_value.is_some(),
            ParamKind::Array(_) => true,
        }
    }

    fn validate(&self, name: &str) -> Result<(), FieldSchemaError> {
        match self {
            ParamKind::String(p) => Scalar::String(p).validate(),
            ParamKind::Double(p) => Scalar::Double(p).validate(),
            ParamKind::Integer(p) => Scalar::Integer(p).validate(),
            ParamKind::Boolean(p) => Scalar::Boolean(p).validate(),
            ParamKind::Timezone(p) => Scalar::Timezone(p).validate(),
            ParamKind::Array(array) => array.validate(),
        }
        .map_err(|reason| FieldSchemaError::InvalidParam {
            name: name.to_owned(),
            reason,
        })
    }
}

impl ItemKind {
    #[must_use]
    pub fn shape(&self) -> ItemShape<'_> {
        match self {
            ItemKind::String(p) => ItemShape::Scalar(Scalar::String(p)),
            ItemKind::Double(p) => ItemShape::Scalar(Scalar::Double(p)),
            ItemKind::Integer(p) => ItemShape::Scalar(Scalar::Integer(p)),
            ItemKind::Boolean(p) => ItemShape::Scalar(Scalar::Boolean(p)),
            ItemKind::Timezone(p) => ItemShape::Scalar(Scalar::Timezone(p)),
            ItemKind::Object(p) => ItemShape::Object(p),
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self.shape() {
            ItemShape::Scalar(scalar) => scalar.validate(),
            ItemShape::Object(object) => object.validate(),
        }
    }
}

impl ScalarKind {
    #[must_use]
    pub fn as_scalar(&self) -> Scalar<'_> {
        match self {
            ScalarKind::String(p) => Scalar::String(p),
            ScalarKind::Double(p) => Scalar::Double(p),
            ScalarKind::Integer(p) => Scalar::Integer(p),
            ScalarKind::Boolean(p) => Scalar::Boolean(p),
            ScalarKind::Timezone(p) => Scalar::Timezone(p),
        }
    }
}

impl ObjectParam {
    fn validate(&self) -> Result<(), String> {
        if self.fields.is_empty() {
            return Err("object items need at least one field".into());
        }
        for (key, field) in &self.fields {
            field
                .kind
                .as_scalar()
                .validate()
                .map_err(|reason| format!("field {:?}: {reason}", key.as_str()))?;
        }
        Ok(())
    }
}

impl<'a> Scalar<'a> {
    #[must_use]
    pub fn placeholder(self) -> Option<&'a str> {
        match self {
            Scalar::String(p) => p.placeholder.as_deref(),
            Scalar::Double(p) => p.placeholder.as_deref(),
            Scalar::Integer(p) => p.placeholder.as_deref(),
            Scalar::Timezone(p) => p.placeholder.as_deref(),
            Scalar::Boolean(_) => None,
        }
    }

    fn validate(self) -> Result<(), String> {
        match self {
            Scalar::String(p) => p.validate(),
            Scalar::Double(p) => p.validate(),
            Scalar::Integer(p) => p.validate(),
            Scalar::Boolean(_) => Ok(()),
            Scalar::Timezone(p) => p.validate(),
        }
    }
}

impl ArrayParam {
    /// The default as the validator types it:
    /// a whole number for a double as a double, an omitted optional row field as null.
    fn projected_default(&self) -> Result<Vec<ParamValue>, String> {
        let mut projected = Vec::with_capacity(self.default_value.len());
        for (i, item) in self.default_value.iter().enumerate() {
            let mut violations = Vec::new();
            let value = validate::validate_item(
                &format!("default_value[{i}]"),
                &self.items,
                item,
                &mut violations,
            );
            if let Some(violation) = violations.into_iter().next() {
                return Err(format!("{}: {}", violation.path, violation.message));
            }
            projected.push(value.expect("BUG: an item without violations has a projection"));
        }
        Ok(projected)
    }

    fn validate(&self) -> Result<(), String> {
        self.items.validate()?;
        if !(1..=MAX_ARRAY_ITEMS).contains(&self.max_items) {
            return Err(format!(
                "max_items must be within 1..={MAX_ARRAY_ITEMS} (got {})",
                self.max_items
            ));
        }
        if self.min_items > self.max_items {
            return Err(format!(
                "min_items ({}) > max_items ({})",
                self.min_items, self.max_items
            ));
        }
        let len = self.default_value.len();
        if !(self.min_items..=self.max_items).contains(&len) {
            return Err(format!(
                "default_value has {len} items, outside min_items..=max_items ({}..={})",
                self.min_items, self.max_items
            ));
        }
        if self.projected_default()? != self.default_value {
            return Err("default_value is not normalized; normalize the param first".into());
        }
        Ok(())
    }
}

impl StringParam {
    fn validate(&self) -> Result<(), String> {
        check_string_options(&self.enum_values)?;
        check_string_default_length(self.default_value.as_deref())?;
        if !self.enum_values.is_empty()
            && let Some(d) = &self.default_value
            && !self.enum_values.iter().any(|o| &o.value == d)
        {
            return Err(format!("default_value {d:?} not in enum_values"));
        }
        Ok(())
    }
}

impl DoubleParam {
    fn validate(&self) -> Result<(), String> {
        check_finite(self.default_value, "default_value")?;
        check_finite(self.min, "min")?;
        check_finite(self.max, "max")?;
        check_finite(self.step, "step")?;
        for o in &self.enum_values {
            check_finite(Some(o.value), "enum_values[].value")?;
        }
        check_double_range(self.min, self.max, self.step, self.default_value)?;
        check_double_options(&self.enum_values, self.default_value)
    }
}

impl IntegerParam {
    fn validate(&self) -> Result<(), String> {
        check_int_range(self.min, self.max, self.step, self.default_value)?;
        check_int_options(&self.enum_values, self.default_value)
    }
}

impl TimezoneParam {
    fn validate(&self) -> Result<(), String> {
        check_string_default_length(self.default_value.as_deref())
    }
}

fn check_string_default_length(default_value: Option<&str>) -> Result<(), String> {
    match default_value {
        Some(d) if d.len() > MAX_PARAM_STRING_LENGTH => Err(format!(
            "default_value exceeds max length of {MAX_PARAM_STRING_LENGTH} bytes (got {})",
            d.len()
        )),
        _ => Ok(()),
    }
}

/// Canonicalise an f64 for bit-equality comparison: collapses `+0.0` and `-0.0` to the same key.
/// NaNs keep their bit pattern; range and finite-ness checks elsewhere reject configured NaNs.
#[must_use]
pub fn f64_canonical_bits(v: f64) -> u64 {
    if v == 0.0 { 0_u64 } else { v.to_bits() }
}

fn check_string_options(options: &[StringOption]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for o in options {
        if o.value.is_empty() {
            return Err(
                "enum_values entry value must be non-empty (collides with FE \"no selection\" sentinel)"
                    .into(),
            );
        }
        if o.value.len() > MAX_PARAM_STRING_LENGTH {
            return Err(format!(
                "enum_values entry value exceeds max length of {MAX_PARAM_STRING_LENGTH} bytes (got {})",
                o.value.len()
            ));
        }
        if o.label.trim().is_empty() {
            return Err("enum_values entry label must be non-empty after trim".into());
        }
        if !seen.insert(o.value.as_str()) {
            return Err(format!("duplicate enum_values entry value {:?}", o.value));
        }
    }
    Ok(())
}

fn check_double_options(
    options: &[DoubleOption],
    default_value: Option<f64>,
) -> Result<(), String> {
    for o in options {
        if o.label.trim().is_empty() {
            return Err("enum_values entry label must be non-empty after trim".into());
        }
    }
    for (i, a) in options.iter().enumerate() {
        for b in &options[i + 1..] {
            if f64_canonical_bits(a.value) == f64_canonical_bits(b.value) {
                return Err(format!("duplicate enum_values entry value {}", a.value));
            }
        }
    }
    if !options.is_empty()
        && let Some(d) = default_value
        && !options
            .iter()
            .any(|o| f64_canonical_bits(o.value) == f64_canonical_bits(d))
    {
        return Err(format!("default_value {d} not in enum_values"));
    }
    Ok(())
}

fn check_int_options(options: &[IntegerOption], default_value: Option<i32>) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for o in options {
        if o.label.trim().is_empty() {
            return Err("enum_values entry label must be non-empty after trim".into());
        }
        if !seen.insert(o.value) {
            return Err(format!("duplicate enum_values entry value {}", o.value));
        }
    }
    if !options.is_empty()
        && let Some(d) = default_value
        && !options.iter().any(|o| o.value == d)
    {
        return Err(format!("default_value {d} not in enum_values"));
    }
    Ok(())
}

fn check_finite(v: Option<f64>, what: &str) -> Result<(), String> {
    match v {
        Some(x) if !x.is_finite() => Err(format!("{what} must be finite (got {x})")),
        _ => Ok(()),
    }
}

fn check_double_range(
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    default_value: Option<f64>,
) -> Result<(), String> {
    if let Some(s) = step
        && s <= 0.0
    {
        return Err(format!("step must be > 0 (got {s})"));
    }
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(format!("min ({lo}) > max ({hi})"));
    }
    if let (Some(d), Some(lo)) = (default_value, min)
        && d < lo
    {
        return Err(format!("default_value {d} < min {lo}"));
    }
    if let (Some(d), Some(hi)) = (default_value, max)
        && d > hi
    {
        return Err(format!("default_value {d} > max {hi}"));
    }
    Ok(())
}

fn check_int_range(
    min: Option<i32>,
    max: Option<i32>,
    step: Option<i32>,
    default_value: Option<i32>,
) -> Result<(), String> {
    if let Some(s) = step
        && s <= 0
    {
        return Err(format!("step must be > 0 (got {s})"));
    }
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(format!("min ({lo}) > max ({hi})"));
    }
    if let (Some(d), Some(lo)) = (default_value, min)
        && d < lo
    {
        return Err(format!("default_value {d} < min {lo}"));
    }
    if let (Some(d), Some(hi)) = (default_value, max)
        && d > hi
    {
        return Err(format!("default_value {d} > max {hi}"));
    }
    Ok(())
}
