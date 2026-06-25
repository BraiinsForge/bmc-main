// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget manifest types — the on-disk schema for `manifest.json`.
//!
//! The structure here is the source of truth for what a widget's `manifest.json` file may contain.
//! Field-level semantics are documented on each type and variant with `///` doc comments;
//! Those comments are propagated through `schemars` into the generated JSON Schema
//! as `description` properties, which is how the schema's editor tooling surfaces them.
//!
//! Validation has two layers. Structural constraints expressible in JSON Schema
//! (length caps, regex patterns, exclusive numeric bounds, required-fields-by-variant)
//! are encoded on the types via `schemars` attributes and enforced by any JSON Schema validator.
//!
//! Cross-field invariants that JSON Schema cannot describe live in [`Manifest::from_str`] and friends.
//! Examples:
//!
//!  - `default_value` lies within `[min, max]`
//!  - `default_value` is in `enum_values`
//!  - `+0.0` / `-0.0` collide in double enum dedup
//!
//! See the per-type docs for the complete split.

use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;

use bmc_ipc::SizeType;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

const MAX_NAME_LENGTH: usize = 50;
const MAX_SUBNAME_LENGTH: usize = 30;
const MAX_DESCRIPTION_LENGTH: usize = 200;

/// Maximum byte length of a [`ParamKey`].
///
/// Keys are identifier-shaped (`^[A-Za-z][A-Za-z0-9_\-]*$`); 64 bytes is comfortably above typical
/// values while staying well under the wire-format `u16` length field, so the encoder's
/// `u16::try_from` is statically infallible.
pub const MAX_PARAM_KEY_LENGTH: usize = 64;

/// Maximum byte length of any string-shaped value: [`ParamValue::String`],
/// [`StringOption::value`], and the `default_value` of [`ParamKind::String`] / [`ParamKind::Timezone`].
///
/// 1024 bytes covers IANA timezone IDs (≤ 32), enum codes (typically ≤ 64), and ad-hoc free-text
/// values without runaway. Well under the wire-format `u32` length field, so the encoder's
/// `u32::try_from` is statically infallible.
pub const MAX_PARAM_STRING_LENGTH: usize = 1024;

/// Errors produced by [`Manifest::from_str`] and the structural / semantic validators it dispatches to.
/// Each variant names the rule that was violated, so a downstream caller can surface them without re-parsing the message.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// JSON could not be parsed at all, or violated a `#[serde(...)]`
    /// schema constraint (e.g. wrong literal type on `default_value`).
    #[error("failed to parse manifest JSON: {0}")]
    ParseError(#[from] serde_json::Error),

    /// The `uid` field was not a syntactically-valid UUID.
    #[error("invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    /// The `uid` field parsed as a UUID but was not version 4.
    /// Widgets are required to use random UUIDs so that any new widget gets a fresh identifier without coordination.
    #[error("UUID must be version 4, got version {0}")]
    InvalidUuidVersion(usize),

    /// The `version` field could not be parsed as a semver string.
    #[error("invalid version '{version}': {source}")]
    InvalidVersion {
        version: String,
        source: semver::Error,
    },

    /// The `name` field exceeded the declared length cap.
    #[error("name exceeds maximum length of {max} characters")]
    NameTooLong { max: usize },

    /// The `subname` field exceeded the declared length cap.
    #[error("subname exceeds maximum length of {max} characters")]
    SubnameTooLong { max: usize },

    /// The `description` field exceeded the declared length cap.
    #[error("description exceeds maximum length of {max} characters")]
    DescriptionTooLong { max: usize },

    /// `supported_viewports` was empty after compatibility normalization.
    #[error("supported_viewports must not be empty")]
    EmptyViewports,

    /// A viewport constraint violated a numeric rule (zero provided bound, or min > max).
    #[error("invalid viewport constraint: {0}")]
    InvalidViewport(String),

    /// Two viewport constraints had identical display type and all six bounds.
    #[error("duplicate viewport constraint")]
    DuplicateViewport,

    /// A manifest provided both legacy `sizes` and new `supported_viewports`.
    #[error("manifest must not provide both `sizes` and `supported_viewports`")]
    MixedSizesAndViewports,

    /// A `settings` entry was not a recognised [`SettingKey`] variant.
    #[error("invalid setting key: {0}")]
    InvalidSettingKey(String),

    /// A specific parameter failed its semantic validator.
    /// The [`ParamDefinition::validate`] check fires for cross-field invariants
    /// (default ∈ enum_values, default ∈ [min, max], etc.) that JSON Schema
    /// cannot express on its own.
    #[error("parameter {name:?}: {reason}")]
    InvalidParam { name: String, reason: String },

    /// Two `params` entries with identical keys.
    /// The deserializer rejects these at parse time; this variant exists
    /// for callers that build manifests programmatically.
    #[error("duplicate parameter key: {0:?}")]
    DuplicateParamKey(String),
}

/// A param key inside a manifest's `params` map.
/// Matches the regex `^[A-Za-z][A-Za-z0-9_-]*$` — starts with an ASCII letter,
/// then any mix of ASCII alphanumerics, hyphen, and underscore. Capped at [`MAX_PARAM_KEY_LENGTH`] bytes.
///
/// The host guarantees keys are stable identifiers safe to use
/// as Rust field names after a snake_case-ish normalisation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[schemars(transparent)]
pub struct ParamKey(
    #[schemars(regex(pattern = r"^[A-Za-z][A-Za-z0-9_\-]*$"), length(max = MAX_PARAM_KEY_LENGTH))]
    String,
);

impl ParamKey {
    /// Construct a `ParamKey` from an owned string, applying the same character-class and length rules as the `Deserialize` impl.
    /// Returns the rejected input back to the caller on failure.
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
}

/// Typed scalar value for a stored widget param.
/// Mirrors the manifest's [`ParamKind`] value space — null, boolean, i32, finite f64, string — nothing wider.
///
/// Used both as the in-memory representation on the compositor side and
/// as the wire shape sent to widgets through the wayland `params` event.
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
    /// JSON projection for the wayland boundary — widget processes receive a JSON object
    /// whose values are bare scalars, not the internally-tagged form.
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

    /// Build the default value for a manifest param.
    /// Required params without a `default_value` are caught at manifest load time
    /// (`ParamKind::has_default_value`); optional params without a default deserialize to `Null`.
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
/// Each variant names a JSON shape the scalar schema does not support;
/// the host drops these at debug log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ParamValueConversionError {
    /// `null`, `bool`, finite number, or string was expected; got an array.
    #[error("expected scalar, got JSON array")]
    Array,
    /// `null`, `bool`, finite number, or string was expected; got an object.
    #[error("expected scalar, got JSON object")]
    Object,
    /// A number that was neither integer- nor f64-representable.
    /// In practice this is unreachable through `serde_json`'s default parser,
    /// but the variant exists so the match is exhaustive.
    #[error("number is not representable as i32 or f64")]
    UnrepresentableNumber,
    /// NaN or ±infinity. `serde_json` does not emit these by default,
    /// but a custom-built `serde_json::Value` can carry them.
    #[error("number is not finite")]
    NonFiniteNumber,
    /// String value exceeded [`MAX_PARAM_STRING_LENGTH`].
    #[error("string value exceeds max length of {max} bytes (got {len})")]
    StringTooLong { len: usize, max: usize },
}

/// Inverse of [`ParamValue::to_json_value`].
///
/// Maps the scalar shapes (`null`, `bool`, integer-shaped number within i32 range,
/// finite non-integer or out-of-i32 finite number, string) into the typed variant.
///
/// Returns `Err` for everything else (objects, arrays, non-finite numbers).
///
/// Used at the wayland JSON edge inside the wasm host runtime:
///  the host receives JSON the compositor built from a parsed-and-validated manifest,
///  and re-types it back into [`ParamValue`] for in-memory storage.
///
/// The two functions together form the round-trip boundary.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RawManifest {
    uid: Uuid,
    version: String,
    name: String,
    #[serde(default)]
    subname: Option<String>,
    description: String,
    #[serde(default)]
    author: Option<Author>,
    binary: PathBuf,
    #[serde(default)]
    icon: Option<PathBuf>,
    #[serde(default)]
    category: WidgetCategory,
    #[serde(default)]
    settings: Vec<SettingKey>,
    #[serde(default)]
    sizes: Option<Vec<SizeType>>,
    #[serde(default)]
    supported_viewports: Option<Vec<WidgetViewportConstraint>>,
    #[serde(default, deserialize_with = "deserialize_unique_params")]
    params: IndexMap<ParamKey, ParamDefinition>,
}

fn deserialize_unique_params<'de, D>(
    deserializer: D,
) -> Result<IndexMap<ParamKey, ParamDefinition>, D::Error>
where
    D: Deserializer<'de>,
{
    struct UniqueMapVisitor;

    impl<'de> Visitor<'de> for UniqueMapVisitor {
        type Value = IndexMap<ParamKey, ParamDefinition>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map with unique param keys")
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut map = IndexMap::with_capacity(access.size_hint().unwrap_or(0));
            while let Some((key, value)) = access.next_entry::<ParamKey, ParamDefinition>()? {
                if map.contains_key(&key) {
                    return Err(M::Error::custom(format!(
                        "duplicate param key {:?}",
                        key.as_str()
                    )));
                }
                map.insert(key, value);
            }
            Ok(map)
        }
    }

    deserializer.deserialize_map(UniqueMapVisitor)
}

/// The parsed and validated form of a widget's `manifest.json`.
/// The canonical loader is [`Manifest::from_str`] (and [`Manifest::from_reader`]);
/// They enforce both the JSON Schema-expressible constraints (via the underlying `RawManifest` deserialize)
/// and the cross-field invariants the schema cannot describe.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Manifest {
    /// Stable unique identifier — must be UUID version 4. Used to distinguish widgets at runtime;
    /// never reused once a widget is published.
    #[schemars(with = "String")]
    pub uid: Uuid,
    /// Widget version, in semver.
    #[schemars(with = "String")]
    pub version: semver::Version,
    /// Human-readable widget name, shown in the operator UI.
    /// Capped at 50 characters.
    #[schemars(length(max = 50))]
    pub name: String,
    /// Optional short secondary label, shown grayed beside `name`.
    /// Capped at 30 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 30))]
    pub subname: Option<String>,
    /// One-line widget description, shown in the operator UI.
    /// Capped at 200 characters.
    #[schemars(length(max = 200))]
    pub description: String,
    /// Optional author block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// Path to the widget binary, relative to the widget's directory.
    #[schemars(with = "String")]
    pub binary: PathBuf,
    /// Optional icon path, relative to the widget dir or absolute. Served by BMC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub icon: Option<PathBuf>,
    /// Catalog grouping for the Add-widget picker. Defaults to [`WidgetCategory::Misc`].
    #[serde(default)]
    pub category: WidgetCategory,
    /// System-provided settings the widget wants injected (locale, timezone, night mode, etc.).
    /// Order is preserved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingKey>,
    /// Viewport families this widget supports. Must be non-empty after
    /// compatibility normalization of any legacy `sizes`.
    #[schemars(length(min = 1))]
    pub supported_viewports: Vec<WidgetViewportConstraint>,
    /// Per-instance parameter declarations, keyed by [`ParamKey`].
    /// Iteration order matches manifest order.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub params: IndexMap<ParamKey, ParamDefinition>,
}

impl FromStr for Manifest {
    type Err = ManifestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw: RawManifest = serde_json::from_str(s)?;
        Self::from_raw(raw)
    }
}

impl Manifest {
    pub fn from_reader<R: Read>(reader: R) -> Result<Self, ManifestError> {
        let raw: RawManifest = serde_json::from_reader(reader)?;
        Self::from_raw(raw)
    }

    fn from_raw(raw: RawManifest) -> Result<Self, ManifestError> {
        let version =
            semver::Version::parse(&raw.version).map_err(|e| ManifestError::InvalidVersion {
                version: raw.version.clone(),
                source: e,
            })?;

        let supported_viewports = match (raw.sizes, raw.supported_viewports) {
            (Some(_), Some(_)) => return Err(ManifestError::MixedSizesAndViewports),
            (Some(sizes), None) => sizes
                .into_iter()
                .map(WidgetViewportConstraint::from)
                .collect(),
            (None, Some(viewports)) => viewports,
            (None, None) => Vec::new(),
        };

        let manifest = Self {
            uid: raw.uid,
            version,
            name: raw.name,
            subname: raw.subname,
            description: raw.description,
            author: raw.author,
            binary: raw.binary,
            icon: raw.icon,
            category: raw.category,
            settings: raw.settings,
            supported_viewports,
            params: raw.params,
        };

        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.uid.get_version_num() != 4 {
            return Err(ManifestError::InvalidUuidVersion(
                self.uid.get_version_num(),
            ));
        }

        if self.name.len() > MAX_NAME_LENGTH {
            return Err(ManifestError::NameTooLong {
                max: MAX_NAME_LENGTH,
            });
        }

        if self
            .subname
            .as_ref()
            .is_some_and(|s| s.len() > MAX_SUBNAME_LENGTH)
        {
            return Err(ManifestError::SubnameTooLong {
                max: MAX_SUBNAME_LENGTH,
            });
        }

        if self.description.len() > MAX_DESCRIPTION_LENGTH {
            return Err(ManifestError::DescriptionTooLong {
                max: MAX_DESCRIPTION_LENGTH,
            });
        }

        if self.supported_viewports.is_empty() {
            return Err(ManifestError::EmptyViewports);
        }
        for vp in &self.supported_viewports {
            validate_viewport_constraint(vp)?;
        }
        for (i, a) in self.supported_viewports.iter().enumerate() {
            for b in &self.supported_viewports[i + 1..] {
                if a == b {
                    return Err(ManifestError::DuplicateViewport);
                }
            }
        }

        for (key, param) in &self.params {
            param.validate(key.as_str())?;
        }

        Ok(())
    }
}

/// Optional author block on a manifest, surfaced in the operator UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Author {
    /// Author name, as shown in the operator UI.
    pub name: String,
    /// Optional link to a project page or organisation site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Catalog grouping a widget belongs to, used by the Add-widget picker to
/// section the list. A widget declares exactly one; the field defaults to
/// [`WidgetCategory::Misc`] when absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WidgetCategory {
    /// Miner, fleet, and pool / network mining stats.
    Mining,
    /// Time and date.
    Clock,
    /// Weather and forecast.
    Weather,
    /// Calendar and dated events.
    Calendar,
    /// Space and astronomy.
    Space,
    /// Diagnostic, demo, and system tools.
    Utility,
    /// Audio, video, and image media.
    Media,
    /// Uncategorized — the default fallback.
    #[default]
    Misc,
}

/// System-provided settings a widget can request, listed in the manifest's `settings` array.
/// Each variant names a category the host injects automatically — the widget does not declare these as params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SettingKey {
    /// Locale and language settings — the widget receives the configured locale on start.
    Localization,
    /// Time zone settings — the widget receives the configured zone on start and on change.
    Timezone,
    /// Night-mode dimming preference.
    NightMode,
}

/// Visible viewport shape a widget viewport constraint targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ViewportShape {
    /// Rectangular viewport.
    Rectangular,
    /// Round viewport.
    Round,
}

/// A viewport family a widget author declares support for. Ranges are
/// inclusive on both ends; `None` means unbounded in that direction, while
/// `min_* == max_*` pins one exact value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct WidgetViewportConstraint {
    /// Viewport shape this constraint targets.
    #[serde(rename = "type")]
    pub viewport_shape: ViewportShape,
    /// Inclusive minimum width in pixels, or unbounded.
    pub min_width: Option<u32>,
    /// Inclusive maximum width in pixels, or unbounded.
    pub max_width: Option<u32>,
    /// Inclusive minimum height in pixels, or unbounded.
    pub min_height: Option<u32>,
    /// Inclusive maximum height in pixels, or unbounded.
    pub max_height: Option<u32>,
    /// Inclusive minimum dpi, or unbounded.
    pub min_dpi: Option<u32>,
    /// Inclusive maximum dpi, or unbounded.
    pub max_dpi: Option<u32>,
}

impl From<SizeType> for WidgetViewportConstraint {
    fn from(size: SizeType) -> Self {
        let (w, h) = match size {
            SizeType::Small => (317, 238),
            SizeType::Medium => (638, 238),
            SizeType::Large => (638, 480),
            SizeType::Full => (1280, 480),
        };
        Self {
            viewport_shape: ViewportShape::Rectangular,
            min_width: Some(w),
            max_width: Some(w),
            min_height: Some(h),
            max_height: Some(h),
            min_dpi: None,
            max_dpi: None,
        }
    }
}

/// Per-instance parameter declaration inside a manifest's `params` map.
/// The `kind` field carries the value-type-specific options (`enum_values`, `min`, `max`, etc.)
/// via a serde-flattened tagged enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParamDefinition {
    /// Human-readable parameter name, shown in the operator UI.
    pub name: String,
    /// Optional one-line parameter description, shown in the operator UI as help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the operator can leave this param unset.
    /// Optional params may have a default; required params *must* (and the host always delivers a value).
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
    pub fn validate(&self, name: &str) -> Result<(), ManifestError> {
        let invalid = |reason: String| ManifestError::InvalidParam {
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

    fn validate(&self, name: &str) -> Result<(), ManifestError> {
        let invalid = |reason: String| ManifestError::InvalidParam {
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

fn validate_viewport_constraint(vp: &WidgetViewportConstraint) -> Result<(), ManifestError> {
    let invalid = |reason: &str| ManifestError::InvalidViewport(reason.to_owned());
    if vp.min_width == Some(0) || vp.max_width == Some(0) {
        return Err(invalid("provided width min/max must be nonzero"));
    }
    if vp.min_height == Some(0) || vp.max_height == Some(0) {
        return Err(invalid("provided height min/max must be nonzero"));
    }
    if vp.min_dpi == Some(0) || vp.max_dpi == Some(0) {
        return Err(invalid("provided dpi min/max must be nonzero"));
    }
    if matches!((vp.min_width, vp.max_width), (Some(min), Some(max)) if min > max) {
        return Err(invalid("min_width > max_width"));
    }
    if matches!((vp.min_height, vp.max_height), (Some(min), Some(max)) if min > max) {
        return Err(invalid("min_height > max_height"));
    }
    if matches!((vp.min_dpi, vp.max_dpi), (Some(min), Some(max)) if min > max) {
        return Err(invalid("min_dpi > max_dpi"));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_manifest_json() -> &'static str {
        r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#
    }

    fn full_manifest_json() -> &'static str {
        r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.2.3",
            "name": "Clock",
            "description": "Analog and digital clock display",
            "author": {
                "name": "Braiins",
                "url": "https://braiins.com"
            },
            "binary": "bin/clock",
            "settings": ["localization", "timezone", "nightMode"],
            "sizes": ["small", "medium", "large", "full"],
            "params": {
                "style": {
                    "name": "Clock Style",
                    "type": "string",
                    "description": "Visual style of the clock",
                    "default_value": "digital",
                    "enum_values": [
                        {"value": "digital", "label": "Digital"},
                        {"value": "analog", "label": "Analog"}
                    ]
                },
                "showSeconds": {
                    "name": "Show Seconds",
                    "type": "boolean",
                    "description": "Display seconds on the clock",
                    "default_value": false
                }
            }
        }"#
    }

    #[test]
    fn parse_minimal_manifest() {
        let manifest = Manifest::from_str(minimal_manifest_json()).expect("BUG: should parse");
        assert_eq!(
            manifest.uid,
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: valid uuid")
        );
        assert_eq!(manifest.version, semver::Version::new(1, 0, 0));
        assert_eq!(manifest.name, "Test Widget");
        assert_eq!(manifest.description, "A test widget");
        assert_eq!(manifest.binary, PathBuf::from("bin/test"));
        assert_eq!(manifest.supported_viewports.len(), 1);
        assert_eq!(
            manifest.supported_viewports[0].viewport_shape,
            ViewportShape::Rectangular
        );
        assert!(manifest.author.is_none());
        assert!(manifest.icon.is_none());
        assert!(manifest.subname.is_none());
        assert!(manifest.settings.is_empty());
        assert!(manifest.params.is_empty());
    }

    #[test]
    fn parse_subname() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "subname": "Analog",
            "description": "A test widget",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        assert_eq!(manifest.subname.as_deref(), Some("Analog"));
    }

    #[test]
    fn reject_subname_too_long() {
        let json = format!(
            r#"{{
                "uid": "550e8400-e29b-41d4-a716-446655440000",
                "version": "1.0.0",
                "name": "Test Widget",
                "subname": "{}",
                "description": "A test widget",
                "binary": "bin/test",
                "supported_viewports": [
                    {{"type":"rectangular","min_width":317,"max_width":317,
                     "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}}
                ]
            }}"#,
            "x".repeat(MAX_SUBNAME_LENGTH + 1)
        );
        let result = Manifest::from_str(&json);
        assert!(matches!(result, Err(ManifestError::SubnameTooLong { .. })));
    }

    #[test]
    fn parse_relative_icon() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "icon": "assets/icon.svg",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        assert_eq!(manifest.icon, Some(PathBuf::from("assets/icon.svg")));
    }

    #[test]
    fn parse_absolute_icon() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "icon": "/usr/share/bmc/icons/test.png",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        assert_eq!(
            manifest.icon,
            Some(PathBuf::from("/usr/share/bmc/icons/test.png"))
        );
    }

    #[test]
    fn parse_category() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "category": "mining",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        assert_eq!(manifest.category, WidgetCategory::Mining);
    }

    #[test]
    fn category_defaults_to_misc_when_absent() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        assert_eq!(manifest.category, WidgetCategory::Misc);
    }

    #[test]
    fn reject_unknown_category() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "category": "bogus",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        let result = Manifest::from_str(json);
        assert!(matches!(result, Err(ManifestError::ParseError(_))));
    }

    #[test]
    fn parse_full_manifest() {
        let manifest = Manifest::from_str(full_manifest_json()).expect("BUG: should parse");
        assert_eq!(manifest.version, semver::Version::new(1, 2, 3));
        assert_eq!(manifest.name, "Clock");

        let author = manifest.author.expect("BUG: should have author");
        assert_eq!(author.name, "Braiins");
        assert_eq!(author.url, Some("https://braiins.com".to_owned()));

        assert_eq!(
            manifest.settings,
            vec![
                SettingKey::Localization,
                SettingKey::Timezone,
                SettingKey::NightMode
            ]
        );
        #[expect(
            clippy::type_complexity,
            reason = "test-only tuple for compact assertion"
        )]
        let viewport_bounds: Vec<(Option<u32>, Option<u32>, Option<u32>, Option<u32>)> = manifest
            .supported_viewports
            .iter()
            .map(|c| (c.min_width, c.max_width, c.min_height, c.max_height))
            .collect();
        assert_eq!(
            viewport_bounds,
            vec![
                (Some(317), Some(317), Some(238), Some(238)),
                (Some(638), Some(638), Some(238), Some(238)),
                (Some(638), Some(638), Some(480), Some(480)),
                (Some(1280), Some(1280), Some(480), Some(480)),
            ]
        );
        assert_eq!(manifest.params.len(), 2);

        let style_param = manifest
            .params
            .get("style")
            .expect("BUG: should have style param");
        assert_eq!(style_param.name, "Clock Style");
        match &style_param.kind {
            ParamKind::String { default_value, .. } => {
                assert_eq!(default_value.as_deref(), Some("digital"));
            }
            ParamKind::Double { .. }
            | ParamKind::Integer { .. }
            | ParamKind::Boolean { .. }
            | ParamKind::Timezone { .. } => panic!("BUG: expected String variant"),
        }
    }

    #[test]
    fn reject_non_v4_uuid() {
        let json = r#"{
            "uid": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small"]
        }"#;
        let result = Manifest::from_str(json);
        assert!(matches!(result, Err(ManifestError::InvalidUuidVersion(1))));
    }

    #[test]
    fn reject_invalid_semver() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "not-a-version",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small"]
        }"#;
        let result = Manifest::from_str(json);
        assert!(matches!(result, Err(ManifestError::InvalidVersion { .. })));
    }

    #[test]
    fn reject_name_too_long() {
        let long_name = "a".repeat(51);
        let json = format!(
            r#"{{
                "uid": "550e8400-e29b-41d4-a716-446655440000",
                "version": "1.0.0",
                "name": "{long_name}",
                "description": "Test",
                "binary": "bin/test",
                "sizes": ["small"]
            }}"#
        );
        let result = Manifest::from_str(&json);
        assert!(matches!(result, Err(ManifestError::NameTooLong { .. })));
    }

    #[test]
    fn reject_param_type_mismatch() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small"],
            "params": {
                "flag": {
                    "name": "Flag",
                    "type": "boolean",
                    "default_value": "not a boolean"
                }
            }
        }"#;
        let result = Manifest::from_str(json);
        assert!(matches!(result, Err(ManifestError::ParseError(_))));
    }

    #[test]
    fn accept_number_param_with_constraints() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small"],
            "params": {
                "brightness": {
                    "name": "Brightness",
                    "type": "integer",
                    "min": 0,
                    "max": 100,
                    "default_value": 50
                }
            }
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        let param = manifest
            .params
            .get("brightness")
            .expect("BUG: should have brightness");
        match &param.kind {
            ParamKind::Integer {
                min,
                max,
                default_value,
                ..
            } => {
                assert_eq!(*min, Some(0));
                assert_eq!(*max, Some(100));
                assert_eq!(*default_value, Some(50));
            }
            ParamKind::String { .. }
            | ParamKind::Double { .. }
            | ParamKind::Boolean { .. }
            | ParamKind::Timezone { .. } => panic!("BUG: expected Integer variant"),
        }
    }

    #[test]
    fn param_key_accepts_valid_keys() {
        for key in ["foo", "Foo", "foo-bar", "foo_bar", "a1", "a1-b2_c3"] {
            let json = format!("\"{key}\"");
            let parsed: ParamKey = serde_json::from_str(&json).expect("BUG: valid key must parse");
            assert_eq!(parsed.as_str(), key);
        }
    }

    #[test]
    fn param_key_rejects_invalid_keys() {
        for bad in ["", "1abc", "_foo", "-foo", "123", "foo bar", "foo!", "föö"] {
            let json = format!("\"{bad}\"");
            let res: Result<ParamKey, _> = serde_json::from_str(&json);
            assert!(res.is_err(), "expected reject for {bad:?}");
        }
    }

    #[test]
    fn param_definition_round_trips_each_variant() {
        let cases = [
            r#"{"name":"S","type":"string","default_value":"x"}"#,
            r#"{"name":"D","type":"double","default_value":1.5,"min":0.0,"max":10.0}"#,
            r#"{"name":"I","type":"integer","default_value":2,"min":1,"max":5}"#,
            r#"{"name":"B","type":"boolean","default_value":true}"#,
            r#"{"name":"T","type":"timezone","default_value":"Europe/Prague"}"#,
        ];
        for case in cases {
            let p: ParamDefinition =
                serde_json::from_str(case).unwrap_or_else(|e| panic!("BUG: parse {case:?}: {e}"));
            let back = serde_json::to_string(&p).expect("BUG: serialize");
            let p2: ParamDefinition = serde_json::from_str(&back).expect("BUG: re-parse");
            assert_eq!(p, p2);
        }
    }

    #[test]
    fn param_definition_rejects_unknown_type_tag() {
        let json = r#"{"name":"X","type":"color","default_value":"red"}"#;
        assert!(serde_json::from_str::<ParamDefinition>(json).is_err());
    }

    #[test]
    fn integer_default_with_fractional_value_fails_at_parse() {
        let json = r#"{"name":"X","type":"integer","default_value":3.14}"#;
        assert!(serde_json::from_str::<ParamDefinition>(json).is_err());
    }

    #[test]
    fn boolean_default_with_int_fails_at_parse() {
        let json = r#"{"name":"X","type":"boolean","default_value":0}"#;
        assert!(serde_json::from_str::<ParamDefinition>(json).is_err());
    }

    #[test]
    fn validate_required_without_default_fails() {
        let p: ParamDefinition =
            serde_json::from_str(r#"{"name":"X","type":"string"}"#).expect("BUG: parse");
        assert!(matches!(
            p.validate("x"),
            Err(ManifestError::InvalidParam { .. })
        ));
    }

    #[test]
    fn validate_optional_without_default_passes() {
        let p: ParamDefinition =
            serde_json::from_str(r#"{"name":"X","type":"string","optional":true}"#)
                .expect("BUG: parse");
        p.validate("x")
            .expect("BUG: optional without default must validate");
    }

    #[test]
    fn validate_double_min_greater_than_max_fails() {
        let p: ParamDefinition = serde_json::from_str(
            r#"{"name":"X","type":"double","default_value":0.0,"min":10.0,"max":5.0}"#,
        )
        .expect("BUG: parse");
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_integer_default_outside_bounds_fails() {
        let p: ParamDefinition = serde_json::from_str(
            r#"{"name":"X","type":"integer","default_value":15,"min":0,"max":10}"#,
        )
        .expect("BUG: parse");
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_double_step_zero_fails() {
        let p: ParamDefinition =
            serde_json::from_str(r#"{"name":"X","type":"double","default_value":1.0,"step":0.0}"#)
                .expect("BUG: parse");
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_double_non_finite_default_fails() {
        let mut p: ParamDefinition =
            serde_json::from_str(r#"{"name":"X","type":"double","default_value":1.0}"#)
                .expect("BUG: parse");
        if let ParamKind::Double { default_value, .. } = &mut p.kind {
            *default_value = Some(f64::NAN);
        }
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_double_non_finite_bound_fails() {
        let mut p: ParamDefinition =
            serde_json::from_str(r#"{"name":"X","type":"double","default_value":1.0}"#)
                .expect("BUG: parse");
        if let ParamKind::Double { max, .. } = &mut p.kind {
            *max = Some(f64::INFINITY);
        }
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_enum_default_not_in_options_fails() {
        let p: ParamDefinition = serde_json::from_str(
            r#"{"name":"X","type":"string","default_value":"blue","enum_values":[
                  {"value":"red","label":"Red"},
                  {"value":"green","label":"Green"}
               ]}"#,
        )
        .expect("BUG: parse");
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_enum_duplicate_values_fails() {
        let p: ParamDefinition = serde_json::from_str(
            r#"{"name":"X","type":"integer","default_value":1,"enum_values":[
                  {"value":1,"label":"One"},
                  {"value":1,"label":"Uno"}
               ]}"#,
        )
        .expect("BUG: parse");
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_enum_empty_label_fails() {
        let p: ParamDefinition = serde_json::from_str(
            r#"{"name":"X","type":"string","default_value":"a","enum_values":[
                  {"value":"a","label":"   "}
               ]}"#,
        )
        .expect("BUG: parse");
        assert!(p.validate("x").is_err());
    }

    #[test]
    fn validate_string_enum_empty_value_rejects() {
        let p: Result<ParamDefinition, _> = serde_json::from_str(
            r#"{"name":"X","type":"string","default_value":"a","enum_values":[
                  {"value":"","label":"None"},
                  {"value":"a","label":"A"}
               ]}"#,
        );
        let err = match p {
            Err(e) => e.to_string(),
            Ok(def) => def
                .validate("x")
                .expect_err("BUG: empty value must be rejected")
                .to_string(),
        };
        assert!(err.contains("non-empty"), "{err}");
    }

    #[test]
    fn validate_string_enum_duplicate_values_rejects() {
        let p: ParamDefinition = serde_json::from_str(
            r#"{"name":"X","type":"string","default_value":"a","enum_values":[
                  {"value":"a","label":"A"},
                  {"value":"a","label":"B"}
               ]}"#,
        )
        .expect("BUG: parse");
        assert!(
            p.validate("x").is_err(),
            "expected error for duplicate enum values",
        );
    }

    #[test]
    fn manifest_rejects_invalid_param_key() {
        let manifest_json = r#"{
            "uid":"550e8400-e29b-41d4-a716-446655440000",
            "version":"1.0.0",
            "name":"Test",
            "description":"Test",
            "binary":"bin/test",
            "sizes":["small"],
            "params":{"123":{"name":"Bad","type":"string","default_value":"x"}}
        }"#;
        let res = Manifest::from_str(manifest_json);
        assert!(res.is_err(), "all-digit param key must reject");
    }

    #[test]
    fn param_value_round_trips_each_variant() {
        let cases = [
            ParamValue::Null,
            ParamValue::Boolean(true),
            ParamValue::Integer(-7),
            ParamValue::Double(3.5),
            ParamValue::String("x".into()),
        ];
        for v in cases {
            let s = serde_json::to_string(&v).expect("BUG: serialize");
            let back: ParamValue = serde_json::from_str(&s).expect("BUG: deserialize");
            assert_eq!(v, back);
        }
    }

    #[test]
    fn param_value_distinguishes_integer_and_double_by_json_literal() {
        let cases: &[(&str, ParamValue)] = &[
            ("42", ParamValue::Integer(42)),
            ("-7", ParamValue::Integer(-7)),
            ("0", ParamValue::Integer(0)),
            ("42.0", ParamValue::Double(42.0)),
            ("-7.0", ParamValue::Double(-7.0)),
            ("0.0", ParamValue::Double(0.0)),
            ("42.5", ParamValue::Double(42.5)),
            ("2147483648", ParamValue::Double(2_147_483_648.0)),
        ];
        for (json, expected) in cases {
            let v: ParamValue =
                serde_json::from_str(json).expect("BUG: deserialize numeric literal");
            assert_eq!(
                &v, expected,
                "json {json} should deserialize to {expected:?}"
            );
        }
    }

    #[test]
    fn param_value_serializes_as_bare_scalar() {
        let cases: &[(ParamValue, &str)] = &[
            (ParamValue::Null, "null"),
            (ParamValue::Boolean(true), "true"),
            (ParamValue::Integer(42), "42"),
            (ParamValue::Double(2.5), "2.5"),
            (ParamValue::String("hi".into()), r#""hi""#),
        ];
        for (v, expected) in cases {
            let s = serde_json::to_string(v).expect("BUG: serialize ParamValue");
            assert_eq!(&s, expected, "{v:?} should serialize to {expected}");
        }
    }

    #[test]
    fn param_value_to_json_value_maps_each_variant() {
        assert!(ParamValue::Null.to_json_value().is_null());
        assert_eq!(
            ParamValue::Boolean(true).to_json_value(),
            serde_json::json!(true)
        );
        assert_eq!(
            ParamValue::Integer(42).to_json_value(),
            serde_json::json!(42)
        );
        assert_eq!(
            ParamValue::Double(2.5).to_json_value(),
            serde_json::json!(2.5)
        );
        assert_eq!(
            ParamValue::String("hi".into()).to_json_value(),
            serde_json::json!("hi")
        );
    }

    #[test]
    fn param_value_try_from_json_maps_each_scalar() {
        let cases: &[(serde_json::Value, ParamValue)] = &[
            (serde_json::Value::Null, ParamValue::Null),
            (serde_json::json!(true), ParamValue::Boolean(true)),
            (serde_json::json!(42), ParamValue::Integer(42)),
            (serde_json::json!(-7), ParamValue::Integer(-7)),
            (serde_json::json!(2.5), ParamValue::Double(2.5)),
            (serde_json::json!("hi"), ParamValue::String("hi".into())),
        ];
        for (json, expected) in cases {
            let got = ParamValue::try_from(json).expect("BUG: should convert");
            assert_eq!(&got, expected, "{json:?} should convert to {expected:?}");
        }
    }

    #[test]
    fn param_value_try_from_json_widens_out_of_i32_to_double() {
        let v = serde_json::json!(2_147_483_648_i64);
        let got = ParamValue::try_from(&v).expect("BUG: should convert");
        assert_eq!(got, ParamValue::Double(2_147_483_648.0));
    }

    #[test]
    fn param_value_try_from_json_rejects_arrays_and_objects() {
        let arr = serde_json::json!([1, 2]);
        let obj = serde_json::json!({"a": 1});
        assert_eq!(
            ParamValue::try_from(&arr),
            Err(ParamValueConversionError::Array)
        );
        assert_eq!(
            ParamValue::try_from(&obj),
            Err(ParamValueConversionError::Object)
        );
    }

    #[test]
    fn param_value_from_param_kind_default_picks_default_or_null() {
        let with_default = ParamKind::Integer {
            min: None,
            max: None,
            step: None,
            enum_values: vec![],
            default_value: Some(7),
        };
        assert_eq!(
            ParamValue::from_param_kind_default(&with_default),
            ParamValue::Integer(7)
        );

        let without_default = ParamKind::String {
            format: None,
            enum_values: vec![],
            default_value: None,
        };
        assert_eq!(
            ParamValue::from_param_kind_default(&without_default),
            ParamValue::Null
        );
    }

    #[test]
    fn validate_double_enum_treats_plus_zero_and_minus_zero_as_duplicate() {
        let p: ParamDefinition = serde_json::from_str(
            r#"{"name":"X","type":"double","default_value":0.0,"enum_values":[
                  {"value":0.0,"label":"plus"},
                  {"value":-0.0,"label":"minus"}
               ]}"#,
        )
        .expect("BUG: parse");
        assert!(
            p.validate("x").is_err(),
            "+0.0 and -0.0 must be duplicates after canonicalisation",
        );
    }

    #[test]
    fn manifest_rejects_duplicate_param_keys() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small"],
            "params": {
                "foo": {"name": "Foo", "type": "string", "default_value": "a"},
                "foo": {"name": "Foo2", "type": "string", "default_value": "b"}
            }
        }"#;
        let result = Manifest::from_str(json);
        let err = result.expect_err("duplicate param key must fail");
        assert!(
            err.to_string().contains("duplicate"),
            "error must mention duplicate: {err}"
        );
    }

    #[test]
    fn manifest_preserves_param_order() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small"],
            "params": {
                "zebra": {"name": "Z", "type": "string", "default_value": "z"},
                "alpha": {"name": "A", "type": "string", "default_value": "a"},
                "mango": {"name": "M", "type": "string", "default_value": "m"}
            }
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: parse");
        let keys: Vec<&str> = manifest.params.keys().map(ParamKey::as_str).collect();
        assert_eq!(keys, vec!["zebra", "alpha", "mango"]);
    }

    fn minimal_viewports_manifest_json() -> &'static str {
        r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "supported_viewports": [
                {
                    "type": "rectangular",
                    "min_width": 317,
                    "max_width": 317,
                    "min_height": 238,
                    "max_height": 238,
                    "min_dpi": 1,
                    "max_dpi": 1
                }
            ]
        }"#
    }

    #[test]
    fn parse_supported_viewports_manifest() {
        let manifest =
            Manifest::from_str(minimal_viewports_manifest_json()).expect("BUG: should parse");
        assert_eq!(manifest.supported_viewports.len(), 1);
        let vp = &manifest.supported_viewports[0];
        assert_eq!(vp.viewport_shape, ViewportShape::Rectangular);
        assert_eq!(vp.min_width, Some(317));
        assert_eq!(vp.max_width, Some(317));
        assert_eq!(vp.min_height, Some(238));
        assert_eq!(vp.max_height, Some(238));
        assert_eq!(vp.min_dpi, Some(1));
        assert_eq!(vp.max_dpi, Some(1));
    }

    #[test]
    fn omitted_viewport_bounds_parse_as_unbounded() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "binary": "bin/test",
            "supported_viewports": [
                {
                    "type": "rectangular",
                    "min_height": 238,
                    "max_height": 480
                }
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: optional bounds must parse");
        let vp = &manifest.supported_viewports[0];
        assert_eq!(vp.min_width, None);
        assert_eq!(vp.max_width, None);
        assert_eq!(vp.min_height, Some(238));
        assert_eq!(vp.max_height, Some(480));
        assert_eq!(vp.min_dpi, None);
        assert_eq!(vp.max_dpi, None);
    }

    #[test]
    fn legacy_sizes_normalize_to_exact_constraints() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small", "medium", "large", "full"]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: legacy sizes must normalize");
        #[expect(
            clippy::type_complexity,
            reason = "test-only tuple for compact assertion"
        )]
        let got: Vec<(Option<u32>, Option<u32>, Option<u32>, Option<u32>)> = manifest
            .supported_viewports
            .iter()
            .map(|c| (c.min_width, c.max_width, c.min_height, c.max_height))
            .collect();
        assert_eq!(
            got,
            vec![
                (Some(317), Some(317), Some(238), Some(238)),
                (Some(638), Some(638), Some(238), Some(238)),
                (Some(638), Some(638), Some(480), Some(480)),
                (Some(1280), Some(1280), Some(480), Some(480)),
            ]
        );
        assert!(
            manifest
                .supported_viewports
                .iter()
                .all(|c| c.viewport_shape == ViewportShape::Rectangular
                    && c.min_dpi.is_none()
                    && c.max_dpi.is_none())
        );
    }

    #[test]
    fn reject_empty_supported_viewports() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "supported_viewports": []
        }"#;
        assert!(matches!(
            Manifest::from_str(json),
            Err(ManifestError::EmptyViewports)
        ));
    }

    #[test]
    fn reject_zero_dimension_constraint() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"rectangular","min_width":0,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        assert!(matches!(
            Manifest::from_str(json),
            Err(ManifestError::InvalidViewport(_))
        ));
    }

    #[test]
    fn reject_min_greater_than_max_constraint() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"rectangular","min_width":640,"max_width":320,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        assert!(matches!(
            Manifest::from_str(json),
            Err(ManifestError::InvalidViewport(_))
        ));
    }

    #[test]
    fn reject_unspecified_viewport_shape() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"unspecified","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        assert!(Manifest::from_str(json).is_err());
    }

    #[test]
    fn reject_duplicate_constraints() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1},
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        assert!(matches!(
            Manifest::from_str(json),
            Err(ManifestError::DuplicateViewport)
        ));
    }

    #[test]
    fn reject_both_sizes_and_supported_viewports() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["small"],
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        assert!(matches!(
            Manifest::from_str(json),
            Err(ManifestError::MixedSizesAndViewports)
        ));
    }

    #[test]
    fn reject_neither_sizes_nor_supported_viewports() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test"
        }"#;
        assert!(matches!(
            Manifest::from_str(json),
            Err(ManifestError::EmptyViewports)
        ));
    }

    #[test]
    fn reject_unrecognized_legacy_size() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": ["enormous"]
        }"#;
        assert!(Manifest::from_str(json).is_err());
    }
}
