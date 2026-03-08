// Copyright (C) 2025  Braiins Systems s.r.o.

use std::process::Stdio;

use bmc_widget::env::vars;
use bmc_widget_protocol::SizeType;
use tokio::process::{Child, Command};

use super::WidgetInfo;
use super::coordinator::WidgetEnv;

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("failed to spawn process: {0}")]
    SpawnProcess(std::io::Error),

    #[error("failed to serialize params: {0}")]
    SerializeParams(serde_json::Error),

    #[error("failed to serialize localization: {0}")]
    SerializeLocalization(serde_json::Error),
}

#[derive(Debug)]
pub struct WaylandSpawner;

impl WaylandSpawner {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn spawn(
        &self,
        widget: &WidgetInfo,
        env: &WidgetEnv,
        xdg_runtime_dir: &str,
    ) -> Result<Child, SpawnError> {
        let params_json =
            serde_json::to_string(&env.params).map_err(SpawnError::SerializeParams)?;

        let mut cmd = Command::new(&widget.binary_path);

        // Standard Wayland environment
        cmd.env("WAYLAND_DISPLAY", &env.wayland_display)
            .env("XDG_RUNTIME_DIR", xdg_runtime_dir);

        // Widget configuration via DECK_* env vars
        cmd.env(vars::INSTANCE_ID, &env.instance_id)
            .env(vars::SIZE_TYPE, size_type_to_str(env.size_type))
            .env(vars::WIDTH, env.width.to_string())
            .env(vars::HEIGHT, env.height.to_string())
            .env(vars::PARAMS, params_json);

        // Settings
        if let Some(ref tz) = env.timezone {
            cmd.env(vars::TIMEZONE, tz);
        }
        cmd.env(vars::NIGHT_MODE, if env.night_mode { "1" } else { "0" });

        if let Some(ref loc) = env.localization {
            let loc_json = serde_json::to_string(loc).map_err(SpawnError::SerializeLocalization)?;
            cmd.env(vars::LOCALIZATION, loc_json);
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let child = cmd.spawn().map_err(SpawnError::SpawnProcess)?;

        tracing::info!(
            "widget process spawned: instance={} pid={:?}",
            env.instance_id,
            child.id()
        );

        Ok(child)
    }
}

impl Default for WaylandSpawner {
    fn default() -> Self {
        Self
    }
}

fn size_type_to_str(size: SizeType) -> &'static str {
    match size {
        SizeType::Small => "small",
        SizeType::Medium => "medium",
        SizeType::Large => "large",
        SizeType::Full => "full",
    }
}
