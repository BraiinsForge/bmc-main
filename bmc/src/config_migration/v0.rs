// Copyright (C) 2025  Braiins Systems s.r.o.

//! v0 schema — the slint-monolith era shape of the BMC config
//! (lived at `/etc/bmc_config.json` before the `/etc/bmc/` folder
//! move; see [`super::relocate_legacy_config_if_present`]).
//!
//! Deserialize-only: we never write this shape back out. A parsed
//! v0 config is fed into [`upgrade_v0::upgrade_with_report`](super::upgrade_v0)
//! to produce the current schema.

use serde::Deserialize;
use uuid::Uuid;

/// Top-level v0 config. Mirrors the on-disk shape written by the
/// old firmware (legacy path `/etc/bmc_config.json`, now relocated
/// to `/etc/bmc/config.json` on first boot of the new firmware).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub scenes: Vec<Scene>,
    /// Pass-through. The shape is identical on both sides so we
    /// keep it as raw JSON and let the current config consume it
    /// unchanged.
    #[serde(default)]
    pub accounts: Vec<serde_json::Value>,
    /// Top-level settings, also pass-through: their shapes did not
    /// change with the widget schema. The upgrade re-parses each one
    /// into its current typed form and drops (with a warning) any
    /// single field that fails — never the whole migration.
    pub scene_cycling: Option<serde_json::Value>,
    pub localization: Option<serde_json::Value>,
    pub data_collection: Option<serde_json::Value>,
    pub brightness_pct: Option<serde_json::Value>,
    pub night_mode: Option<serde_json::Value>,
    pub sound_volume_pct: Option<serde_json::Value>,
    pub alarms: Option<serde_json::Value>,
    pub led_enabled: Option<serde_json::Value>,
    pub boot_sound_enabled: Option<serde_json::Value>,
    pub autoupgrade: Option<serde_json::Value>,
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
