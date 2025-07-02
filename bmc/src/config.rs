// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::utils::{NumberFormat, replace_file};
use anyhow::Result;
use bmc_display::data::{Scene, Widget};
use bmc_shared_time::time::{DateFormat, TimeSystem};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tracing::warn;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ConfigRoot {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    scenes: Vec<Scene>,
    #[serde(skip_serializing_if = "Option::is_none")]
    localization: Option<LocalizationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_collection: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct ConfigHandle {
    path: PathBuf,
    data: ConfigRoot,
}

impl ConfigHandle {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            data: ConfigRoot::default(),
        }
    }

    pub async fn init(&mut self) {
        if let Err(e) = self.load_from_file().await {
            warn!("Invalid format of display config file: {}", e);
        }
    }

    pub fn scenes(&self) -> Vec<Scene> {
        self.data.scenes.clone()
    }

    pub fn add_widget(&mut self, scene_id: Option<u32>, widget: Widget) {
        // TODO: Remove after scene ID functionality is properly implemented
        let scene_id = scene_id.unwrap_or_default();

        // NOTE: In case of empty `scenes` vector new widget won't be added since `find` will
        // return None
        if let Some(scene) = self.data.scenes.iter_mut().find(|s| s.id == scene_id) {
            scene.widgets.push(widget);
        }
    }

    async fn load_from_file(&mut self) -> Result<()> {
        let config_data = fs::read_to_string(&self.path).await?;
        self.data = serde_json::from_str(config_data.as_str())?;

        Ok(())
    }

    pub async fn sync_to_storage(&self) -> Result<()> {
        let config_data = serde_json::to_string_pretty(&self.data)?;
        replace_file(&self.path, config_data.as_bytes()).await?;

        Ok(())
    }

    #[expect(
        dead_code,
        reason = "localization_config will be used in settings page"
    )]
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
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub(crate) struct LocalizationConfig {
    pub(crate) time_system: TimeSystem,
    pub(crate) number_format: NumberFormat,
    pub(crate) date_format: DateFormat,
}
