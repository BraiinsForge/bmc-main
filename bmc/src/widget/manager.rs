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
use std::time::{Duration, Instant};

use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};
use uuid::Uuid;

use super::coordinator::WidgetEnv;
use super::{SpawnError, WaylandSpawner, WidgetIdentity, WidgetInfo, WidgetRegistry};

const DEFAULT_XDG_RUNTIME_DIR: &str = "/tmp/run";

/// Grace period for widget processes to clean up after SIGTERM before
/// resorting to SIGKILL. Widgets that hold GPU resources (GEM/DMA-BUF)
/// need time to run destructors so the kernel can reclaim CMA memory.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// What the actor knows about an instance's process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildObservation {
    Running,
    /// Still supervised, but between processes: its respawn is pending.
    Exited,
    /// Not supervised at all: never spawned, stopped, or given up on.
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

/// Crash respawn backoff: the delay starts at the initial value,
/// doubles per crash up to the cap,
/// and resets once a process stays up for the healthy uptime.
const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(30);
const RESTART_HEALTHY_UPTIME: Duration = Duration::from_mins(1);

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
    /// A crashed widget was respawned; the consumer must bind the new pid
    /// for the reconnecting process to be recognized.
    Respawned { instance_id: String, pid: u32 },
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
        Self::with_parts(registry, Box::new(spawner))
    }

    fn with_parts(
        registry: Arc<WidgetRegistry>,
        spawner: Box<dyn WidgetSpawn>,
    ) -> (Self, mpsc::UnboundedReceiver<WidgetEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let actor = Actor {
            registry: registry.clone(),
            spawner,
            children: HashMap::new(),
            next_restart_token: 0,
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
    /// A crashed widget is respawned automatically with backoff until it is
    /// stopped or its type leaves the registry. Self-exits and respawns are
    /// reported on the stream returned by [`Self::init`].
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
    /// Also cancels a pending crash respawn of the instance.
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

/// Injected so supervision tests do not launch real widget binaries.
trait WidgetSpawn: Send + 'static {
    fn spawn(
        &self,
        widget: &WidgetInfo,
        env: &WidgetEnv,
        xdg_runtime_dir: &str,
    ) -> Result<Child, SpawnError>;
}

impl WidgetSpawn for WaylandSpawner {
    fn spawn(
        &self,
        widget: &WidgetInfo,
        env: &WidgetEnv,
        xdg_runtime_dir: &str,
    ) -> Result<Child, SpawnError> {
        WaylandSpawner::spawn(self, widget, env, xdg_runtime_dir)
    }
}

/// A widget instance as the actor tracks it:
/// either a live process, or a crashed one whose respawn timer runs.
/// An external stop removes the entry entirely,
/// which is what distinguishes "stopped" from "crashed".
enum WidgetState {
    Running(RunningWidget),
    PendingRestart(PendingRestart),
}

/// A live child. The `Child` itself is owned by the instance's
/// [`run_child`] task; the actor holds the channels to reach it.
struct RunningWidget {
    pid: u32,
    /// Requests a graceful stop;
    /// carries the reply sender acknowledged once the process is gone.
    stop_tx: oneshot::Sender<oneshot::Sender<()>>,
    /// Spawn parameters kept for a crash respawn.
    widget_uid: Uuid,
    env: WidgetEnv,
    spawned_at: Instant,
    /// Delay before the respawn if this process crashes.
    backoff: Duration,
}

/// A crashed instance waiting out its respawn delay.
struct PendingRestart {
    widget_uid: Uuid,
    env: WidgetEnv,
    /// Delay before the respawn after this one, should it crash again.
    backoff: Duration,
    /// Matches the queued [`Internal::RestartDue`] to this crash;
    /// a stop or an external re-spawn replaces the entry,
    /// orphaning the token and cancelling the respawn.
    token: u64,
}

enum Internal {
    /// A [`run_child`] task reports its process exited on its own
    /// (externally stopped children are acknowledged through the stop
    /// reply instead).
    Exit(ChildExit),
    /// A respawn timer elapsed.
    RestartDue { instance_id: String, token: u64 },
}

struct ChildExit {
    instance_id: String,
    pid: u32,
}

fn next_backoff(delay: Duration) -> Duration {
    (delay * 2).min(RESTART_BACKOFF_MAX)
}

/// The actor task: sole owner of the running widget set.
struct Actor {
    registry: Arc<WidgetRegistry>,
    spawner: Box<dyn WidgetSpawn>,
    children: HashMap<String, WidgetState>,
    next_restart_token: u64,
    events_tx: mpsc::UnboundedSender<WidgetEvent>,
}

impl Actor {
    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Command>) {
        let (internal_tx, mut internal_rx) = mpsc::unbounded_channel();
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(cmd) => self.handle_command(cmd, &internal_tx),
                    // All handles dropped; dropping the children map lets
                    // kill_on_drop reap any survivors.
                    None => break,
                },
                Some(msg) = internal_rx.recv() => match msg {
                    Internal::Exit(exit) => self.handle_exit(exit, &internal_tx),
                    Internal::RestartDue { instance_id, token } => {
                        self.handle_restart_due(&instance_id, token, &internal_tx);
                    }
                },
            }
        }
    }

    fn handle_command(&mut self, cmd: Command, internal_tx: &mpsc::UnboundedSender<Internal>) {
        match cmd {
            Command::Spawn {
                widget_uid,
                env,
                reply,
            } => {
                let _ = reply.send(self.spawn_process(
                    widget_uid,
                    env,
                    RESTART_BACKOFF_INITIAL,
                    internal_tx,
                ));
            }
            Command::Stop { instance_id, reply } => self.stop(&instance_id, reply),
            Command::StopAll { reply } => self.stop_all(reply),
            Command::Observe { instance_id, reply } => {
                let _ = reply.send(self.observe(&instance_id));
            }
        }
    }

    fn observe(&self, instance_id: &str) -> ChildObservation {
        match self.children.get(instance_id) {
            Some(WidgetState::Running(_)) => ChildObservation::Running,
            Some(WidgetState::PendingRestart(_)) => ChildObservation::Exited,
            None => ChildObservation::Missing,
        }
    }

    fn spawn_process(
        &mut self,
        widget_uid: Uuid,
        env: WidgetEnv,
        backoff: Duration,
        internal_tx: &mpsc::UnboundedSender<Internal>,
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

        let (stop_tx, stop_rx) = oneshot::channel();
        let instance_id = env.instance_id.clone();
        // A re-spawn of a live instance replaces its entry;
        // the replaced entry's run_child task drops its Child,
        // which kill_on_drop reaps.
        self.children.insert(
            instance_id.clone(),
            WidgetState::Running(RunningWidget {
                pid,
                stop_tx,
                widget_uid,
                env,
                spawned_at: Instant::now(),
                backoff,
            }),
        );
        tokio::spawn(run_child(
            instance_id.clone(),
            pid,
            child,
            stop_rx,
            internal_tx.clone(),
        ));

        info!("widget instance {} spawned (pid={})", instance_id, pid);

        Ok(SpawnedWidget {
            pid,
            identity: widget.identity,
        })
    }

    fn stop(&mut self, instance_id: &str, reply: oneshot::Sender<()>) {
        match self.children.remove(instance_id) {
            Some(WidgetState::Running(widget)) => {
                if let Err(reply) = widget.stop_tx.send(reply) {
                    // The child task is already gone (its exit notification
                    // may still be queued); nothing left to stop.
                    let _ = reply.send(());
                }
            }
            Some(WidgetState::PendingRestart(_)) => {
                // Removing the entry orphans the restart token,
                // cancelling the queued respawn.
                let _ = reply.send(());
            }
            None => {
                warn!("attempted to stop unknown widget instance {}", instance_id);
                let _ = reply.send(());
            }
        }
    }

    fn stop_all(&mut self, reply: oneshot::Sender<Vec<String>>) {
        let mut ids = Vec::with_capacity(self.children.len());
        let mut done_rxs = Vec::with_capacity(self.children.len());
        for (id, state) in self.children.drain() {
            match state {
                WidgetState::Running(widget) => {
                    let (done_tx, done_rx) = oneshot::channel();
                    if widget.stop_tx.send(done_tx).is_ok() {
                        done_rxs.push(done_rx);
                    }
                }
                WidgetState::PendingRestart(_) => {}
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

    /// Take the entry only if it is the live process `pid` belongs to.
    /// A stopped instance, a pending restart, or an already-replaced spawn
    /// leaves the map untouched — which is what makes a stale exit a no-op.
    fn take_running(&mut self, instance_id: &str, pid: u32) -> Option<RunningWidget> {
        match self.children.get(instance_id) {
            Some(WidgetState::Running(widget)) if widget.pid == pid => {}
            _ => return None,
        }
        match self.children.remove(instance_id) {
            Some(WidgetState::Running(widget)) => Some(widget),
            other => {
                debug_assert!(false, "BUG: entry changed between check and removal");
                other.map(|state| self.children.insert(instance_id.to_owned(), state));
                None
            }
        }
    }

    /// Take the entry only if it is still waiting out `token`'s respawn delay.
    /// A stop or an external re-spawn replaces the entry and orphans the token,
    /// so a mismatch means the timer lost its race and must do nothing.
    fn take_pending_restart(&mut self, instance_id: &str, token: u64) -> Option<PendingRestart> {
        match self.children.get(instance_id) {
            Some(WidgetState::PendingRestart(pending)) if pending.token == token => {}
            _ => return None,
        }
        match self.children.remove(instance_id) {
            Some(WidgetState::PendingRestart(pending)) => Some(pending),
            other => {
                debug_assert!(false, "BUG: entry changed between check and removal");
                other.map(|state| self.children.insert(instance_id.to_owned(), state));
                None
            }
        }
    }

    fn handle_exit(&mut self, exit: ChildExit, internal_tx: &mpsc::UnboundedSender<Internal>) {
        // Only a live entry with a matching pid counts as a crash;
        // anything else is a stale exit of an already replaced spawn.
        let Some(widget) = self.take_running(&exit.instance_id, exit.pid) else {
            return;
        };

        let _ = self.events_tx.send(WidgetEvent::Exited {
            instance_id: exit.instance_id.clone(),
            pid: exit.pid,
        });

        let delay = if widget.spawned_at.elapsed() >= RESTART_HEALTHY_UPTIME {
            RESTART_BACKOFF_INITIAL
        } else {
            widget.backoff
        };
        // Scheduling, not promising:
        // the respawn re-checks the registry when the timer fires,
        // and gives up if the widget type has been uninstalled.
        warn!(
            "widget {} (pid={}) died unexpectedly; scheduling a respawn in {:?}",
            exit.instance_id, exit.pid, delay
        );
        self.schedule_restart(
            exit.instance_id,
            widget.widget_uid,
            widget.env,
            delay,
            internal_tx,
        );
    }

    fn schedule_restart(
        &mut self,
        instance_id: String,
        widget_uid: Uuid,
        env: WidgetEnv,
        delay: Duration,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) {
        let token = self.next_restart_token;
        self.next_restart_token += 1;
        self.children.insert(
            instance_id.clone(),
            WidgetState::PendingRestart(PendingRestart {
                widget_uid,
                env,
                backoff: next_backoff(delay),
                token,
            }),
        );
        let internal_tx = internal_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = internal_tx.send(Internal::RestartDue { instance_id, token });
        });
    }

    fn handle_restart_due(
        &mut self,
        instance_id: &str,
        token: u64,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) {
        // A mismatched token means the instance was stopped or replaced
        // while the timer ran; the respawn is cancelled.
        let Some(pending) = self.take_pending_restart(instance_id, token) else {
            return;
        };

        if self.registry.get(&pending.widget_uid).is_none() {
            warn!(
                "widget type {} is no longer installed; not respawning instance {}",
                pending.widget_uid, instance_id
            );
            return;
        }

        match self.spawn_process(
            pending.widget_uid,
            pending.env.clone(),
            pending.backoff,
            internal_tx,
        ) {
            Ok(spawned) => {
                let _ = self.events_tx.send(WidgetEvent::Respawned {
                    instance_id: instance_id.to_owned(),
                    pid: spawned.pid,
                });
            }
            Err(e) => {
                warn!(
                    "failed to respawn widget {}: {}; retrying in {:?}",
                    instance_id, e, pending.backoff
                );
                self.schedule_restart(
                    instance_id.to_owned(),
                    pending.widget_uid,
                    pending.env,
                    pending.backoff,
                    internal_tx,
                );
            }
        }
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
    internal_tx: mpsc::UnboundedSender<Internal>,
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
            let _ = internal_tx.send(Internal::Exit(ChildExit { instance_id, pid }));
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
    use std::process::Stdio;

    use bmc_widget_manifest::{Manifest, ViewportShape, WidgetCategory, WidgetViewportConstraint};
    use tokio::time::timeout;

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

    async fn wait_for_pending_restart(manager: &WidgetManager, instance_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let observation = manager.observe_child(instance_id).await;
            if observation == ChildObservation::Exited {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "crashed child never reached a pending restart: {observation:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn child_observation_distinguishes_running_exited_and_missing() {
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
            "an external stop ends supervision, so nothing is tracked"
        );

        std::fs::remove_dir_all(&running_dir).expect("BUG: remove running widget");
        let exited_dir = temp.path().join("exited");
        write_widget(&exited_dir, "#!/bin/sh\nexit 0\n");
        let (exited_manager, _exited_events) =
            WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let _ = spawn(&exited_manager, "exited").await;
        wait_for_pending_restart(&exited_manager, "exited").await;
    }

    const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
    /// Longer than the initial respawn backoff,
    /// so silence proves a respawn was cancelled rather than still pending.
    const SILENCE_TIMEOUT: Duration = Duration::from_millis(1_500);

    fn test_widget_info(uid: Uuid) -> WidgetInfo {
        let manifest = Manifest {
            uid,
            version: semver::Version::new(1, 0, 0),
            name: "test-widget".to_owned(),
            subname: None,
            description: "Test widget".to_owned(),
            config_help: None,
            author: None,
            binary: PathBuf::from("bin/widget"),
            icon: None,
            category: WidgetCategory::Misc,
            settings: vec![],
            supported_viewports: vec![WidgetViewportConstraint {
                viewport_shape: ViewportShape::Rectangular,
                min_width: None,
                max_width: None,
                min_height: None,
                max_height: None,
                min_dpi: None,
                max_dpi: None,
            }],
            params: indexmap::IndexMap::new(),
            credentials: indexmap::IndexMap::new(),
        };

        WidgetInfo::for_test(
            manifest,
            PathBuf::from("/test/widgets/test-widget"),
            PathBuf::from("/test/widgets/test-widget/bin/widget"),
            None,
        )
    }

    /// Spawns a fixed command instead of the registry binary,
    /// so tests can simulate crashes and long-lived widgets.
    struct FakeSpawner {
        program: String,
        args: Vec<String>,
    }

    impl WidgetSpawn for FakeSpawner {
        fn spawn(
            &self,
            _widget: &WidgetInfo,
            _env: &WidgetEnv,
            _xdg_runtime_dir: &str,
        ) -> Result<Child, SpawnError> {
            let mut cmd = tokio::process::Command::new(&self.program);
            cmd.args(&self.args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            cmd.spawn().map_err(SpawnError::SpawnProcess)
        }
    }

    fn manager_with(
        uid: Uuid,
        program: &str,
        args: &[&str],
    ) -> (WidgetManager, mpsc::UnboundedReceiver<WidgetEvent>) {
        let registry = Arc::new(WidgetRegistry::new([test_widget_info(uid)]));
        WidgetManager::with_parts(
            registry,
            Box::new(FakeSpawner {
                program: program.to_owned(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            }),
        )
    }

    fn env(instance_id: &str) -> WidgetEnv {
        WidgetEnv {
            instance_id: instance_id.to_owned(),
            wayland_display: "wayland-test".to_owned(),
        }
    }

    async fn next_event(events: &mut mpsc::UnboundedReceiver<WidgetEvent>) -> WidgetEvent {
        timeout(EVENT_TIMEOUT, events.recv())
            .await
            .expect("timed out waiting for a widget event")
            .expect("BUG: widget event channel closed")
    }

    #[tokio::test]
    async fn spawn_unknown_widget_fails() {
        let (manager, _events) = WidgetManager::init(Vec::new(), false).await;
        let result = manager
            .spawn_widget(Uuid::new_v4(), env("test-instance"))
            .await;
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

    #[tokio::test]
    async fn crashed_widget_respawns_with_a_new_pid() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "true", &[]);
        let pid = manager
            .spawn_widget(uid, env("crashy"))
            .await
            .expect("BUG: test spawn failed")
            .pid;

        match next_event(&mut events).await {
            WidgetEvent::Exited {
                instance_id,
                pid: exited_pid,
            } => {
                assert_eq!(instance_id, "crashy");
                assert_eq!(exited_pid, pid, "the exit must report the crashed pid");
            }
            WidgetEvent::Respawned { .. } => panic!("a crash must be reported before the respawn"),
        }

        match next_event(&mut events).await {
            WidgetEvent::Respawned {
                instance_id,
                pid: new_pid,
            } => {
                assert_eq!(instance_id, "crashy");
                assert_ne!(new_pid, pid, "the respawn must carry the new pid");
            }
            WidgetEvent::Exited { .. } => panic!("expected a respawn after the crash"),
        }
    }

    #[tokio::test]
    async fn stopped_widget_is_not_respawned() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(uid, env("stoppable"))
            .await
            .expect("BUG: test spawn failed");

        manager.stop_widget("stoppable").await;

        let silence = timeout(SILENCE_TIMEOUT, events.recv()).await;
        assert!(
            silence.is_err(),
            "an externally stopped widget must emit no events, got {silence:?}"
        );
    }

    #[tokio::test]
    async fn stop_cancels_a_pending_respawn() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "true", &[]);
        manager
            .spawn_widget(uid, env("flappy"))
            .await
            .expect("BUG: test spawn failed");

        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Exited { .. }),
            "the crash must be reported first"
        );
        manager.stop_widget("flappy").await;

        let silence = timeout(SILENCE_TIMEOUT, events.recv()).await;
        assert!(
            silence.is_err(),
            "a stop during the respawn window must cancel the respawn, got {silence:?}"
        );
    }

    #[tokio::test]
    async fn stop_all_returns_every_instance_id() {
        let uid = Uuid::new_v4();
        let (manager, _events) = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(uid, env("one"))
            .await
            .expect("BUG: test spawn failed");
        manager
            .spawn_widget(uid, env("two"))
            .await
            .expect("BUG: test spawn failed");

        let mut ids = manager.stop_all().await;
        ids.sort();
        assert_eq!(ids, ["one", "two"]);
    }
}
