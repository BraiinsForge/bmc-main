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

const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
/// A ceiling, never a give-up.
/// A crashed `bmc-wasm-host` drops every thin's control socket at once,
/// so one host fault exits the whole wasm fleet together —
/// a restart budget would blank the device for a fault no widget caused.
const RESTART_BACKOFF_MAX: Duration = Duration::from_mins(5);
/// Clears the thin's own startup budget:
/// `DEFAULT_HOST_WAIT` + `DEFAULT_ACK_WAIT` (10 s + 10 s) is how long a widget
/// that never reaches its host can burn before failing,
/// and one that never loaded must not read as healthy.
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
    /// The instance stays registered for the respawn.
    Exited { instance_id: String, pid: u32 },
    /// A crashed widget was respawned; the consumer must bind the new pid
    /// for the reconnecting process to be recognized.
    Respawned { instance_id: String, pid: u32 },
    /// Supervision gave up on the instance; nothing will bind a pid to it again,
    /// so the consumer must end the registration
    /// that [`Self::Exited`] deliberately left standing.
    Abandoned { instance_id: String },
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
    /// The registry was re-scanned, so pending respawns should stop waiting
    /// out delays they earned against binaries that may no longer be installed.
    RegistryRefreshed {
        reply: oneshot::Sender<()>,
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
        Self::with_parts(registry, Box::new(spawner), RestartPolicy::default())
    }

    fn with_parts(
        registry: Arc<WidgetRegistry>,
        spawner: Box<dyn WidgetSpawn>,
        policy: RestartPolicy,
    ) -> (Self, mpsc::UnboundedReceiver<WidgetEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let actor = Actor {
            registry: registry.clone(),
            spawner,
            children: HashMap::new(),
            next_restart_token: 0,
            events_tx,
            policy,
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
    /// available without a restart. The re-scan itself is a no-op for a static
    /// registry; what follows happens either way.
    ///
    /// Also brings any pending crash respawn forward to the initial delay,
    /// so a widget that crash-looped while its package was being replaced
    /// returns on the new binary instead of waiting out a delay
    /// it earned against the old one.
    /// A failed re-scan leaves the delays alone: nothing changed.
    pub async fn refresh(&self) -> Result<(), super::RegistryError> {
        self.registry.refresh().await?;

        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::RegistryRefreshed { reply: reply_tx })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a registry-refresh reply");
        Ok(())
    }

    /// Spawn a widget process and return its OS pid. The compositor needs it
    /// to correlate the eventual Wayland connection back to the widget's
    /// registered instance id.
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
        // A dropped reply says the same thing as a sent one: the process is gone.
        // The child can exit between the actor queueing the acknowledger
        // and `run_child` selecting on it.
        let _ = reply_rx.await;
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

    /// Report what the actor tracks for the instance.
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
    /// `env.wayland_display` is the one value a respawn cannot re-derive,
    /// since the manager holds no compositor reference by design,
    /// so it stays valid only while the compositor outlives every widget
    /// it spawned. A compositor restart would rebind a fresh socket
    /// (`ListeningSocket::bind_auto`, so not necessarily the same name)
    /// and strand every cached env here.
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
    /// Reported with the crash so the cause survives warn-level log filtering,
    /// where [`run_child`]'s info line is dropped.
    /// `None` when the wait itself failed and the cause is unknowable.
    status: Option<ExitStatus>,
}

/// The respawn ladder. Injected so tests can walk it in milliseconds.
#[derive(Debug, Clone, Copy)]
struct RestartPolicy {
    initial: Duration,
    max: Duration,
    healthy_uptime: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial: RESTART_BACKOFF_INITIAL,
            max: RESTART_BACKOFF_MAX,
            healthy_uptime: RESTART_HEALTHY_UPTIME,
        }
    }
}

fn next_backoff(delay: Duration, max: Duration) -> Duration {
    (delay * 2).min(max)
}

/// A process that outlived [`RestartPolicy::healthy_uptime`] started successfully,
/// so its failure begins a new ladder rather than continuing the old one.
/// An absolute threshold, deliberately, and not a fraction of `backoff`:
/// a widget dying just past it is up ~98% of the time,
/// and escalating would trade an unnoticeable blink for a mostly-blank cell.
fn restart_delay(uptime: Duration, backoff: Duration, policy: &RestartPolicy) -> Duration {
    if uptime >= policy.healthy_uptime {
        policy.initial
    } else {
        backoff
    }
}

/// The actor task: sole owner of the running widget set.
struct Actor {
    registry: Arc<WidgetRegistry>,
    spawner: Box<dyn WidgetSpawn>,
    children: HashMap<String, WidgetState>,
    next_restart_token: u64,
    events_tx: mpsc::UnboundedSender<WidgetEvent>,
    policy: RestartPolicy,
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
                let initial = self.policy.initial;
                let _ = reply.send(self.spawn_process(widget_uid, env, initial, internal_tx));
            }
            Command::Stop { instance_id, reply } => self.stop(&instance_id, reply),
            Command::StopAll { reply } => self.stop_all(reply),
            Command::Observe { instance_id, reply } => {
                let _ = reply.send(self.observe(&instance_id));
            }
            Command::RegistryRefreshed { reply } => {
                self.reset_restart_backoff(internal_tx);
                let _ = reply.send(());
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
        // Re-spawning over a live instance must not leave the old process
        // to kill_on_drop: SIGKILL skips the destructors that free GEM/DMA-BUF,
        // leaking CMA.
        // Dropping the acknowledger keeps the actor unblocked
        // while the old run_child task runs the full SIGTERM -> SIGKILL stop.
        // Replacing a PendingRestart entry needs none of this —
        // orphaning its token is how an external re-spawn cancels a queued one.
        if let Some(WidgetState::Running(previous)) = self.children.remove(&instance_id) {
            warn!(
                "re-spawning live widget {} (pid={}); stopping it first",
                instance_id, previous.pid
            );
            let (done_tx, _done_rx) = oneshot::channel();
            let _ = previous.stop_tx.send(done_tx);
        }
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
        match self.children.remove(instance_id)? {
            WidgetState::Running(widget) if widget.pid == pid => Some(widget),
            other @ (WidgetState::Running(_) | WidgetState::PendingRestart(_)) => {
                self.children.insert(instance_id.to_owned(), other);
                None
            }
        }
    }

    /// Take the entry only if it is still waiting out `token`'s respawn delay.
    /// A stop or an external re-spawn replaces the entry and orphans the token,
    /// so a mismatch means the timer lost its race and must do nothing.
    fn take_pending_restart(&mut self, instance_id: &str, token: u64) -> Option<PendingRestart> {
        match self.children.remove(instance_id)? {
            WidgetState::PendingRestart(pending) if pending.token == token => Some(pending),
            other @ (WidgetState::Running(_) | WidgetState::PendingRestart(_)) => {
                self.children.insert(instance_id.to_owned(), other);
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

        let delay = restart_delay(widget.spawned_at.elapsed(), widget.backoff, &self.policy);
        let cause = exit
            .status
            .map_or_else(|| "wait failed".to_owned(), |status| status.to_string());
        // Scheduling, not promising:
        // the respawn re-checks the registry when the timer fires,
        // and gives up if the widget type has been uninstalled.
        warn!(
            "widget {} (pid={}) died unexpectedly ({cause}); scheduling a respawn in {:?}",
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
                backoff: next_backoff(delay, self.policy.max),
                token,
            }),
        );
        let internal_tx = internal_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = internal_tx.send(Internal::RestartDue { instance_id, token });
        });
    }

    /// Bring every pending respawn forward to the initial delay.
    ///
    /// A registry change means the binaries on disk are no longer the ones
    /// the climbed delays were earned against,
    /// so holding a widget off any longer punishes it for a version that is gone.
    /// Same reasoning as [`RestartPolicy::healthy_uptime`],
    /// which resets the ladder for a process that proved itself.
    fn reset_restart_backoff(&mut self, internal_tx: &mpsc::UnboundedSender<Internal>) {
        let pending: Vec<_> = self
            .children
            .iter()
            .filter_map(|(instance_id, state)| match state {
                WidgetState::PendingRestart(pending) => {
                    Some((instance_id.clone(), pending.widget_uid, pending.env.clone()))
                }
                WidgetState::Running(_) => None,
            })
            .collect();

        if pending.is_empty() {
            return;
        }
        info!(
            "widget registry changed; retrying {} pending respawn(s) in {:?}",
            pending.len(),
            self.policy.initial
        );
        for (instance_id, widget_uid, env) in pending {
            self.schedule_restart(
                instance_id,
                widget_uid,
                env,
                self.policy.initial,
                internal_tx,
            );
        }
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
            let _ = self.events_tx.send(WidgetEvent::Abandoned {
                instance_id: instance_id.to_owned(),
            });
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
        Some(result) => {
            let status = match result {
                Ok(status) => {
                    info!("widget {} (pid={}) exited: {}", instance_id, pid, status);
                    Some(status)
                }
                Err(e) => {
                    warn!(
                        "failed to wait for widget {} (pid={}): {}",
                        instance_id, pid, e
                    );
                    None
                }
            };
            let _ = internal_tx.send(Internal::Exit(ChildExit {
                instance_id,
                pid,
                status,
            }));
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
        manager_with_policy(uid, program, args, RestartPolicy::default())
    }

    fn manager_with_policy(
        uid: Uuid,
        program: &str,
        args: &[&str],
        policy: RestartPolicy,
    ) -> (WidgetManager, mpsc::UnboundedReceiver<WidgetEvent>) {
        let registry = Arc::new(WidgetRegistry::new([test_widget_info(uid)]));
        WidgetManager::with_parts(
            registry,
            Box::new(FakeSpawner {
                program: program.to_owned(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            }),
            policy,
        )
    }

    /// A ladder short enough to walk several rungs inside a test.
    fn fast_policy() -> RestartPolicy {
        RestartPolicy {
            initial: Duration::from_millis(10),
            max: Duration::from_millis(40),
            healthy_uptime: Duration::from_hours(1),
        }
    }

    fn env(instance_id: &str) -> WidgetEnv {
        WidgetEnv {
            instance_id: instance_id.to_owned(),
            wayland_display: "wayland-test".to_owned(),
        }
    }

    /// Wait for a marker file the fake widget writes, returning whether it
    /// appeared before [`EVENT_TIMEOUT`].
    async fn await_file(path: &str) -> bool {
        let path = std::path::Path::new(path);
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while !path.exists() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        path.exists()
    }

    async fn next_event(events: &mut mpsc::UnboundedReceiver<WidgetEvent>) -> WidgetEvent {
        timeout(EVENT_TIMEOUT, events.recv())
            .await
            .expect("timed out waiting for a widget event")
            .expect("BUG: widget event channel closed")
    }

    #[test]
    fn backoff_doubles_up_to_the_ceiling() {
        let max = Duration::from_mins(5);
        assert_eq!(
            next_backoff(Duration::from_secs(1), max),
            Duration::from_secs(2)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(128), max),
            Duration::from_secs(256)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(256), max),
            max,
            "doubling past the ceiling clamps to it"
        );
        assert_eq!(next_backoff(max, max), max, "the ceiling is a fixed point");
    }

    #[test]
    fn healthy_uptime_restarts_the_ladder() {
        let policy = RestartPolicy::default();
        let climbed = Duration::from_secs(256);

        assert_eq!(
            restart_delay(policy.healthy_uptime, climbed, &policy),
            policy.initial,
            "reaching the healthy uptime exactly restarts the ladder"
        );
        assert_eq!(
            restart_delay(Duration::ZERO, climbed, &policy),
            climbed,
            "a process that died on startup continues the ladder"
        );
    }

    /// The crash-loop case the ceiling exists for:
    /// supervision must keep retrying however long it lasts,
    /// since a host fault exits every wasm widget at once
    /// and no restart budget could tell that apart.
    #[tokio::test]
    async fn a_repeat_crasher_keeps_being_respawned() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with_policy(uid, "true", &[], fast_policy());
        manager
            .spawn_widget(uid, env("flapping"))
            .await
            .expect("BUG: test spawn failed");

        for cycle in 0..4 {
            assert!(
                matches!(next_event(&mut events).await, WidgetEvent::Exited { .. }),
                "cycle {cycle} must report the crash"
            );
            assert!(
                matches!(next_event(&mut events).await, WidgetEvent::Respawned { .. }),
                "cycle {cycle} must respawn — the ladder caps the delay, it never gives up"
            );
        }
    }

    /// A package replacement leaves affected widgets crash-looping
    /// while their files are swapped,
    /// and the reload that follows hands every non-running instance
    /// to supervision rather than replacing it.
    /// None of them may then serve out a delay
    /// earned against a binary that is no longer installed.
    #[tokio::test]
    async fn a_registry_refresh_retries_pending_respawns_promptly() {
        /// Far enough above the assertion window
        /// that a prompt respawn cannot be the climbed timer simply elapsing.
        const CLIMBED: Duration = Duration::from_secs(1);
        const PROMPT: Duration = Duration::from_millis(400);

        let uid = Uuid::new_v4();
        let policy = RestartPolicy {
            initial: Duration::from_millis(1),
            max: Duration::from_secs(30),
            healthy_uptime: Duration::from_hours(1),
        };
        let (manager, mut events) = manager_with_policy(uid, "true", &[], policy);
        manager
            .spawn_widget(uid, env("flapping"))
            .await
            .expect("BUG: test spawn failed");

        let mut delay = policy.initial;
        while delay < CLIMBED {
            assert!(matches!(
                next_event(&mut events).await,
                WidgetEvent::Exited { .. }
            ));
            assert!(matches!(
                next_event(&mut events).await,
                WidgetEvent::Respawned { .. }
            ));
            delay = next_backoff(delay, policy.max);
        }
        // This crash leaves a wait of at least CLIMBED pending.
        assert!(matches!(
            next_event(&mut events).await,
            WidgetEvent::Exited { .. }
        ));

        manager
            .refresh()
            .await
            .expect("BUG: static registry refresh failed");

        let respawn = timeout(PROMPT, events.recv()).await;
        assert!(
            matches!(respawn, Ok(Some(WidgetEvent::Respawned { .. }))),
            "a refresh must retry the pending respawn well inside its climbed delay, got {respawn:?}"
        );
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

    /// The event stream closing is the only signal a listener has to stop, so
    /// anything holding the handle for as long as it consumes events would
    /// keep both alive forever.
    #[tokio::test]
    async fn dropping_the_handle_ends_the_actor_and_closes_the_stream() {
        let (manager, mut events) = WidgetManager::init(Vec::new(), false).await;

        drop(manager);

        assert!(
            timeout(EVENT_TIMEOUT, events.recv())
                .await
                .expect("timed out waiting for the event stream to close")
                .is_none(),
            "dropping the last handle must end the actor and close the stream"
        );
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
            WidgetEvent::Respawned { .. } | WidgetEvent::Abandoned { .. } => {
                panic!("a crash must be reported before the respawn")
            }
        }

        match next_event(&mut events).await {
            WidgetEvent::Respawned {
                instance_id,
                pid: new_pid,
            } => {
                assert_eq!(instance_id, "crashy");
                assert_ne!(new_pid, pid, "the respawn must carry the new pid");
            }
            WidgetEvent::Exited { .. } | WidgetEvent::Abandoned { .. } => {
                panic!("expected a respawn after the crash")
            }
        }
    }

    #[tokio::test]
    async fn uninstalled_widget_type_ends_supervision() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "true", &[]);
        manager
            .spawn_widget(uid, env("uninstalled"))
            .await
            .expect("BUG: test spawn failed");

        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Exited { .. }),
            "the crash must be reported first"
        );
        manager.registry().remove(&uid);

        match next_event(&mut events).await {
            WidgetEvent::Abandoned { instance_id } => {
                assert_eq!(instance_id, "uninstalled");
            }
            WidgetEvent::Exited { .. } | WidgetEvent::Respawned { .. } => {
                panic!("an uninstalled widget type must end supervision, not respawn")
            }
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

    /// Re-spawning over a live instance is reachable in production (preview a
    /// disabled scene, then enable it while the preview holds the stream open),
    /// and the replaced process must still get its destructors — a SIGKILL here
    /// leaks the CMA that the graceful stop exists to reclaim.
    #[tokio::test]
    async fn re_spawn_over_a_live_instance_stops_it_gracefully() {
        let dir = tempfile::tempdir().expect("BUG: test tempdir");
        let path = |name: &str| {
            dir.path()
                .join(name)
                .to_str()
                .expect("BUG: tempdir path is not UTF-8")
                .to_owned()
        };
        let (ready, terminated) = (path("ready"), path("sigterm-received"));
        let uid = Uuid::new_v4();
        // `ready` marks the trap as installed: a signal delivered before that
        // still carries the default disposition and would kill the shell
        // outright, which is a race in the test rather than in the manager.
        // `terminated` then separates a handled SIGTERM from a raw SIGKILL.
        // `sleep & wait` rather than a foreground `sleep`, since a shell waiting
        // on a foreground child defers the trap until that child exits.
        let script =
            format!("trap 'touch {terminated}; exit 0' TERM; touch {ready}; sleep 30 & wait");
        let (manager, mut events) = manager_with(uid, "sh", &["-c", &script]);

        let first = manager
            .spawn_widget(uid, env("doubled"))
            .await
            .expect("BUG: test spawn failed")
            .pid;
        await_file(&ready).await;

        let second = manager
            .spawn_widget(uid, env("doubled"))
            .await
            .expect("BUG: test re-spawn failed")
            .pid;
        assert_ne!(first, second, "the re-spawn must be a new process");

        assert!(
            await_file(&terminated).await,
            "the replaced widget must receive SIGTERM, not be dropped onto kill_on_drop"
        );

        let silence = timeout(SILENCE_TIMEOUT, events.recv()).await;
        assert!(
            silence.is_err(),
            "replacing a live instance is not a crash, so it must emit no events, got {silence:?}"
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
