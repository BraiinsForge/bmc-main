// Copyright (C) 2025  Braiins Systems s.r.o.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to parse manifest JSON: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    #[error("UUID must be version 4, got version {0}")]
    InvalidUuidVersion(usize),

    #[error("invalid version '{version}': {source}")]
    InvalidVersion {
        version: String,
        source: semver::Error,
    },

    #[error("name exceeds maximum length of {max} characters")]
    NameTooLong { max: usize },

    #[error("description exceeds maximum length of {max} characters")]
    DescriptionTooLong { max: usize },

    #[error("sizes array must not be empty")]
    EmptySizes,

    #[error("invalid size type: {0}")]
    InvalidSizeType(String),

    #[error("invalid setting key: {0}")]
    InvalidSettingKey(String),

    #[error("parameter {name:?}: {reason}")]
    InvalidParam { name: String, reason: String },

    #[error("duplicate parameter key: {0:?}")]
    DuplicateParamKey(String),
}
