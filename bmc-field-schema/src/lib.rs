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
//! JSON-Schema-expressible constraints ride on `schemars` attributes.
//! Cross-field invariants (`default_value` in `[min, max]` / in `enum_values`, `±0.0` enum collision)
//! live in [`ParamDefinition::validate`],
//! except those on keys — duplicates and the fields `unique_items` names — which parsing refuses.

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

/// How the operator UI presents a field's `enum_values`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnumControl {
    /// A dropdown, which suits any number of options.
    #[default]
    Dropdown,
    /// Every option on show as a radio group, which suits a few short ones.
    Radio,
}

impl EnumControl {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if hands the field by reference"
    )]
    fn is_dropdown(&self) -> bool {
        *self == Self::Dropdown
    }
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
            Shape::Scalar(scalar) => scalar.default_value(),
            Shape::Array(ArrayParam { default_value, .. }) => {
                ParamValue::List(default_value.clone())
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

/// As [`deserialize_unique_params`], for any value type. `what` names the key kind in errors:
/// `"param key"` yields `duplicate param key "theme"`, or `param key "theme": …` for a bad value.
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
            while let Some(key) = access.next_key::<ParamKey>()? {
                if map.contains_key(&key) {
                    return Err(M::Error::custom(format!(
                        "duplicate {} {:?}",
                        self.what,
                        key.as_str()
                    )));
                }
                let value = access.next_value::<V>().map_err(|error| {
                    M::Error::custom(format!("{} {:?}: {error}", self.what, key.as_str()))
                })?;
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
    Array(#[schemars(with = "ArrayParamRepr")] ArrayParam),
}

/// The options of a [`ParamKind::Array`] field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ArrayParamRepr", into = "ArrayParamRepr")]
pub struct ArrayParam {
    /// What every item is, and which items count as repeats of each other.
    pub items: ListItems,
    /// Fewest items the list may hold.
    pub min_items: usize,
    /// Most items the list may hold, capped at [`MAX_ARRAY_ITEMS`].
    pub max_items: usize,
    /// Items seeded at widget creation; must fit `min_items..=max_items`.
    pub default_value: Vec<ParamValue>,
}

/// The options of a [`ParamKind::Array`] field.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
struct ArrayParamRepr {
    /// What every item is. A newly added item starts at the item's `default_value`,
    /// or an object item at each field's; neither is required.
    items: ItemKind,
    /// Fewest items the list may hold.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(range(max = MAX_ARRAY_ITEMS))]
    min_items: usize,
    /// Most items the list may hold, capped at [`MAX_ARRAY_ITEMS`].
    #[schemars(range(min = 1, max = MAX_ARRAY_ITEMS))]
    max_items: usize,
    /// Refuse repeated items: `true` compares whole items,
    /// a list of field keys compares object rows on those fields only.
    #[serde(default, skip_serializing_if = "UniqueItemsRepr::is_off")]
    unique_items: UniqueItemsRepr,
    /// Items seeded at widget creation; must fit `min_items..=max_items`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    default_value: Vec<ParamValue>,
}

impl TryFrom<ArrayParamRepr> for ArrayParam {
    type Error = String;

    fn try_from(repr: ArrayParamRepr) -> Result<Self, String> {
        let unique = repr.unique_items;
        let items = match repr.items {
            ItemKind::String(p) => ListItems::scalar(ScalarKind::String(p), &unique),
            ItemKind::Double(p) => ListItems::scalar(ScalarKind::Double(p), &unique),
            ItemKind::Integer(p) => ListItems::scalar(ScalarKind::Integer(p), &unique),
            ItemKind::Boolean(p) => ListItems::scalar(ScalarKind::Boolean(p), &unique),
            ItemKind::Timezone(p) => ListItems::scalar(ScalarKind::Timezone(p), &unique),
            ItemKind::Object(object) => ListItems::object(object, unique),
        }?;
        Ok(Self {
            items,
            min_items: repr.min_items,
            max_items: repr.max_items,
            default_value: repr.default_value,
        })
    }
}

impl From<ArrayParam> for ArrayParamRepr {
    fn from(array: ArrayParam) -> Self {
        let (items, unique_items) = match array.items {
            ListItems::Scalar { kind, unique } => {
                (ItemKind::from(kind), UniqueItemsRepr::Flag(unique))
            }
            ListItems::Object { object, unique } => (
                ItemKind::Object(object),
                match unique {
                    RowUniqueness::Off => UniqueItemsRepr::Flag(false),
                    RowUniqueness::Whole => UniqueItemsRepr::Flag(true),
                    RowUniqueness::By(keys) => {
                        UniqueItemsRepr::Keys(keys.into_iter().map(|key| key.0).collect())
                    }
                },
            ),
        };
        Self {
            items,
            min_items: array.min_items,
            max_items: array.max_items,
            unique_items,
            default_value: array.default_value,
        }
    }
}

/// What every item of an [`ArrayParam`] is, and which items count as repeats of each other.
/// A newly added item starts at the item's `default_value`, or an object item at each field's.
#[derive(Debug, Clone, PartialEq)]
pub enum ListItems {
    /// `unique` refuses an item equal to an earlier one.
    Scalar { kind: ScalarKind, unique: bool },
    /// A row of named scalar fields.
    Object {
        object: ObjectParam,
        unique: RowUniqueness,
    },
}

/// Which object rows count as repeats of each other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RowUniqueness {
    /// Repeats are allowed.
    #[default]
    Off,
    /// No row may equal an earlier one on every field.
    Whole,
    /// No row may match an earlier one on all of these fields:
    /// at least one, each a declared field, none twice.
    By(Vec<ParamKey>),
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands the predicate a reference"
)]
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Whether a list refuses repeated items,
/// and which fields of an object row make one.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(
    untagged,
    expecting = "unique_items as true, false or a list of field keys"
)]
#[schemars(rename = "UniqueItems")]
enum UniqueItemsRepr {
    /// `true` compares whole items;
    /// `false` allows repeats.
    Flag(bool),
    /// The fields an object row is compared on.
    // Read as plain text so a malformed key is named, which the untagged parse would swallow.
    Keys(#[schemars(with = "Vec<ParamKey>")] Vec<String>),
}

impl Default for UniqueItemsRepr {
    fn default() -> Self {
        Self::Flag(false)
    }
}

impl UniqueItemsRepr {
    fn is_off(&self) -> bool {
        matches!(self, Self::Flag(false))
    }
}

/// The kind of an [`ArrayParam`]'s items, tagged like [`ParamKind`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ItemKind {
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

/// The fields of an object list item, in display order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectParam {
    /// The row's fields, keyed like params.
    #[serde(deserialize_with = "deserialize_unique_fields")]
    pub fields: IndexMap<ParamKey, ScalarField>,
}

fn deserialize_unique_fields<'de, D>(
    deserializer: D,
) -> Result<IndexMap<ParamKey, ScalarField>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_unique_keyed(deserializer, "field key")
}

/// A named field that is always a scalar: a field of an object item, or of a credential type.
/// Being scalar is what keeps objects from nesting and credentials to one piece of text each.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScalarField {
    /// Human-readable field name, shown in the operator UI.
    pub name: String,
    /// Optional one-line field description, shown in the operator UI as help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the operator can leave this field unset, which holds it as `Null`.
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

/// The kind of a [`ScalarField`], tagged like [`ParamKind`].
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
    /// Fewest characters (Unicode code points) a value may hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = MAX_PARAM_STRING_LENGTH))]
    pub min_length: Option<usize>,
    /// Most characters (Unicode code points) a value may hold, up to [`MAX_PARAM_STRING_LENGTH`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = MAX_PARAM_STRING_LENGTH))]
    pub max_length: Option<usize>,
    /// Optional closed set of allowed values.
    /// When non-empty, the `default_value` must be one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<StringOption>,
    /// How the UI presents `enum_values`; needs them to be set.
    #[serde(default, skip_serializing_if = "EnumControl::is_dropdown")]
    pub enum_control: EnumControl,
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
    /// What the number counts, such as "s" or "%"; the operator UI shows it with the field's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub unit: Option<String>,
    /// Optional closed set of allowed values.
    /// When non-empty, the `default_value` must be one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<DoubleOption>,
    /// How the UI presents `enum_values`; needs them to be set.
    #[serde(default, skip_serializing_if = "EnumControl::is_dropdown")]
    pub enum_control: EnumControl,
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
    /// What the number counts, such as "s" or "%"; the operator UI shows it with the field's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub unit: Option<String>,
    /// Optional closed set of allowed values.
    /// When non-empty, the `default_value` must be one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<IntegerOption>,
    /// How the UI presents `enum_values`; needs them to be set.
    #[serde(default, skip_serializing_if = "EnumControl::is_dropdown")]
    pub enum_control: EnumControl,
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
        self.kind.validate(name, !self.is_optional)
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

    fn validate(&self, name: &str, required: bool) -> Result<(), FieldSchemaError> {
        match self.shape() {
            Shape::Scalar(scalar) => scalar.validate(required),
            Shape::Array(array) => array.validate(),
        }
        .map_err(|reason| FieldSchemaError::InvalidParam {
            name: name.to_owned(),
            reason,
        })
    }
}

impl ListItems {
    fn scalar(kind: ScalarKind, unique: &UniqueItemsRepr) -> Result<Self, String> {
        let unique = match unique {
            UniqueItemsRepr::Flag(unique) => *unique,
            UniqueItemsRepr::Keys(_) => {
                return Err(
                    "unique_items names fields, which scalar items lack; true compares whole items"
                        .into(),
                );
            }
        };
        Ok(Self::Scalar { kind, unique })
    }

    fn object(object: ObjectParam, unique: UniqueItemsRepr) -> Result<Self, String> {
        let unique = match unique {
            UniqueItemsRepr::Flag(false) => RowUniqueness::Off,
            UniqueItemsRepr::Flag(true) => RowUniqueness::Whole,
            UniqueItemsRepr::Keys(keys) => {
                let keys = keys
                    .into_iter()
                    .map(|key| {
                        ParamKey::try_new(key).map_err(|key| {
                            format!("unique_items names an invalid param key {key:?}")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                check_unique_keys(&keys, &object)?;
                RowUniqueness::By(keys)
            }
        };
        Ok(Self::Object { object, unique })
    }

    #[must_use]
    pub fn shape(&self) -> ItemShape<'_> {
        match self {
            ListItems::Scalar { kind, .. } => ItemShape::Scalar(kind.as_scalar()),
            ListItems::Object { object, .. } => ItemShape::Object(object),
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self.shape() {
            ItemShape::Scalar(scalar) => scalar.validate(true),
            ItemShape::Object(object) => object.validate(),
        }
    }
}

impl From<ScalarKind> for ItemKind {
    fn from(kind: ScalarKind) -> Self {
        match kind {
            ScalarKind::String(p) => ItemKind::String(p),
            ScalarKind::Double(p) => ItemKind::Double(p),
            ScalarKind::Integer(p) => ItemKind::Integer(p),
            ScalarKind::Boolean(p) => ItemKind::Boolean(p),
            ScalarKind::Timezone(p) => ItemKind::Timezone(p),
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
                .validate(!field.is_optional)
                .map_err(|reason| format!("field {:?}: {reason}", key.as_str()))?;
        }
        Ok(())
    }
}

impl<'a> Scalar<'a> {
    /// `Null` when no default is declared.
    #[must_use]
    pub fn default_value(self) -> ParamValue {
        match self {
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

    fn validate(self, required: bool) -> Result<(), String> {
        match self {
            Scalar::String(p) => p.validate(),
            Scalar::Double(p) => p.validate(),
            Scalar::Integer(p) => p.validate(),
            Scalar::Boolean(_) | Scalar::Timezone(_) => Ok(()),
        }?;
        let default = self.default_value();
        if required && matches!(&default, ParamValue::String(s) if s.is_empty()) {
            return Err("default_value: Value is required".into());
        }
        if matches!(default, ParamValue::Null) {
            return Ok(());
        }
        let mut violations = Vec::new();
        validate::validate_scalar("default_value", self, &default, &mut violations);
        match violations.into_iter().next() {
            Some(violation) => Err(format!("{}: {}", violation.path, violation.message)),
            None => Ok(()),
        }
    }
}

impl ArrayParam {
    /// The default as the validator types it:
    /// a whole number for a double as a double, an omitted optional row field as null.
    fn projected_default(&self) -> Result<Vec<ParamValue>, String> {
        let mut violations = Vec::new();
        validate::validate_list_items("default_value", self, &self.default_value, &mut violations)
            .ok_or_else(|| {
                violations
                    .iter()
                    .map(|violation| format!("{}: {}", violation.path, violation.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
    }

    fn validate(&self) -> Result<(), String> {
        self.items
            .validate()
            .map_err(|reason| format!("items: {reason}"))?;
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
        self.check_unique_defaults()?;
        if self.projected_default()? != self.default_value {
            return Err("default_value is not normalized; normalize the param first".into());
        }
        Ok(())
    }

    fn check_unique_defaults(&self) -> Result<(), String> {
        let has_default = |scalar: Scalar<'_>| scalar.default_value() != ParamValue::Null;
        match &self.items {
            ListItems::Scalar { kind, unique } => {
                if *unique && has_default(kind.as_scalar()) {
                    return Err("an item default_value cannot go with unique_items: \
                        every added item would start as a repeat"
                        .into());
                }
            }
            ListItems::Object { object, unique } => {
                if let RowUniqueness::By(keys) = unique {
                    let defaulted = keys.iter().find(|key| {
                        object
                            .fields
                            .get(*key)
                            .is_some_and(|field| has_default(field.kind.as_scalar()))
                    });
                    if let Some(key) = defaulted {
                        return Err(format!(
                            "key {:?} cannot have a default_value under unique_items: \
                            every added row would start as a repeat",
                            key.as_str()
                        ));
                    }
                }
                let starts_blank = |field: &ScalarField| {
                    !field.is_optional
                        && !matches!(field.kind, ScalarKind::Boolean(_))
                        && !has_default(field.kind.as_scalar())
                };
                if !matches!(unique, RowUniqueness::Off)
                    && !object.fields.values().any(starts_blank)
                {
                    return Err(
                        "unique_items needs a required field a new row leaves blank: \
                        otherwise every added row would start as a repeat"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }
}

fn check_unique_keys(keys: &[ParamKey], object: &ObjectParam) -> Result<(), String> {
    if keys.is_empty() {
        return Err("unique_items needs a field key; true compares whole rows".into());
    }
    for (i, key) in keys.iter().enumerate() {
        if !object.fields.contains_key(key) {
            return Err(format!(
                "unique_items names {:?}, which is not a field",
                key.as_str()
            ));
        }
        if keys.iter().take(i).any(|earlier| earlier == key) {
            return Err(format!("unique_items names {:?} twice", key.as_str()));
        }
    }
    Ok(())
}

impl StringParam {
    fn validate(&self) -> Result<(), String> {
        check_string_options(&self.enum_values)?;
        check_enum_control(self.enum_control, !self.enum_values.is_empty())?;
        check_length_bounds(self.min_length, self.max_length)?;
        for o in &self.enum_values {
            if let Some(message) =
                validate::length_violation(&o.value, self.min_length, self.max_length)
            {
                return Err(format!("enum_values {:?}: {message}", o.value));
            }
        }
        Ok(())
    }
}

fn check_length_bounds(min: Option<usize>, max: Option<usize>) -> Result<(), String> {
    if let Some(lo) = min
        && lo > MAX_PARAM_STRING_LENGTH
    {
        return Err(format!(
            "min_length must be at most {MAX_PARAM_STRING_LENGTH} (got {lo})"
        ));
    }
    if let Some(hi) = max
        && !(1..=MAX_PARAM_STRING_LENGTH).contains(&hi)
    {
        return Err(format!(
            "max_length must be within 1..={MAX_PARAM_STRING_LENGTH} (got {hi})"
        ));
    }
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(format!("min_length ({lo}) > max_length ({hi})"));
    }
    Ok(())
}

impl DoubleParam {
    fn validate(&self) -> Result<(), String> {
        check_finite(self.min, "min")?;
        check_finite(self.max, "max")?;
        check_finite(self.step, "step")?;
        for o in &self.enum_values {
            check_finite(Some(o.value), "enum_values[].value")?;
        }
        check_double_range(self.min, self.max, self.step)?;
        check_enum_control(self.enum_control, !self.enum_values.is_empty())?;
        check_unit(self.unit.as_deref())?;
        check_double_options(&self.enum_values)?;
        for o in &self.enum_values {
            if let Some(message) = validate::bound_violation(o.value, self.min, self.max) {
                return Err(format!("enum_values {}: {message}", o.value));
            }
        }
        Ok(())
    }
}

impl IntegerParam {
    fn validate(&self) -> Result<(), String> {
        check_int_range(self.min, self.max, self.step)?;
        check_enum_control(self.enum_control, !self.enum_values.is_empty())?;
        check_unit(self.unit.as_deref())?;
        check_int_options(&self.enum_values)?;
        for o in &self.enum_values {
            if let Some(message) = validate::bound_violation(o.value, self.min, self.max) {
                return Err(format!("enum_values {}: {message}", o.value));
            }
        }
        Ok(())
    }
}

fn check_unit(unit: Option<&str>) -> Result<(), String> {
    match unit {
        Some(unit) if unit.trim().is_empty() => Err(String::from("unit must not be blank")),
        Some(_) | None => Ok(()),
    }
}

fn check_enum_control(control: EnumControl, has_options: bool) -> Result<(), String> {
    match control {
        EnumControl::Radio if !has_options => {
            Err(String::from("enum_control radio needs enum_values"))
        }
        EnumControl::Dropdown | EnumControl::Radio => Ok(()),
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

fn check_double_options(options: &[DoubleOption]) -> Result<(), String> {
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
    Ok(())
}

fn check_int_options(options: &[IntegerOption]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for o in options {
        if o.label.trim().is_empty() {
            return Err("enum_values entry label must be non-empty after trim".into());
        }
        if !seen.insert(o.value) {
            return Err(format!("duplicate enum_values entry value {}", o.value));
        }
    }
    Ok(())
}

fn check_finite(v: Option<f64>, what: &str) -> Result<(), String> {
    match v {
        Some(x) if !x.is_finite() => Err(format!("{what} must be finite (got {x})")),
        _ => Ok(()),
    }
}

fn check_double_range(min: Option<f64>, max: Option<f64>, step: Option<f64>) -> Result<(), String> {
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
    Ok(())
}

fn check_int_range(min: Option<i32>, max: Option<i32>, step: Option<i32>) -> Result<(), String> {
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
    Ok(())
}
