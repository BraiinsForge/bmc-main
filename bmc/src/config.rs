// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::utils::{NumberFormat, replace_file};
use anyhow::{Context, Result, bail};
use bmc_display::data::{
    Scene, SceneCycling, SceneId, SceneKind, WidgetSize, deserialize_scenes, serialize_scenes,
};
use bmc_shared_time::time::{DateFormat, TimeSystem};
use chrono::NaiveTime;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use tokio::fs;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(
        serialize_with = "serialize_scenes",
        deserialize_with = "deserialize_scenes"
    )]
    pub scenes: IndexMap<SceneId, Scene>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene_cycling: Option<SceneCycling>,
    #[serde(skip_serializing_if = "Option::is_none")]
    localization: Option<LocalizationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_collection: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    night_mode: Option<NightModeConfigData>,
}

impl Config {
    pub fn scene_cycling(&self) -> SceneCycling {
        self.scene_cycling.clone().unwrap_or_default()
    }

    pub fn set_scene_cycling(&mut self, config: SceneCycling) {
        self.scene_cycling = Some(config);
    }

    pub fn localization_config(&self) -> LocalizationConfig {
        self.localization.clone().unwrap_or_default()
    }

    pub fn set_time_system(&mut self, time_system: TimeSystem) {
        self.localization.get_or_insert_default().time_system = time_system;
    }

    pub fn set_number_format(&mut self, number_format: NumberFormat) {
        self.localization.get_or_insert_default().number_format = number_format;
    }

    pub fn set_date_format(&mut self, date_format: DateFormat) {
        self.localization.get_or_insert_default().date_format = date_format;
    }

    #[expect(
        dead_code,
        reason = "data_collection will be used in system settings page"
    )]
    pub fn data_collection(&self) -> bool {
        self.data_collection.unwrap_or_default()
    }

    pub fn set_data_collection(&mut self, data_collection: bool) {
        self.data_collection = Some(data_collection);
    }

    fn validate(&self) -> Result<()> {
        self.validate_scenes()
    }

    fn validate_scenes(&self) -> Result<()> {
        for scene in self.scenes.values() {
            if scene
                .cycle_duration
                .is_some_and(|duration| duration < Scene::MIN_CYCLE_DURATION)
            {
                bail!("Duration for scene `{}` is too short", scene.id);
            }

            match scene.kind {
                SceneKind::Fullscreen => {
                    if scene.widgets.len() != 1 {
                        bail!(
                            "Fullscreen scene `{}` does not have exactly one widget",
                            scene.id
                        );
                    }

                    let widget = &scene.widgets[0];
                    if widget.position.row != 0 || widget.position.col != 0 {
                        bail!(
                            "Fullscreen scene `{}` has widget `{}` with incorrect position (expected row=0, col=0)",
                            scene.id,
                            widget.id
                        );
                    }
                    if widget.size != WidgetSize::Full {
                        bail!(
                            "Fullscreen scene `{}` has widget `{}` with incorrect size (expected `full`)",
                            scene.id,
                            widget.id
                        );
                    }
                }
                SceneKind::Combined => {
                    for widget in scene.widgets.values() {
                        if widget.size == WidgetSize::Full {
                            bail!(
                                "Combined scene `{}` has widget `{}` with incorrect size (expected `small`, `medium` or `large`)",
                                scene.id,
                                widget.id
                            );
                        }

                        if !widget.in_bounds() {
                            bail!(
                                "Combined scene `{}` has widget `{}` which is out of bounds (position + size)",
                                scene.id,
                                widget.id
                            );
                        }
                    }

                    for (widget, other_widget) in scene.widgets.values().tuple_combinations() {
                        if widget.overlaps(other_widget) {
                            bail!(
                                "Combined scene `{}` has widget `{}` which overlaps with widget `{}`",
                                scene.id,
                                widget.id,
                                other_widget.id
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LocalizationConfig {
    pub time_system: TimeSystem,
    pub number_format: NumberFormat,
    pub date_format: DateFormat,
}

#[derive(Clone, Debug)]
pub struct ConfigHandle {
    path: PathBuf,
    config: Config,
    default_brightness_pct: u8,
    default_night_mode_brightness_pct: u8,
}

impl ConfigHandle {
    pub async fn init(
        path: PathBuf,
        default_brightness_pct: u8,
        default_night_mode_brightness_pct: u8,
    ) -> Result<Self> {
        let config_data = fs::read_to_string(&path).await?;
        let config: Config = serde_json::from_str(config_data.as_str())?;

        config.validate().context("Config validation failed")?;

        Ok(Self {
            path,
            config,
            default_brightness_pct,
            default_night_mode_brightness_pct,
        })
    }

    pub async fn sync_to_storage(&mut self) -> Result<()> {
        let config_data = serde_json::to_string_pretty(&self.config)?;
        replace_file(&self.path, config_data.as_bytes()).await?;

        Ok(())
    }

    pub fn set_brightness(&mut self, brightness_pct: u8) {
        self.brightness_pct = Some(brightness_pct);
    }

    pub fn brightness_pct(&self) -> u8 {
        self.brightness_pct.unwrap_or(self.default_brightness_pct)
    }

    pub fn night_mode(&self) -> NightModeConfig {
        self.night_mode
            .clone()
            .unwrap_or_default()
            .into_night_mode_config(self.default_night_mode_brightness_pct)
    }

    pub fn set_night_mode_enabled(&mut self, enabled: bool) {
        if let Some(ref mut night_mode) = self.night_mode {
            night_mode.enabled = enabled;
        } else {
            let night_mode = NightModeConfigData {
                enabled,
                ..Default::default()
            };
            self.night_mode = Some(night_mode);
        }
    }

    pub fn set_night_mode_brightness(&mut self, brightness_pct: u8) {
        if let Some(ref mut night_mode) = self.night_mode {
            night_mode.brightness_pct = Some(brightness_pct);
        } else {
            let night_mode = NightModeConfigData {
                brightness_pct: Some(brightness_pct),
                ..Default::default()
            };
            self.night_mode = Some(night_mode);
        }
    }

    pub fn set_night_mode_interval(&mut self, from: NaiveTime, to: NaiveTime) {
        if let Some(ref mut night_mode) = self.night_mode {
            night_mode.from = from;
            night_mode.to = to;
        } else {
            let night_mode = NightModeConfigData {
                from,
                to,
                ..Default::default()
            };

            self.night_mode = Some(night_mode);
        }
    }
}

impl Deref for ConfigHandle {
    type Target = Config;

    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl DerefMut for ConfigHandle {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.config
    }
}

impl AsRef<Config> for ConfigHandle {
    fn as_ref(&self) -> &Config {
        self
    }
}

impl AsMut<Config> for ConfigHandle {
    fn as_mut(&mut self) -> &mut Config {
        self
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct NightModeConfigData {
    enabled: bool,
    from: NaiveTime,
    to: NaiveTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness_pct: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct NightModeConfig {
    pub enabled: bool,
    pub from: NaiveTime,
    pub to: NaiveTime,
    pub brightness_pct: u8,
}

impl NightModeConfigData {
    fn default_from() -> NaiveTime {
        NaiveTime::from_hms_opt(22, 30, 0).expect("BUG: Invalid default night mode interval")
    }

    fn default_to() -> NaiveTime {
        NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: Invalid default night mode interval")
    }

    fn into_night_mode_config(self, default_brightness: u8) -> NightModeConfig {
        NightModeConfig {
            enabled: self.enabled,
            from: self.from,
            to: self.to,
            brightness_pct: self.brightness_pct.unwrap_or(default_brightness),
        }
    }
}

impl Default for NightModeConfigData {
    fn default() -> Self {
        Self {
            enabled: false,
            from: NightModeConfigData::default_from(),
            to: NightModeConfigData::default_to(),
            brightness_pct: None,
        }
    }
}
