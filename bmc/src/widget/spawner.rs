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
pub struct WaylandSpawner {
    capture_output: bool,
}

impl WaylandSpawner {
    #[must_use]
    pub fn new(capture_output: bool) -> Self {
        Self { capture_output }
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

        let (stdout, stderr) = if self.capture_output {
            (Stdio::piped(), Stdio::piped())
        } else {
            (Stdio::inherit(), Stdio::inherit())
        };
        cmd.stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
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
