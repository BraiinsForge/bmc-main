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

use std::marker::PhantomData;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub mod credential;

/// Maximum byte length of a [`ParamKey`]. Under the wire-format `u16` length field, so the
/// encoder's `u16::try_from` is statically infallible.
pub const MAX_PARAM_KEY_LENGTH: usize = 64;

/// Maximum byte length of any string-shaped value. Under the wire-format `u32` length field, so the
/// encoder's `u32::try_from` is statically infallible.
pub const MAX_PARAM_STRING_LENGTH: usize = 1024;

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

/// Typed scalar value for a stored field — the [`ParamKind`] value space (null, bool, i32, finite
/// f64, string). Both the compositor's in-memory form and the wire shape sent to widgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ParamValue {
    /// Absence of a value. Sent for optional params the operator cleared.
    Null,
    /// A boolean.
    Boolean(bool),
    /// An i32 — the integer width the manifest declares.
    Integer(i32),
    /// A finite f64. NaN / ±infinity are rejected at parse time.
    #[serde(deserialize_with = "deserialize_finite_f64")]
    Double(f64),
    /// A UTF-8 string.
    String(String),
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

impl ParamValue {
    /// JSON projection for the wayland boundary — bare scalars, not the internally-tagged form.
    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            ParamValue::Null => serde_json::Value::Null,
            ParamValue::Boolean(b) => serde_json::Value::Bool(*b),
            ParamValue::Integer(i) => serde_json::Value::Number((*i).into()),
            ParamValue::Double(d) => serde_json::Number::from_f64(*d)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            ParamValue::String(s) => serde_json::Value::String(s.clone()),
        }
    }

    /// Build the default value for a field; optional fields without a default yield `Null`.
    #[must_use]
    pub fn from_param_kind_default(kind: &ParamKind) -> Self {
        match kind {
            ParamKind::String { default_value, .. } | ParamKind::Timezone { default_value } => {
                default_value
                    .clone()
                    .map_or(ParamValue::Null, ParamValue::String)
            }
            ParamKind::Double { default_value, .. } => {
                default_value.map_or(ParamValue::Null, ParamValue::Double)
            }
            ParamKind::Integer { default_value, .. } => {
                default_value.map_or(ParamValue::Null, ParamValue::Integer)
            }
            ParamKind::Boolean { default_value } => {
                default_value.map_or(ParamValue::Null, ParamValue::Boolean)
            }
        }
    }
}

/// Reasons the wayland-side JSON-to-[`ParamValue`] conversion can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ParamValueConversionError {
    /// Expected a scalar, got an array.
    #[error("expected scalar, got JSON array")]
    Array,
    /// Expected a scalar, got an object.
    #[error("expected scalar, got JSON object")]
    Object,
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

/// Inverse of [`ParamValue::to_json_value`] — re-types wayland-edge JSON into a scalar
/// [`ParamValue`], erroring on objects, arrays, and non-finite numbers.
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
            serde_json::Value::Array(_) => Err(ParamValueConversionError::Array),
            serde_json::Value::Object(_) => Err(ParamValueConversionError::Object),
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
    /// Optional fields may have a default; required fields *must* (and the host always delivers a value).
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
/// (`"string"`, `"double"`, `"integer"`, `"boolean"`, `"timezone"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ParamKind {
    /// A UTF-8 string.
    String {
        /// Optional structural hint to the operator UI.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<StringFormat>,
        /// Optional closed set of allowed values.
        /// When non-empty, the `default_value` must be one of these.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        enum_values: Vec<StringOption>,
        /// Initial value seeded at widget creation; later updates with the operator field unset are delivered as `Null`.
        /// Required when `optional == false`. Capped at [`MAX_PARAM_STRING_LENGTH`] bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(length(max = MAX_PARAM_STRING_LENGTH))]
        default_value: Option<String>,
    },
    /// A finite f64 — JSON Schema "number".
    Double {
        /// Inclusive lower bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        /// Inclusive upper bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        /// UI step granularity. Strictly positive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("exclusiveMinimum" = 0.0))]
        step: Option<f64>,
        /// Optional closed set of allowed values.
        /// When non-empty, the `default_value` must be one of these.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        enum_values: Vec<DoubleOption>,
        /// Initial value seeded at widget creation; later updates with the operator field unset are delivered as `Null`.
        /// Required when `optional == false`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<f64>,
    },
    /// A 32-bit signed integer — JSON Schema "integer" with i32 range.
    Integer {
        /// Inclusive lower bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i32>,
        /// Inclusive upper bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i32>,
        /// UI step granularity. Strictly positive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("exclusiveMinimum" = 0))]
        step: Option<i32>,
        /// Optional closed set of allowed values.
        /// When non-empty, the `default_value` must be one of these.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        enum_values: Vec<IntegerOption>,
        /// Initial value seeded at widget creation; later updates with the operator field unset are delivered as `Null`.
        /// Required when `optional == false`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<i32>,
    },
    /// A boolean.
    Boolean {
        /// Initial value seeded at widget creation; later updates with the operator field unset are delivered as `Null`.
        /// Required when `optional == false`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<bool>,
    },
    /// An IANA timezone identifier. Wire form is a string;
    /// the dedicated variant lets the operator UI render a zone picker instead of a free-form text input.
    Timezone {
        /// Initial zone seeded at widget creation; later updates with the operator field unset are delivered as `Null`.
        /// Required when `optional == false`. Capped at [`MAX_PARAM_STRING_LENGTH`] bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(length(max = MAX_PARAM_STRING_LENGTH))]
        default_value: Option<String>,
    },
}

impl ParamDefinition {
    pub fn validate(&self, name: &str) -> Result<(), FieldSchemaError> {
        let invalid = |reason: String| FieldSchemaError::InvalidParam {
            name: name.to_owned(),
            reason,
        };

        if !self.is_optional && !self.kind.has_default_value() {
            return Err(invalid("required param needs default_value".into()));
        }
        self.kind.validate(name)
    }
}

impl ParamKind {
    fn has_default_value(&self) -> bool {
        match self {
            ParamKind::String { default_value, .. } | ParamKind::Timezone { default_value } => {
                default_value.is_some()
            }
            ParamKind::Double { default_value, .. } => default_value.is_some(),
            ParamKind::Integer { default_value, .. } => default_value.is_some(),
            ParamKind::Boolean { default_value } => default_value.is_some(),
        }
    }

    fn validate(&self, name: &str) -> Result<(), FieldSchemaError> {
        let invalid = |reason: String| FieldSchemaError::InvalidParam {
            name: name.to_owned(),
            reason,
        };

        match self {
            ParamKind::String {
                enum_values,
                default_value,
                ..
            } => {
                check_string_options(enum_values).map_err(&invalid)?;
                if let Some(d) = default_value
                    && d.len() > MAX_PARAM_STRING_LENGTH
                {
                    return Err(invalid(format!(
                        "default_value exceeds max length of {MAX_PARAM_STRING_LENGTH} bytes (got {})",
                        d.len()
                    )));
                }
                if !enum_values.is_empty()
                    && let Some(d) = default_value
                    && !enum_values.iter().any(|o| &o.value == d)
                {
                    return Err(invalid(format!("default_value {d:?} not in enum_values")));
                }
            }
            ParamKind::Double {
                min,
                max,
                step,
                enum_values,
                default_value,
            } => {
                check_finite(*default_value, "default_value").map_err(&invalid)?;
                check_finite(*min, "min").map_err(&invalid)?;
                check_finite(*max, "max").map_err(&invalid)?;
                check_finite(*step, "step").map_err(&invalid)?;
                for o in enum_values {
                    check_finite(Some(o.value), "enum_values[].value").map_err(&invalid)?;
                }
                check_double_range(*min, *max, *step, *default_value).map_err(&invalid)?;
                check_double_options(enum_values, *default_value).map_err(&invalid)?;
            }
            ParamKind::Integer {
                min,
                max,
                step,
                enum_values,
                default_value,
            } => {
                check_int_range(*min, *max, *step, *default_value).map_err(&invalid)?;
                check_int_options(enum_values, *default_value).map_err(&invalid)?;
            }
            ParamKind::Boolean { .. } => {}
            ParamKind::Timezone { default_value } => {
                if let Some(d) = default_value
                    && d.len() > MAX_PARAM_STRING_LENGTH
                {
                    return Err(invalid(format!(
                        "default_value exceeds max length of {MAX_PARAM_STRING_LENGTH} bytes (got {})",
                        d.len()
                    )));
                }
            }
        }
        Ok(())
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
