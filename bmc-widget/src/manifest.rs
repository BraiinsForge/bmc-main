// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;

use bmc_ipc::SizeType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ManifestError;

const MAX_NAME_LENGTH: usize = 50;
const MAX_DESCRIPTION_LENGTH: usize = 200;

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
    params: HashMap<String, ParamDefinition>,
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
    pub params: HashMap<String, ParamDefinition>,
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

        for (name, param) in &self.params {
            param.validate(name)?;
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
    #[serde(rename = "type")]
    pub param_type: ParamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub default: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "enum")]
    pub enum_values: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl ParamDefinition {
    fn validate(&self, param_name: &str) -> Result<(), ManifestError> {
        match self.param_type {
            ParamType::String => {
                if !self.default.is_string() {
                    return Err(ManifestError::ParamDefaultTypeMismatch {
                        name: param_name.to_owned(),
                    });
                }
                if let Some(ref enum_vals) = self.enum_values {
                    if enum_vals.is_empty() {
                        return Err(ManifestError::ParamEnumMissing {
                            name: param_name.to_owned(),
                        });
                    }
                }
            }
            ParamType::Boolean => {
                if !self.default.is_boolean() {
                    return Err(ManifestError::ParamDefaultTypeMismatch {
                        name: param_name.to_owned(),
                    });
                }
            }
            ParamType::Number => {
                if !self.default.is_number() {
                    return Err(ManifestError::ParamDefaultTypeMismatch {
                        name: param_name.to_owned(),
                    });
                }
            }
            ParamType::Array => {
                if !self.default.is_array() {
                    return Err(ManifestError::ParamDefaultTypeMismatch {
                        name: param_name.to_owned(),
                    });
                }
            }
            ParamType::Timezone => {
                if !self.default.is_null() && !self.default.is_string() {
                    return Err(ManifestError::ParamDefaultTypeMismatch {
                        name: param_name.to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    String,
    Boolean,
    Number,
    Array,
    Timezone,
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
                    "default": "digital",
                    "enum": {
                        "digital": "Digital",
                        "analog": "Analog"
                    }
                },
                "showSeconds": {
                    "name": "Show Seconds",
                    "type": "boolean",
                    "description": "Display seconds on the clock",
                    "default": false
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
        assert_eq!(style_param.default, serde_json::json!("digital"));
    }

    #[test]
    fn reject_non_v4_uuid() {
        // UUID v1 (time-based)
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
                    "default": "not a boolean"
                }
            }
        }"#;
        let result = Manifest::from_str(json);
        assert!(matches!(
            result,
            Err(ManifestError::ParamDefaultTypeMismatch { .. })
        ));
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
                    "type": "number",
                    "min": 0,
                    "max": 100,
                    "default": 50
                }
            }
        }"#;
        let manifest = Manifest::from_str(json).expect("BUG: should parse");
        let param = manifest
            .params
            .get("brightness")
            .expect("BUG: should have brightness");
        assert_eq!(param.param_type, ParamType::Number);
        assert_eq!(param.min, Some(0.0));
        assert_eq!(param.max, Some(100.0));
    }
}
