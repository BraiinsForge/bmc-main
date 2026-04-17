// Copyright (C) 2025  Braiins Systems s.r.o.

//! v0 schema — the slint-monolith era shape of `/etc/bmc_config.json`.
//!
//! Deserialize-only: we never write this shape back out. A parsed
//! v0 config is fed into the [`Upgrade`](super::Upgrade) chain that
//! produces the current schema.

use serde::Deserialize;
use uuid::Uuid;

use super::Version;

/// Top-level v0 config. Mirrors `/etc/bmc_config.json` from a
/// device running the old firmware.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub scenes: Vec<Scene>,
    /// Pass-through. The shape is identical on both sides so we
    /// keep it as raw JSON and let the current config consume it
    /// unchanged.
    #[serde(default)]
    pub accounts: Vec<serde_json::Value>,
}

impl Version for Config {
    const VERSION: u32 = 0;
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

/// A v0 widget keyed by its `kind` string. The current system uses
/// a manifest UID instead; the upgrade step maps `kind` + `params`
/// to a `widget_type_id` and a normalized param map.
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
