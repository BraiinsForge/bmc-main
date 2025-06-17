// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::utils::replace_file;
use anyhow::Result;
use bmc_display::data::{Scene, Widget};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tracing::warn;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ConfigRoot {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    scenes: Vec<Scene>,
}

#[derive(Clone, Debug)]
pub struct DisplayConfigHandle {
    path: PathBuf,
    data: ConfigRoot,
}

impl DisplayConfigHandle {
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

    pub fn add_widget(&mut self, scene_id: Option<u32>, widget: Widget) {
        // TODO: Remove after scene ID functionality is properly implemented
        let scene_id = scene_id.unwrap_or_default();

        if let Some(scene) = self.data.scenes.iter_mut().find(|s| s.id == scene_id) {
            scene.widgets.push(widget);
        }
    }

    async fn load_from_file(&mut self) -> Result<()> {
        let display_config_data = fs::read_to_string(&self.path).await?;
        self.data = serde_json::from_str(display_config_data.as_str())?;

        Ok(())
    }

    pub async fn sync_to_storage(&self) -> Result<()> {
        let config_data = serde_json::to_string_pretty(&self.data)?;
        replace_file(&self.path, config_data.as_bytes()).await?;

        Ok(())
    }
}
