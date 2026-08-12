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
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};
use uuid::Uuid;

use super::coordinator::WidgetEnv;
use super::{SpawnError, WaylandSpawner, WidgetIdentity, WidgetRegistry};

const DEFAULT_XDG_RUNTIME_DIR: &str = "/tmp/run";

/// Grace period for widget processes to clean up after SIGTERM before
/// resorting to SIGKILL. Widgets that hold GPU resources (GEM/DMA-BUF)
/// need time to run destructors so the kernel can reclaim CMA memory.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// What the actor knows about an instance's process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildObservation {
    Running,
    /// Not tracked: never spawned, stopped, or exited on its own.
    Missing,
}

pub(crate) struct SpawnedWidget {
    pub pid: u32,
    pub identity: WidgetIdentity,
}

/// Each command is awaited for its reply, so the depth is based on the number
/// of concurrent callers (not message rate).
///
/// Per-scene loops are sequential and gRPC edits serialise on the config lock,
/// so the ceiling is one command per widget of the largest scene —
/// the 4x2 slot grid. Sixteen is comfortably clear of it.
const COMMAND_CHANNEL_CAPACITY: usize = 16;

/// Handle to the widget child actor.
/// Child process handles live in a dedicated task;
/// this handle only forwards commands to it,
/// so no caller can touch a `Child` or race another caller for one.
#[derive(Debug)]
pub struct WidgetManager {
    registry: Arc<WidgetRegistry>,
    cmd_tx: mpsc::Sender<Command>,
}

/// Widget lifecycle notification from the child actor.
#[derive(Debug)]
pub enum WidgetEvent {
    /// The process exited on its own; externally stopped widgets are not
    /// reported. The pid lets the consumer drop stale pid associations.
    Exited { instance_id: String, pid: u32 },
}

enum Command {
    Spawn {
        widget_uid: Uuid,
        env: WidgetEnv,
        reply: oneshot::Sender<SpawnReply>,
    },
    Stop {
        instance_id: String,
        reply: oneshot::Sender<()>,
    },
    StopAll {
        reply: oneshot::Sender<Vec<String>>,
    },
    Observe {
        instance_id: String,
        reply: oneshot::Sender<ChildObservation>,
    },
}

type SpawnReply = Result<SpawnedWidget, SpawnError>;

impl WidgetManager {
    /// Returns the handle together with its lifecycle event stream.
    /// Handing the stream out here, rather than fetching it later,
    /// makes it impossible for a caller to forget: the events must be consumed
    /// for the compositor's pid bookkeeping to stay in step with the processes.
    pub async fn init(
        widgets_paths: Vec<PathBuf>,
        capture_widget_output: bool,
    ) -> (Self, mpsc::UnboundedReceiver<WidgetEvent>) {
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

        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let actor = Actor {
            registry: registry.clone(),
            spawner,
            children: HashMap::new(),
            events_tx,
        };
        tokio::spawn(actor.run(cmd_rx));

        (Self { registry, cmd_tx }, events_rx)
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
    /// A self-exit of the process is reported as [`WidgetEvent::Exited`]
    /// on the stream returned by [`Self::init`].
    pub(crate) async fn spawn_widget(&self, widget_uid: Uuid, env: WidgetEnv) -> SpawnReply {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Spawn {
                widget_uid,
                env,
                reply: reply_tx,
            })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a spawn reply")
    }

    /// Stop a widget process, returning once it has exited.
    pub async fn stop_widget(&self, instance_id: &str) {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Stop {
                instance_id: instance_id.to_owned(),
                reply: reply_tx,
            })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a stop reply");
    }

    /// Stop all widget processes and return the instance ids that were
    /// stopped, so callers can run per-instance cleanup for each of them.
    pub async fn stop_all(&self) -> Vec<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::StopAll { reply: reply_tx })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a stop-all reply")
    }

    /// Report whether the actor still tracks a live process for the instance,
    /// so callers that replace a widget can skip the ones it would not replace.
    pub(crate) async fn observe_child(&self, instance_id: &str) -> ChildObservation {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Observe {
                instance_id: instance_id.to_owned(),
                reply: reply_tx,
            })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped an observe reply")
    }
}

/// A live child as the actor tracks it.
/// The `Child` itself is owned by the instance's [`run_child`] task;
/// the actor holds the channels to reach it.
struct RunningWidget {
    pid: u32,
    /// Requests a graceful stop;
    /// carries the reply sender acknowledged once the process is gone.
    stop_tx: oneshot::Sender<oneshot::Sender<()>>,
}

/// Notification from a [`run_child`] task that its process exited on its own
/// (externally stopped children are acknowledged through the stop reply instead).
struct ChildExit {
    instance_id: String,
    pid: u32,
}

/// The actor task: sole owner of the running widget set.
struct Actor {
    registry: Arc<WidgetRegistry>,
    spawner: WaylandSpawner,
    children: HashMap<String, RunningWidget>,
    events_tx: mpsc::UnboundedSender<WidgetEvent>,
}

impl Actor {
    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Command>) {
        let (exit_tx, mut exit_rx) = mpsc::unbounded_channel();
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(cmd) => self.handle_command(cmd, &exit_tx),
                    // All handles dropped; dropping the children map lets
                    // kill_on_drop reap any survivors.
                    None => break,
                },
                Some(exit) = exit_rx.recv() => self.handle_exit(exit),
            }
        }
    }

    fn handle_command(&mut self, cmd: Command, exit_tx: &mpsc::UnboundedSender<ChildExit>) {
        match cmd {
            Command::Spawn {
                widget_uid,
                env,
                reply,
            } => {
                let _ = reply.send(self.spawn(widget_uid, &env, exit_tx));
            }
            Command::Stop { instance_id, reply } => self.stop(&instance_id, reply),
            Command::StopAll { reply } => self.stop_all(reply),
            Command::Observe { instance_id, reply } => {
                let _ = reply.send(self.observe(&instance_id));
            }
        }
    }

    /// An entry exists only while the instance is supposed to have a process,
    /// so its absence is what "not tracked" means.
    fn observe(&self, instance_id: &str) -> ChildObservation {
        if self.children.contains_key(instance_id) {
            ChildObservation::Running
        } else {
            ChildObservation::Missing
        }
    }

    fn spawn(
        &mut self,
        widget_uid: Uuid,
        env: &WidgetEnv,
        exit_tx: &mpsc::UnboundedSender<ChildExit>,
    ) -> SpawnReply {
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

        let mut child = self.spawner.spawn(&widget, env, &xdg_runtime_dir)?;
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

        let (stop_tx, stop_rx) = oneshot::channel();
        // A re-spawn of a live instance replaces its entry;
        // the replaced entry's run_child task drops its Child,
        // which kill_on_drop reaps.
        self.children
            .insert(env.instance_id.clone(), RunningWidget { pid, stop_tx });
        tokio::spawn(run_child(
            env.instance_id.clone(),
            pid,
            child,
            stop_rx,
            exit_tx.clone(),
        ));

        info!("widget instance {} spawned (pid={})", env.instance_id, pid);

        Ok(SpawnedWidget {
            pid,
            identity: widget.identity,
        })
    }

    fn stop(&mut self, instance_id: &str, reply: oneshot::Sender<()>) {
        let Some(widget) = self.children.remove(instance_id) else {
            warn!("attempted to stop unknown widget instance {}", instance_id);
            let _ = reply.send(());
            return;
        };
        if let Err(reply) = widget.stop_tx.send(reply) {
            // The child task is already gone (its exit notification may
            // still be queued); the process no longer needs stopping.
            let _ = reply.send(());
        }
    }

    fn stop_all(&mut self, reply: oneshot::Sender<Vec<String>>) {
        let mut ids = Vec::with_capacity(self.children.len());
        let mut done_rxs = Vec::with_capacity(self.children.len());
        for (id, widget) in self.children.drain() {
            let (done_tx, done_rx) = oneshot::channel();
            if widget.stop_tx.send(done_tx).is_ok() {
                done_rxs.push(done_rx);
            }
            ids.push(id);
        }
        // Await the acknowledgements outside the actor so a slow widget
        // (up to the SIGKILL timeout) cannot stall other commands.
        tokio::spawn(async move {
            futures::future::join_all(done_rxs).await;
            info!("all widgets stopped");
            let _ = reply.send(ids);
        });
    }

    fn handle_exit(&mut self, exit: ChildExit) {
        let Some(widget) = self.children.get(&exit.instance_id) else {
            return;
        };
        if widget.pid != exit.pid {
            // The exit belongs to an older spawn of this instance,
            // already replaced by a newer one.
            return;
        }
        self.children.remove(&exit.instance_id);
        let _ = self.events_tx.send(WidgetEvent::Exited {
            instance_id: exit.instance_id,
            pid: exit.pid,
        });
    }
}

/// Own a child process until it ends:
/// either it exits on its own (reported to the actor),
/// or the actor requests a stop (SIGTERM → SIGKILL, then acknowledged).
/// If the actor drops the stop channel instead,
/// dropping the `Child` reaps the process via kill_on_drop.
async fn run_child(
    instance_id: String,
    pid: u32,
    mut child: Child,
    mut stop_rx: oneshot::Receiver<oneshot::Sender<()>>,
    exit_tx: mpsc::UnboundedSender<ChildExit>,
) {
    let mut stop_reply = None;
    let wait_result: Option<std::io::Result<ExitStatus>> = tokio::select! {
        status = child.wait() => Some(status),
        stop = &mut stop_rx => {
            stop_reply = stop.ok();
            None
        }
    };

    match wait_result {
        Some(status) => {
            match status {
                Ok(status) => info!("widget {} (pid={}) exited: {}", instance_id, pid, status),
                Err(e) => warn!(
                    "failed to wait for widget {} (pid={}): {}",
                    instance_id, pid, e
                ),
            }
            let _ = exit_tx.send(ChildExit { instance_id, pid });
        }
        None => {
            if let Some(reply) = stop_reply {
                graceful_stop(instance_id, child).await;
                let _ = reply.send(());
            }
        }
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

    async fn wait_until_untracked(manager: &WidgetManager, instance_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let observation = manager.observe_child(instance_id).await;
            if observation == ChildObservation::Missing {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "child was still tracked at the deadline: {observation:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A self-exit and an external stop are both `Missing`: the actor drops the
    /// entry either way, so tracking is what the observation reports.
    #[tokio::test]
    async fn child_observation_distinguishes_running_from_untracked() {
        let temp = tempfile::TempDir::new().expect("BUG: tempdir");
        let running_dir = temp.path().join("running");
        write_widget(&running_dir, "#!/bin/sh\nwhile :; do sleep 1; done\n");
        let (running_manager, _events) = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let spawned = spawn(&running_manager, "running").await;
        assert_eq!(spawned.identity.canonical_dir, running_dir);
        assert_eq!(
            running_manager.observe_child("running").await,
            ChildObservation::Running
        );
        running_manager.stop_widget("running").await;
        assert_eq!(
            running_manager.observe_child("running").await,
            ChildObservation::Missing,
            "a stopped instance is no longer tracked"
        );

        std::fs::remove_dir_all(&running_dir).expect("BUG: remove running widget");
        let exited_dir = temp.path().join("exited");
        write_widget(&exited_dir, "#!/bin/sh\nexit 0\n");
        let (exited_manager, _exited_events) =
            WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let _ = spawn(&exited_manager, "exited").await;
        wait_until_untracked(&exited_manager, "exited").await;
    }

    #[tokio::test]
    async fn spawn_unknown_widget_fails() {
        let (manager, _events) = WidgetManager::init(Vec::new(), false).await;
        let env = WidgetEnv {
            instance_id: "test-instance".to_owned(),
            wayland_display: "wayland-test".to_owned(),
        };
        let result = manager.spawn_widget(Uuid::new_v4(), env).await;
        assert!(result.is_err(), "spawn must fail for an unknown widget");
    }

    #[tokio::test]
    async fn stop_unknown_widget_returns() {
        let (manager, _events) = WidgetManager::init(Vec::new(), false).await;
        manager.stop_widget("no-such-instance").await;
    }

    #[tokio::test]
    async fn stop_all_without_widgets_returns_no_ids() {
        let (manager, _events) = WidgetManager::init(Vec::new(), false).await;
        assert!(manager.stop_all().await.is_empty());
    }
}
