// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::process::Stdio;

use tokio::process::{Child, Command};
use uuid::Uuid;

use super::WidgetEnv;
use super::WidgetInfo;

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
        widget_key: Uuid,
        env: &WidgetEnv,
        xdg_runtime_dir: &str,
    ) -> Result<Child, SpawnError> {
        let mut cmd = Command::new(&widget.binary_path);

        cmd.env("WAYLAND_DISPLAY", &env.wayland_display)
            .env("BMC_WIDGET_KEY", widget_key.to_string())
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
            widget_key,
            child.id()
        );

        Ok(child)
    }
}
