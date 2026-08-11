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

pub use bmc_field_schema::credential;
pub use bmc_field_schema::{
    DoubleOption, FieldSchemaError, IntegerOption, MAX_PARAM_KEY_LENGTH, MAX_PARAM_STRING_LENGTH,
    ParamDefinition, ParamKey, ParamKind, ParamValue, ParamValueConversionError, StringFormat,
    StringOption, f64_canonical_bits,
};
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type CredentialKey = ParamKey;

const MAX_NAME_LENGTH: usize = 50;
const MAX_SUBNAME_LENGTH: usize = 30;
const MAX_DESCRIPTION_LENGTH: usize = 200;
/// Shorter than the widget's own name: a slot label sits inline beside a picker.
const MAX_CREDENTIAL_LABEL_LENGTH: usize = 40;
const MAX_CONFIG_HELP_LENGTH: usize = 2_000;

/// Errors produced by [`Manifest::from_str`] and the structural / semantic
/// validators it dispatches to. Each variant names the rule that was violated,
/// so a downstream caller can surface them without re-parsing the message.
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
    /// Widgets are required to use random UUIDs so that
    /// any new widget gets a fresh identifier without coordination.
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

    /// A credential slot's `label` exceeded the declared length cap.
    #[error("credential slot {slot:?}: label exceeds maximum length of {max} characters")]
    CredentialLabelTooLong { slot: String, max: usize },

    /// A credential slot's `description` exceeded the declared length cap.
    #[error("credential slot {slot:?}: description exceeds maximum length of {max} characters")]
    CredentialDescriptionTooLong { slot: String, max: usize },

    /// The `config_help` field exceeded the declared length cap.
    #[error("config_help exceeds maximum length of {max} characters")]
    ConfigHelpTooLong { max: usize },

    /// `supported_viewports` was empty after compatibility normalization.
    #[error("supported_viewports must not be empty")]
    EmptyViewports,

    /// A viewport constraint violated a numeric rule (zero provided bound, or min > max).
    #[error("invalid viewport constraint: {0}")]
    InvalidViewport(String),

    /// Two viewport constraints had identical display type and all six bounds.
    #[error("duplicate viewport constraint")]
    DuplicateViewport,

    /// A `settings` entry was not a recognised [`SettingKey`] variant.
    #[error("invalid setting key: {0}")]
    InvalidSettingKey(String),

    /// A field-schema validation failure in the manifest's `params` map,
    /// forwarded from the shared field-schema validator.
    #[error(transparent)]
    Field(#[from] FieldSchemaError),

    /// Lists the valid ids inline: discovery only logs this plus the manifest path.
    #[error("credential slot {slot:?}: unknown credential type {type_id:?} (valid types: {valid})")]
    UnknownCredentialType {
        slot: String,
        type_id: String,
        valid: String,
    },

    #[error("credential slot {slot:?}: label must not be empty")]
    EmptyCredentialLabel { slot: String },
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
    config_help: Option<String>,
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
    supported_viewports: Vec<WidgetViewportConstraint>,
    #[serde(
        default,
        deserialize_with = "bmc_field_schema::deserialize_unique_params"
    )]
    params: IndexMap<ParamKey, ParamDefinition>,
    #[serde(default, deserialize_with = "deserialize_unique_credentials")]
    credentials: IndexMap<CredentialKey, CredentialSlot>,
}

fn deserialize_unique_credentials<'de, D>(
    deserializer: D,
) -> Result<IndexMap<CredentialKey, CredentialSlot>, D::Error>
where
    D: Deserializer<'de>,
{
    bmc_field_schema::deserialize_unique_keyed(deserializer, "credential slot")
}

/// One credential slot a widget declares: which kind of account it accepts, and how the operator
/// sees it in the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CredentialSlot {
    /// Id of the credential type this slot accepts,
    /// from the firmware catalog ([`credential::builtins`]).
    #[serde(rename = "type")]
    pub type_id: String,
    /// Operator-facing slot label, shown beside the account picker.
    /// Must not be blank, and is capped like the widget's own name
    /// — it reaches the editor's picker unabridged.
    #[schemars(length(max = 40))]
    pub label: String,
    /// Help text shown under the picker, for a slot whose label leaves something unsaid.
    /// Optional on purpose: requiring it only buys descriptions that restate the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 200))]
    pub description: Option<String>,
    /// Whether the widget cannot work without this slot bound.
    ///
    /// Defaults to true: an omitted key means the author did not consider it,
    /// and the cautious reading warns an operator about an unbound slot
    /// rather than staying silent.
    #[serde(default = "required_by_default")]
    pub required: bool,
}

/// `serde(default)` would give `false`, inverting the documented default.
fn required_by_default() -> bool {
    true
}

impl CredentialSlot {
    fn validate(&self, slot: &str) -> Result<(), ManifestError> {
        if self.label.trim().is_empty() {
            return Err(ManifestError::EmptyCredentialLabel {
                slot: slot.to_owned(),
            });
        }

        if self.label.len() > MAX_CREDENTIAL_LABEL_LENGTH {
            return Err(ManifestError::CredentialLabelTooLong {
                slot: slot.to_owned(),
                max: MAX_CREDENTIAL_LABEL_LENGTH,
            });
        }

        if self
            .description
            .as_ref()
            .is_some_and(|d| d.len() > MAX_DESCRIPTION_LENGTH)
        {
            return Err(ManifestError::CredentialDescriptionTooLong {
                slot: slot.to_owned(),
                max: MAX_DESCRIPTION_LENGTH,
            });
        }

        let catalog = credential::builtins();
        if !catalog.iter().any(|t| t.id == self.type_id) {
            let mut valid: Vec<&str> = catalog.iter().map(|t| t.id.as_str()).collect();
            valid.sort_unstable();
            return Err(ManifestError::UnknownCredentialType {
                slot: slot.to_owned(),
                type_id: self.type_id.clone(),
                valid: valid.join(", "),
            });
        }

        Ok(())
    }
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
    /// Optional Markdown shown in the operator's widget-config window: a
    /// preface above the parameter fields, and the sole content when the
    /// widget declares no parameters. Capped at 2000 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 2000))]
    pub config_help: Option<String>,
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
    /// Viewport families this widget supports. Must be non-empty.
    #[schemars(length(min = 1))]
    pub supported_viewports: Vec<WidgetViewportConstraint>,
    /// Per-instance parameter declarations, keyed by [`ParamKey`].
    /// Iteration order matches manifest order.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub params: IndexMap<ParamKey, ParamDefinition>,
    /// Credential slots the operator binds a saved account to, keyed by [`CredentialKey`].
    /// Iteration order matches manifest order.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub credentials: IndexMap<CredentialKey, CredentialSlot>,
}

impl FromStr for Manifest {
    type Err = ManifestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw: RawManifest = serde_json::from_str(s)?;
        Self::from_raw(raw)
    }
}

impl Manifest {
    /// Whether any declared viewport accepts a `shape` surface of
    /// `width`×`height` pixels.
    #[must_use]
    pub fn supports_viewport(&self, shape: ViewportShape, width: u32, height: u32) -> bool {
        self.supported_viewports
            .iter()
            .any(|v| v.admits_size(shape, width, height))
    }

    /// Whether any declared viewport accepts a `shape` surface
    /// of `width`×`height` pixels on a display of `dpi`.
    ///
    /// The DPI bounds are checked on the same constraint that admits the size,
    /// since a widget may declare a different density range per geometry.
    #[must_use]
    pub fn supports_viewport_at_dpi(
        &self,
        shape: ViewportShape,
        width: u32,
        height: u32,
        dpi: u32,
    ) -> bool {
        self.supported_viewports
            .iter()
            .any(|v| v.admits_size(shape, width, height) && v.admits_dpi(dpi))
    }

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

        let manifest = Self {
            uid: raw.uid,
            version,
            name: raw.name,
            subname: raw.subname,
            description: raw.description,
            config_help: raw.config_help,
            author: raw.author,
            binary: raw.binary,
            icon: raw.icon,
            category: raw.category,
            settings: raw.settings,
            supported_viewports: raw.supported_viewports,
            params: raw.params,
            credentials: raw.credentials,
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

        if self
            .config_help
            .as_ref()
            .is_some_and(|s| s.len() > MAX_CONFIG_HELP_LENGTH)
        {
            return Err(ManifestError::ConfigHelpTooLong {
                max: MAX_CONFIG_HELP_LENGTH,
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

        for (key, slot) in &self.credentials {
            slot.validate(key.as_str())?;
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
    /// Facts & trivia.
    Knowledge,
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

impl WidgetViewportConstraint {
    #[must_use]
    fn admits_size(&self, shape: ViewportShape, width: u32, height: u32) -> bool {
        self.viewport_shape == shape
            && self.min_width.is_none_or(|min| width >= min)
            && self.max_width.is_none_or(|max| width <= max)
            && self.min_height.is_none_or(|min| height >= min)
            && self.max_height.is_none_or(|max| height <= max)
    }

    #[must_use]
    fn admits_dpi(&self, dpi: u32) -> bool {
        self.min_dpi.is_none_or(|min| dpi >= min) && self.max_dpi.is_none_or(|max| dpi <= max)
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
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238},
                {"type":"rectangular","min_width":638,"max_width":638,
                 "min_height":238,"max_height":238},
                {"type":"rectangular","min_width":638,"max_width":638,
                 "min_height":480,"max_height":480},
                {"type":"rectangular","min_width":1280,"max_width":1280,
                 "min_height":480,"max_height":480}
            ],
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
            },
            "credentials": {
                "weather": {
                    "type": "generic-token",
                    "label": "Weather service",
                    "description": "Token for the forecast API",
                    "required": true
                },
                "media": {
                    "type": "generic-userpass",
                    "label": "Media server",
                    "required": false
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
    fn parse_config_help() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test Widget",
            "description": "A test widget",
            "config_help": "Use **markdown** here.",
            "binary": "bin/test",
            "supported_viewports": [
                {"type":"rectangular","min_width":317,"max_width":317,
                 "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        assert_eq!(
            manifest.config_help.as_deref(),
            Some("Use **markdown** here.")
        );
    }

    #[test]
    fn reject_config_help_too_long() {
        let json = format!(
            r#"{{
                "uid": "550e8400-e29b-41d4-a716-446655440000",
                "version": "1.0.0",
                "name": "Test Widget",
                "description": "A test widget",
                "config_help": "{}",
                "binary": "bin/test",
                "supported_viewports": [
                    {{"type":"rectangular","min_width":317,"max_width":317,
                     "min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}}
                ]
            }}"#,
            "x".repeat(MAX_CONFIG_HELP_LENGTH + 1)
        );
        let result = Manifest::from_str(&json);
        assert!(matches!(
            result,
            Err(ManifestError::ConfigHelpTooLong { .. })
        ));
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

        let slots: Vec<(&str, &str, bool)> = manifest
            .credentials
            .iter()
            .map(|(key, slot)| (key.as_str(), slot.type_id.as_str(), slot.required))
            .collect();
        assert_eq!(
            slots,
            vec![
                ("weather", "generic-token", true),
                ("media", "generic-userpass", false),
            ],
            "credential slots keep manifest order; an omitted `required` reads true, an explicit false is kept"
        );

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
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}]
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
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}]
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
                "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}]
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
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}],
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
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}],
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
            Err(FieldSchemaError::InvalidParam { .. })
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
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}],
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
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}],
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

    fn manifest_with_credentials(credentials: &str) -> String {
        format!(
            r#"{{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}],
            "credentials": {credentials}
        }}"#
        )
    }

    #[test]
    fn manifest_without_credentials_declares_none() {
        let manifest = Manifest::from_str(minimal_manifest_json()).expect("BUG: should parse");
        assert!(manifest.credentials.is_empty());
    }

    #[test]
    fn manifest_rejects_unknown_credential_type() {
        let json =
            manifest_with_credentials(r#"{"pool": {"type": "braiins_pool", "label": "Pool"}}"#);
        let err = Manifest::from_str(&json).expect_err("unknown credential type must fail");
        let msg = err.to_string();

        assert!(msg.contains(r#""pool""#), "names the slot: {msg}");
        assert!(
            msg.contains(r#""braiins_pool""#),
            "quotes the bad id: {msg}"
        );
        for valid in ["braiins-pool", "generic-token", "generic-userpass"] {
            assert!(msg.contains(valid), "lists {valid}: {msg}");
        }
    }

    #[test]
    fn manifest_rejects_blank_credential_label() {
        let json =
            manifest_with_credentials(r#"{"pool": {"type": "braiins-pool", "label": "   "}}"#);
        let err = Manifest::from_str(&json).expect_err("blank credential label must fail");
        assert!(
            err.to_string().contains("label"),
            "error must mention the label: {err}"
        );
    }

    #[test]
    fn manifest_rejects_an_over_long_credential_label() {
        let label = "l".repeat(MAX_CREDENTIAL_LABEL_LENGTH + 1);
        let json = manifest_with_credentials(&format!(
            r#"{{"pool": {{"type": "braiins-pool", "label": "{label}"}}}}"#
        ));
        let err = Manifest::from_str(&json).expect_err("an over-long label must fail");
        assert!(
            err.to_string().contains("pool") && err.to_string().contains("label"),
            "error must name the slot and the field: {err}"
        );
    }

    #[test]
    fn manifest_rejects_an_over_long_credential_description() {
        let description = "d".repeat(MAX_DESCRIPTION_LENGTH + 1);
        let json = manifest_with_credentials(&format!(
            r#"{{"pool": {{"type": "braiins-pool", "label": "Pool", "description": "{description}"}}}}"#
        ));
        let err = Manifest::from_str(&json).expect_err("an over-long description must fail");
        assert!(
            err.to_string().contains("description"),
            "error must mention the description: {err}"
        );
    }

    #[test]
    fn a_slot_that_omits_required_is_required() {
        // The opposite default would drop the editor's unbound warning
        // for exactly the manifests whose author never considered the field.
        let json =
            manifest_with_credentials(r#"{"pool": {"type": "braiins-pool", "label": "Pool"}}"#);
        let manifest = Manifest::from_str(&json).expect("BUG: should parse");

        assert!(manifest.credentials["pool"].required);
    }

    #[test]
    fn manifest_rejects_duplicate_credential_slots() {
        let json = manifest_with_credentials(
            r#"{
                "pool": {"type": "braiins-pool", "label": "First"},
                "pool": {"type": "generic-token", "label": "Second"}
            }"#,
        );
        let err = Manifest::from_str(&json).expect_err("duplicate credential slot must fail");
        assert!(
            err.to_string().contains("duplicate credential slot"),
            "error must name the duplicate slot kind: {err}"
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
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}],
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
    fn reject_missing_supported_viewports() {
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
}
