// Copyright (C) 2025  Braiins Systems s.r.o.

//! Serde types matching the on-disk shape of the slint-monolith
//! config. Deserialize-only; we never write these back out.

use serde::Deserialize;
use uuid::Uuid;

/// Top-level legacy config. Mirrors `/etc/bmc_config.json` from a
/// device running the old firmware.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub scenes: Vec<Scene>,
    /// Pass-through. The shape is identical on both sides so we keep
    /// it as raw JSON and let the new config re-parse it.
    #[serde(default)]
    pub accounts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Scene {
    pub id: Uuid,
    pub enabled: bool,
    pub kind: SceneKind,
    #[serde(default)]
    pub widgets: Vec<Widget>,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SceneKind {
    Fullscreen,
    Combined,
}

/// A legacy widget keyed by its `kind` string. The new system uses a
/// manifest UID instead; the translator maps `kind` + `params` to a
/// `widget_type_id` and a normalized param map.
#[derive(Debug, Clone, Deserialize)]
pub struct Widget {
    pub id: Uuid,
    pub row: u8,
    pub col: u8,
    pub size: String,
    pub kind: String,
    #[serde(default)]
    pub params: serde_json::Value,
}
