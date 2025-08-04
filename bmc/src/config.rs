// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::utils::{NumberFormat, replace_file};
use anyhow::{Context, Result, bail};
use bmc_display::data::{
    Scene, SceneId, SceneKind, WidgetSize, deserialize_scenes, serialize_scenes,
};
use bmc_shared_time::time::{DateFormat, TimeSystem};
use indexmap::IndexMap;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ConfigRoot {
    #[serde(
        serialize_with = "serialize_scenes",
        deserialize_with = "deserialize_scenes"
    )]
    pub scenes: IndexMap<SceneId, Scene>,
    #[serde(skip_serializing_if = "Option::is_none")]
    localization: Option<LocalizationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_collection: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness_pct: Option<u8>,
}

impl ConfigRoot {
    fn validate(&self) -> Result<()> {
        self.validate_scenes()
    }

    fn validate_scenes(&self) -> Result<()> {
        for scene in self.scenes.values() {
            if scene.cycle_duration < Scene::MIN_CYCLE_DURATION {
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

#[derive(Clone, Debug)]
pub struct ConfigHandle {
    path: PathBuf,
    pub(crate) data: ConfigRoot,
    default_brightness_pct: u8,
}

impl ConfigHandle {
    pub async fn init(path: PathBuf, default_brightness_pct: u8) -> Result<Self> {
        let config_data = fs::read_to_string(&path).await?;
        let data: ConfigRoot = serde_json::from_str(config_data.as_str())?;

        data.validate().context("Config validation failed")?;

        Ok(Self {
            path,
            data,
            default_brightness_pct,
        })
    }

    pub async fn sync_to_storage(&self) -> Result<()> {
        let config_data = serde_json::to_string_pretty(&self.data)?;
        replace_file(&self.path, config_data.as_bytes()).await?;

        Ok(())
    }

    pub fn localization_config(&self) -> LocalizationConfig {
        self.data.localization.clone().unwrap_or_default()
    }

    pub fn set_time_system(&mut self, time_system: TimeSystem) {
        self.data.localization.get_or_insert_default().time_system = time_system;
    }

    pub fn set_number_format(&mut self, number_format: NumberFormat) {
        self.data.localization.get_or_insert_default().number_format = number_format;
    }

    pub fn set_date_format(&mut self, date_format: DateFormat) {
        self.data.localization.get_or_insert_default().date_format = date_format;
    }

    #[expect(
        dead_code,
        reason = "data_collection will be used in system settings page"
    )]
    pub fn data_collection(&self) -> bool {
        self.data.data_collection.unwrap_or_default()
    }

    pub fn set_data_collection(&mut self, data_collection: bool) {
        self.data.data_collection = Some(data_collection);
    }

    pub fn set_brightness_pct(&mut self, brightness_pct: u8) {
        self.data.brightness_pct = Some(brightness_pct);
    }

    pub fn brightness_pct(&self) -> u8 {
        self.data
            .brightness_pct
            .unwrap_or(self.default_brightness_pct)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub(crate) struct LocalizationConfig {
    pub(crate) time_system: TimeSystem,
    pub(crate) number_format: NumberFormat,
    pub(crate) date_format: DateFormat,
}
