// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;

use bmc_ipc::SizeType;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::ManifestError;

const MAX_NAME_LENGTH: usize = 50;
const MAX_DESCRIPTION_LENGTH: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ParamKey(String);

impl ParamKey {
    /// Construct a `ParamKey` from an owned string, applying the same
    /// character-class rules as the `Deserialize` impl. Returns the
    /// rejected input back to the caller on failure.
    pub fn try_new(s: String) -> Result<Self, String> {
        if Self::is_valid(&s) {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }

    fn is_valid(s: &str) -> bool {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StringOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoubleOption {
    pub value: f64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegerOption {
    pub value: i32,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StringFormat {
    Date,
    Time,
    Email,
    Uri,
}

/// Typed scalar value for a stored widget param. Mirrors the manifest's
/// `ParamKind` value space — null, boolean, i32, finite f64, string —
/// nothing wider. The on-disk form is internally tagged so integer vs
/// double survives a JSON round-trip without needing the manifest.
///
/// Construct `Double` only with finite values; the `Deserialize` impl
/// rejects NaN/inf, and validators upstream should reject them too.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ParamValue {
    Null,
    Boolean(bool),
    Integer(i32),
    #[serde(deserialize_with = "deserialize_finite_f64")]
    Double(f64),
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
    /// JSON projection for the wayland boundary — widget processes
    /// receive a JSON object whose values are bare scalars, not the
    /// internally-tagged form.
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

    /// Build the default value for a manifest param. Required params
    /// without a `default_value` are caught at manifest load time
    /// (`ParamKind::has_default_value`); optional params without a
    /// default deserialize to `Null`.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RawManifest {
    uid: Uuid,
    version: String,
    name: String,
    description: String,
    #[serde(default)]
    author: Option<Author>,
    binary: PathBuf,
    #[serde(default)]
    settings: Vec<SettingKey>,
    sizes: Vec<SizeType>,
    #[serde(default)]
    params: HashMap<ParamKey, ParamDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Manifest {
    pub uid: Uuid,
    pub version: semver::Version,
    pub name: String,
    pub description: String,
    pub author: Option<Author>,
    pub binary: PathBuf,
    pub settings: Vec<SettingKey>,
    pub sizes: Vec<SizeType>,
    pub params: HashMap<ParamKey, ParamDefinition>,
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
                version: raw.version,
                source: e,
            })?;

        let manifest = Self {
            uid: raw.uid,
            version,
            name: raw.name,
            description: raw.description,
            author: raw.author,
            binary: raw.binary,
            settings: raw.settings,
            sizes: raw.sizes,
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

        if self.description.len() > MAX_DESCRIPTION_LENGTH {
            return Err(ManifestError::DescriptionTooLong {
                max: MAX_DESCRIPTION_LENGTH,
            });
        }

        if self.sizes.is_empty() {
            return Err(ManifestError::EmptySizes);
        }

        for (key, param) in &self.params {
            param.validate(key.as_str())?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingKey {
    Localization,
    Timezone,
    NightMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        rename = "optional",
        skip_serializing_if = "core::ops::Not::not"
    )]
    pub is_optional: bool,
    #[serde(flatten)]
    pub kind: ParamKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ParamKind {
    String {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<StringFormat>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        enum_values: Vec<StringOption>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<String>,
    },
    Double {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        enum_values: Vec<DoubleOption>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<f64>,
    },
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<i32>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        enum_values: Vec<IntegerOption>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<i32>,
    },
    Boolean {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_value: Option<bool>,
    },
    Timezone {
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
            ParamKind::Boolean { .. } | ParamKind::Timezone { .. } => {}
        }
        Ok(())
    }
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

fn check_string_options(options: &[StringOption]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for o in options {
        if o.value.is_empty() {
            return Err(
                "enum_values entry value must be non-empty (collides with FE \"no selection\" sentinel)"
                    .into(),
            );
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
            if a.value.to_bits() == b.value.to_bits() {
                return Err(format!("duplicate enum_values entry value {}", a.value));
            }
        }
    }
    if !options.is_empty()
        && let Some(d) = default_value
        && !options.iter().any(|o| o.value.to_bits() == d.to_bits())
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
            "sizes": ["small"]
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
        assert_eq!(manifest.sizes, vec![SizeType::Small]);
        assert!(manifest.author.is_none());
        assert!(manifest.settings.is_empty());
        assert!(manifest.params.is_empty());
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
        assert_eq!(
            manifest.sizes,
            vec![
                SizeType::Small,
                SizeType::Medium,
                SizeType::Large,
                SizeType::Full
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
    fn reject_empty_sizes() {
        let json = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0.0",
            "name": "Test",
            "description": "Test",
            "binary": "bin/test",
            "sizes": []
        }"#;
        let result = Manifest::from_str(json);
        assert!(matches!(result, Err(ManifestError::EmptySizes)));
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
    fn param_value_double_rejects_non_finite_on_deserialize() {
        for s in [
            r#"{"type":"double","value":null}"#,
            r#"{"type":"integer","value":1.5}"#,
        ] {
            assert!(
                serde_json::from_str::<ParamValue>(s).is_err(),
                "expected reject: {s}"
            );
        }
    }

    #[test]
    fn param_value_to_json_value_strips_tag() {
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
}
