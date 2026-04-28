// Copyright (C) 2025  Braiins Systems s.r.o.

use std::process::Stdio;

use tokio::process::{Child, Command};

use super::WidgetInfo;
use super::coordinator::WidgetEnv;

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("failed to spawn process: {0}")]
    SpawnProcess(std::io::Error),
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
        let mut cmd = Command::new(&widget.binary_path);

        cmd.env("WAYLAND_DISPLAY", &env.wayland_display)
            .env("XDG_RUNTIME_DIR", xdg_runtime_dir);

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
