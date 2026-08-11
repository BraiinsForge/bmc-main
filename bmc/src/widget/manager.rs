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

use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Child;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use super::coordinator::WidgetEnv;
use super::{SpawnError, WaylandSpawner, WidgetIdentity, WidgetRegistry};

const DEFAULT_XDG_RUNTIME_DIR: &str = "/tmp/run";

/// Grace period for widget processes to clean up after SIGTERM before
/// resorting to SIGKILL. Widgets that hold GPU resources (GEM/DMA-BUF)
/// need time to run destructors so the kernel can reclaim CMA memory.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildObservation {
    Running,
    Exited,
    Missing,
}

pub(crate) struct SpawnedWidget {
    pub pid: u32,
    pub exit_rx: tokio::sync::oneshot::Receiver<u32>,
    pub identity: WidgetIdentity,
}

#[derive(Debug)]
pub struct WidgetManager {
    registry: Arc<WidgetRegistry>,
    spawner: WaylandSpawner,
    children: Arc<RwLock<HashMap<String, Child>>>,
}

impl WidgetManager {
    pub async fn init(widgets_paths: Vec<PathBuf>, capture_widget_output: bool) -> Self {
        info!("initializing widget manager");
        for path in &widgets_paths {
            info!(path = %path.display(), "scanning widget directory");
        }

        let registry = Arc::new(WidgetRegistry::discover(widgets_paths).await);
        info!(count = registry.len(), "widget discovery complete");

        for widget in registry.list() {
            info!(
                name = %widget.manifest.name,
                version = %widget.manifest.version,
                uid = %widget.manifest.uid,
                "registered widget"
            );
        }

        let spawner = WaylandSpawner::new(capture_widget_output);

        Self {
            registry,
            spawner,
            children: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get a shared reference to the widget registry.
    #[must_use]
    pub fn registry(&self) -> Arc<WidgetRegistry> {
        self.registry.clone()
    }

    /// Re-scan the widget discovery paths so newly-installed widgets become
    /// available without a restart. A no-op if the registry is static.
    pub async fn refresh(&self) -> Result<(), super::RegistryError> {
        self.registry.refresh().await
    }

    /// Spawn a widget process and return its OS pid. The compositor needs
    /// the pid to correlate the eventual Wayland connection back to the
    /// widget's registered instance id.
    ///
    /// Also returns a oneshot receiver that fires with the pid when the
    /// child process exits, so the caller can clear the pid from the
    /// compositor and prevent stale pid recycling.
    pub(crate) async fn spawn_widget(
        &self,
        widget_uid: Uuid,
        env: WidgetEnv,
    ) -> Result<SpawnedWidget, SpawnError> {
        let widget = self.registry.get(&widget_uid).ok_or_else(|| {
            SpawnError::SpawnProcess(Error::new(
                ErrorKind::NotFound,
                format!("widget not found: {widget_uid}"),
            ))
        })?;

        info!(
            "spawning widget '{}' instance {}",
            widget.manifest.name, env.instance_id
        );

        let xdg_runtime_dir =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| DEFAULT_XDG_RUNTIME_DIR.to_owned());

        let mut child = self.spawner.spawn(&widget, &env, &xdg_runtime_dir)?;
        let pid = child.id().ok_or_else(|| {
            SpawnError::SpawnProcess(Error::other("spawned child has no pid (already exited?)"))
        })?;

        let short_instance_id = env
            .instance_id
            .get(..8)
            .unwrap_or(&env.instance_id)
            .to_owned();
        if let Some(stdout) = child.stdout.take() {
            let name = widget.manifest.name.clone();
            let instance = short_instance_id.clone();
            tokio::spawn(async move {
                super::capture::forward_widget_output(stdout, &name, &instance, pid).await;
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let name = widget.manifest.name.clone();
            let instance = short_instance_id;
            tokio::spawn(async move {
                super::capture::forward_widget_output(stderr, &name, &instance, pid).await;
            });
        }

        self.children
            .write()
            .await
            .insert(env.instance_id.clone(), child);

        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
        let children = Arc::clone(&self.children);
        let instance_id = env.instance_id.clone();
        tokio::spawn(async move {
            let mut exited = false;
            loop {
                let mut guard = children.write().await;
                if let Some(child) = guard.get_mut(&instance_id) {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            info!("widget {} (pid={}) exited: {}", instance_id, pid, status);
                            exited = true;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!("failed to poll widget {} (pid={}): {}", instance_id, pid, e);
                            exited = true;
                        }
                    }
                } else {
                    break;
                }
                drop(guard);
                if exited {
                    let _ = exit_tx.send(pid);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        info!("widget instance {} spawned (pid={})", env.instance_id, pid);

        Ok(SpawnedWidget {
            pid,
            exit_rx,
            identity: widget.identity,
        })
    }

    pub(crate) async fn observe_child(&self, instance_id: &str) -> Result<ChildObservation, Error> {
        let mut children = self.children.write().await;
        let Some(child) = children.get_mut(instance_id) else {
            return Ok(ChildObservation::Missing);
        };
        child.try_wait().map(|status| match status {
            Some(_) => ChildObservation::Exited,
            None => ChildObservation::Running,
        })
    }

    pub async fn stop_widget(&self, instance_id: &str) {
        let mut children = self.children.write().await;
        if let Some(child) = children.remove(instance_id) {
            graceful_stop(instance_id.to_owned(), child).await;
        } else {
            warn!("attempted to stop unknown widget instance {}", instance_id);
        }
    }

    /// Stop all widget processes and return the instance ids that were
    /// stopped, so callers can run per-instance cleanup for each of them.
    pub async fn stop_all(&self) -> Vec<String> {
        let mut children = self.children.write().await;
        let (ids, futures): (Vec<_>, Vec<_>) = children
            .drain()
            .map(|(id, child)| (id.clone(), graceful_stop(id, child)))
            .unzip();
        futures::future::join_all(futures).await;
        info!("all widgets stopped");
        ids
    }
}

/// Send SIGTERM and wait for graceful exit; fall back to SIGKILL after
/// [`GRACEFUL_SHUTDOWN_TIMEOUT`]. This gives widget processes time to
/// run destructors and free GPU resources (GEM handles, DMA-BUFs)
/// that would otherwise leak CMA memory.
async fn graceful_stop(instance_id: String, mut child: Child) {
    let Some(pid) = child.id() else {
        info!("widget {instance_id} already exited");
        return;
    };

    // SAFETY: pid is a valid child process id obtained from `child.id()`.
    let term_result = unsafe { libc::kill(pid.cast_signed(), libc::SIGTERM) };
    if term_result != 0 {
        warn!("failed to send SIGTERM to widget {instance_id} (pid {pid})");
        let _ = child.kill().await;
        return;
    }

    info!("sent SIGTERM to widget {instance_id} (pid {pid}), waiting for exit");
    match tokio::time::timeout(GRACEFUL_SHUTDOWN_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => {
            info!("widget {instance_id} exited: {status}");
        }
        Ok(Err(e)) => {
            warn!("error waiting for widget {instance_id}: {e}");
        }
        Err(_) => {
            warn!(
                "widget {instance_id} did not exit within {}s, sending SIGKILL",
                GRACEFUL_SHUTDOWN_TIMEOUT.as_secs()
            );
            if let Err(e) = child.kill().await {
                warn!("failed to SIGKILL widget {instance_id}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    fn write_widget(root: &std::path::Path, body: &str) {
        std::fs::create_dir_all(root).expect("BUG: create widget root");
        std::fs::write(
            root.join("manifest.json"),
            r#"{"uid":"550e8400-e29b-41d4-a716-446655440000","version":"1.0.0","name":"manager-test","description":"manager test","binary":"widget","supported_viewports":[{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}]}"#,
        )
        .expect("BUG: write manifest");
        let binary = root.join("widget");
        std::fs::write(&binary, body).expect("BUG: write widget binary");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: chmod widget binary");
    }

    async fn spawn(manager: &WidgetManager, instance_id: &str) -> SpawnedWidget {
        manager
            .spawn_widget(
                Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: widget uid"),
                WidgetEnv {
                    instance_id: instance_id.to_owned(),
                    wayland_display: "wayland-test".to_owned(),
                },
            )
            .await
            .expect("BUG: spawn widget")
    }

    async fn wait_for_child_exit(manager: &WidgetManager, instance_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let observation = manager
                .observe_child(instance_id)
                .await
                .expect("BUG: observe child");
            if observation == ChildObservation::Exited {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "child did not exit before deadline: {observation:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn child_observation_distinguishes_running_exited_and_missing() {
        let temp = tempfile::TempDir::new().expect("BUG: tempdir");
        let running_dir = temp.path().join("running");
        write_widget(&running_dir, "#!/bin/sh\nwhile :; do sleep 1; done\n");
        let running_manager = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let spawned = spawn(&running_manager, "running").await;
        assert_eq!(spawned.identity.canonical_dir, running_dir);
        assert_eq!(
            running_manager
                .observe_child("running")
                .await
                .expect("observe"),
            ChildObservation::Running
        );
        running_manager.stop_widget("running").await;
        assert_eq!(
            running_manager
                .observe_child("running")
                .await
                .expect("observe"),
            ChildObservation::Missing
        );

        std::fs::remove_dir_all(&running_dir).expect("BUG: remove running widget");
        let exited_dir = temp.path().join("exited");
        write_widget(&exited_dir, "#!/bin/sh\nexit 0\n");
        let exited_manager = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let _ = spawn(&exited_manager, "exited").await;
        wait_for_child_exit(&exited_manager, "exited").await;
        exited_manager.stop_widget("exited").await;
    }
}
