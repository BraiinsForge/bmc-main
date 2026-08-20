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
use crate::compositor::WidgetGeneration;

const DEFAULT_XDG_RUNTIME_DIR: &str = "/tmp/run";

/// Grace period for widget processes to clean up after SIGTERM before
/// resorting to SIGKILL. Widgets that hold GPU resources (GEM/DMA-BUF)
/// need time to run destructors so the kernel can reclaim CMA memory.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// What the actor knows about an instance's process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChildObservation {
    /// Running the carried build. A respawn re-reads the registry, so what a
    /// process runs can differ from what its first spawn asked for.
    Running(WidgetIdentity),
    /// Still supervised, but between processes: its respawn is pending.
    Exited,
    /// Not supervised at all: never spawned, stopped, or given up on.
    Missing,
}

/// Each command is awaited for its reply, so the depth is based on the number
/// of concurrent callers (not message rate).
///
/// Per-scene loops are sequential and gRPC edits serialise on the config lock,
/// so the ceiling is one command per widget of the largest scene —
/// the 4x2 slot grid. Sixteen is comfortably clear of it.
const COMMAND_CHANNEL_CAPACITY: usize = 16;

const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const RESTART_BACKOFF_FACTOR: u32 = 2;
const _: () = assert!(
    RESTART_BACKOFF_FACTOR >= 2,
    "a factor of 1 pins the ladder at its initial delay and 0 respawns without waiting at all"
);
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
    Exited {
        instance_id: String,
        generation: WidgetGeneration,
        pid: u32,
    },
    /// A crashed widget was respawned; the consumer must bind the new pid
    /// for the reconnecting process to be recognized.
    Respawned {
        instance_id: String,
        generation: WidgetGeneration,
        pid: u32,
    },
    /// Supervision gave up on the instance; nothing will bind a pid to it again,
    /// so the consumer must end the registration
    /// that [`Self::Exited`] deliberately left standing.
    Abandoned {
        instance_id: String,
        generation: WidgetGeneration,
    },
}

enum Command {
    Spawn {
        widget_uid: Uuid,
        generation: WidgetGeneration,
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
    RetryPending {
        instance_id: String,
        reply: oneshot::Sender<()>,
    },
    RegistryRefreshed {
        reply: oneshot::Sender<()>,
    },
}

type SpawnReply = Result<u32, SpawnError>;

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

    /// Bring a pending crash respawn forward to the initial delay while keeping
    /// its earned rung. A running or already-stopped instance is untouched.
    ///
    /// Callers push something that may be the fix — new params, a re-resolved
    /// credential — and this asks the widget to try it now rather than sit out
    /// a delay it earned against the configuration that was just replaced.
    pub(crate) async fn retry_pending(&self, instance_id: &str) {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::RetryPending {
                instance_id: instance_id.to_owned(),
                reply: reply_tx,
            })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a retry-pending reply");
    }

    /// Re-scan the widget discovery paths so newly-installed widgets become
    /// available without a restart. The re-scan itself is a no-op for a static
    /// registry; what follows happens either way.
    ///
    /// Also retries any pending crash respawn promptly, keeping the rung it
    /// had climbed. A failed re-scan leaves them alone: nothing changed.
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
    pub(crate) async fn spawn_widget(
        &self,
        widget_uid: Uuid,
        generation: WidgetGeneration,
        env: WidgetEnv,
    ) -> SpawnReply {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Spawn {
                widget_uid,
                generation,
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
    /// The compositor registration this process belongs to,
    /// carried across respawns so the events it emits stay addressed to it.
    generation: WidgetGeneration,
    /// The build this process was launched from, which the registry may since
    /// have replaced.
    identity: WidgetIdentity,
    /// Requests a graceful stop;
    /// carries the reply sender acknowledged once the process is gone.
    stop_tx: oneshot::Sender<oneshot::Sender<()>>,
    widget_uid: Uuid,
    /// Kept for a crash respawn. `wayland_display` cannot be re-derived —
    /// the manager holds no compositor reference by design — so a compositor
    /// restart (`ListeningSocket::bind_auto`) strands every env cached here.
    env: WidgetEnv,
    spawned_at: Instant,
    /// Delay before the respawn if this process crashes.
    backoff: Duration,
}

/// What a respawn needs to re-exec an instance, independent of when it fires.
struct Respawn {
    instance_id: String,
    widget_uid: Uuid,
    generation: WidgetGeneration,
    env: WidgetEnv,
}

/// A crashed instance waiting out its respawn delay.
struct PendingRestart {
    widget_uid: Uuid,
    generation: WidgetGeneration,
    env: WidgetEnv,
    /// Delay before the respawn after this one, should it crash again.
    backoff: Duration,
    /// Matches the queued [`Internal::RestartDue`] to this crash;
    /// a stop or an external re-spawn replaces the entry,
    /// orphaning the token and cancelling the respawn.
    token: u64,
    timer: tokio::task::JoinHandle<()>,
}

/// Stop a cancelled respawn's timer from sleeping out a delay
/// nobody will act on. Every cancellation path drops the entry,
/// so aborting here is the one place that cannot be forgotten.
///
/// The token stays load-bearing either way:
/// an [`Internal::RestartDue`] already in the channel cannot be un-sent.
impl Drop for PendingRestart {
    fn drop(&mut self) {
        self.timer.abort();
    }
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
    (delay * RESTART_BACKOFF_FACTOR).min(max)
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
                    // All handles dropped; dropping the children map ends
                    // every `run_child` task with it.
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
                generation,
                env,
                reply,
            } => {
                let initial = self.policy.initial;
                let _ = reply.send(self.spawn_process(
                    widget_uid,
                    generation,
                    env,
                    initial,
                    internal_tx,
                ));
            }
            Command::Stop { instance_id, reply } => self.stop(&instance_id, reply),
            Command::StopAll { reply } => self.stop_all(reply),
            Command::Observe { instance_id, reply } => {
                let _ = reply.send(self.observe(&instance_id));
            }
            Command::RetryPending { instance_id, reply } => {
                self.retry_pending(&instance_id, internal_tx);
                let _ = reply.send(());
            }
            Command::RegistryRefreshed { reply } => {
                self.reset_restart_backoff(internal_tx);
                let _ = reply.send(());
            }
        }
    }

    fn observe(&self, instance_id: &str) -> ChildObservation {
        match self.children.get(instance_id) {
            Some(WidgetState::Running(widget)) => {
                ChildObservation::Running(widget.identity.clone())
            }
            Some(WidgetState::PendingRestart(_)) => ChildObservation::Exited,
            None => ChildObservation::Missing,
        }
    }

    fn spawn_process(
        &mut self,
        widget_uid: Uuid,
        generation: WidgetGeneration,
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
                generation,
                identity: widget.identity,
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

        Ok(pid)
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
            generation: widget.generation,
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
            Respawn {
                instance_id: exit.instance_id,
                widget_uid: widget.widget_uid,
                generation: widget.generation,
                env: widget.env,
            },
            delay,
            next_backoff(delay, self.policy.max),
            internal_tx,
        );
    }

    fn schedule_restart(
        &mut self,
        respawn: Respawn,
        delay: Duration,
        next_delay: Duration,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) {
        let Respawn {
            instance_id,
            widget_uid,
            generation,
            env,
        } = respawn;
        let token = self.next_restart_token;
        self.next_restart_token += 1;
        let timer = tokio::spawn({
            let internal_tx = internal_tx.clone();
            let instance_id = instance_id.clone();
            async move {
                tokio::time::sleep(delay).await;
                let _ = internal_tx.send(Internal::RestartDue { instance_id, token });
            }
        });
        // Inserting drops any entry already here, aborting its timer.
        self.children.insert(
            instance_id,
            WidgetState::PendingRestart(PendingRestart {
                widget_uid,
                generation,
                env,
                backoff: next_delay,
                token,
                timer,
            }),
        );
    }

    /// Bring every pending respawn forward to the initial delay,
    /// keeping the rung it had already climbed to.
    ///
    /// A re-scan follows a package apply, which may or may not fix
    /// whatever keeps killing the widget. Retrying promptly finds out.
    /// Forgiving the ladder too would let repeated unrelated applies
    /// grind a real crash-looper back down to a 1 s tempo.
    fn reset_restart_backoff(&mut self, internal_tx: &mpsc::UnboundedSender<Internal>) {
        let pending: Vec<String> = self
            .children
            .iter()
            .filter_map(|(instance_id, state)| match state {
                WidgetState::PendingRestart(_) => Some(instance_id.clone()),
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
        for instance_id in pending {
            self.retry_pending(&instance_id, internal_tx);
        }
    }

    /// Re-arm one pending respawn at the initial delay, keeping its rung.
    ///
    /// Re-scheduling replaces the entry and orphans the old token,
    /// preventing the shortened delay from also firing.
    /// Running instances need nothing; stopped instances must stay stopped.
    fn retry_pending(&mut self, instance_id: &str, internal_tx: &mpsc::UnboundedSender<Internal>) {
        let Some(WidgetState::PendingRestart(pending)) = self.children.get(instance_id) else {
            return;
        };
        let respawn = Respawn {
            instance_id: instance_id.to_owned(),
            widget_uid: pending.widget_uid,
            generation: pending.generation,
            env: pending.env.clone(),
        };
        let earned = pending.backoff;
        self.schedule_restart(respawn, self.policy.initial, earned, internal_tx);
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
                generation: pending.generation,
            });
            return;
        }

        match self.spawn_process(
            pending.widget_uid,
            pending.generation,
            pending.env.clone(),
            pending.backoff,
            internal_tx,
        ) {
            Ok(pid) => {
                let _ = self.events_tx.send(WidgetEvent::Respawned {
                    instance_id: instance_id.to_owned(),
                    generation: pending.generation,
                    pid,
                });
            }
            Err(e) => {
                warn!(
                    "failed to respawn widget {}: {}; retrying in {:?}",
                    instance_id, e, pending.backoff
                );
                self.schedule_restart(
                    Respawn {
                        instance_id: instance_id.to_owned(),
                        widget_uid: pending.widget_uid,
                        generation: pending.generation,
                        env: pending.env.clone(),
                    },
                    pending.backoff,
                    next_backoff(pending.backoff, self.policy.max),
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
/// dropping the `Child` reaps the process via kill_on_drop —
/// a bare SIGKILL, skipping the CMA cleanup a graceful stop buys.
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
    use std::path::Path;
    use std::time::Instant;

    const GEN: WidgetGeneration = WidgetGeneration(1);

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

    async fn spawn(manager: &WidgetManager, instance_id: &str) -> u32 {
        manager
            .spawn_widget(
                Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: widget uid"),
                GEN,
                WidgetEnv {
                    instance_id: instance_id.to_owned(),
                    wayland_display: "wayland-test".to_owned(),
                },
            )
            .await
            .expect("BUG: spawn widget")
    }

    async fn wait_for_pending_restart(manager: &WidgetManager, instance_id: &str) {
        let deadline = Instant::now() + EVENT_TIMEOUT;
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
        let (running_manager, _events) =
            WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let _ = spawn(&running_manager, "running").await;
        let observation = running_manager.observe_child("running").await;
        assert!(
            matches!(&observation, ChildObservation::Running(identity)
                if identity.canonical_dir == running_dir),
            "a live child must report the build it was spawned from, got {observation:?}"
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

    /// The actor emits before it replies, so anything it had to say
    /// is already queued once the awaited command returns.
    fn assert_no_event(events: &mut mpsc::UnboundedReceiver<WidgetEvent>, expectation: &str) {
        let queued = events.try_recv();
        assert!(queued.is_err(), "{expectation}, got {queued:?}");
    }

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

        test_widget_build(manifest, "/test/widgets/test-widget")
    }

    /// The same widget type installed at another path, as a package replacement
    /// leaves it: same uid, different build.
    fn test_widget_build(manifest: Manifest, dir: &str) -> WidgetInfo {
        WidgetInfo::for_test(
            manifest,
            PathBuf::from(dir),
            PathBuf::from(dir).join("bin/widget"),
            None,
        )
    }

    /// Spawns a fixed command instead of the registry binary,
    /// so tests can simulate crashes and long-lived widgets.
    /// The binaries a spawner was handed, newest last.
    type LaunchLog = Arc<std::sync::Mutex<Vec<PathBuf>>>;

    struct FakeSpawner {
        program: String,
        args: Vec<String>,
        launched: LaunchLog,
    }

    impl WidgetSpawn for FakeSpawner {
        fn spawn(
            &self,
            widget: &WidgetInfo,
            _env: &WidgetEnv,
            _xdg_runtime_dir: &str,
        ) -> Result<Child, SpawnError> {
            self.launched
                .lock()
                .expect("BUG: launch log lock poisoned")
                .push(widget.binary_path.clone());
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
        let (manager, events, _launched) = manager_with_launch_log(uid, program, args, policy);
        (manager, events)
    }

    fn manager_with_launch_log(
        uid: Uuid,
        program: &str,
        args: &[&str],
        policy: RestartPolicy,
    ) -> (
        WidgetManager,
        mpsc::UnboundedReceiver<WidgetEvent>,
        LaunchLog,
    ) {
        let registry = Arc::new(WidgetRegistry::new([test_widget_info(uid)]));
        let launched = LaunchLog::default();
        let (manager, events) = WidgetManager::with_parts(
            registry,
            Box::new(FakeSpawner {
                program: program.to_owned(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                launched: Arc::clone(&launched),
            }),
            policy,
        );
        (manager, events, launched)
    }

    /// A ladder short enough to walk several rungs inside a test.
    fn fast_policy() -> RestartPolicy {
        RestartPolicy {
            initial: Duration::from_millis(10),
            max: Duration::from_millis(40),
            healthy_uptime: Duration::from_hours(1),
        }
    }

    fn bare_actor(uid: Uuid) -> Actor {
        let (events_tx, _events_rx) = mpsc::unbounded_channel();
        Actor {
            registry: Arc::new(WidgetRegistry::new([test_widget_info(uid)])),
            spawner: Box::new(FakeSpawner {
                program: "true".to_owned(),
                args: Vec::new(),
                launched: LaunchLog::default(),
            }),
            children: HashMap::new(),
            next_restart_token: 0,
            events_tx,
            policy: RestartPolicy::default(),
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

    /// The ceiling is a ceiling, not a give-up:
    /// supervision keeps retrying however long the crash loop lasts.
    #[tokio::test]
    async fn a_repeat_crasher_keeps_being_respawned() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with_policy(uid, "true", &[], fast_policy());
        manager
            .spawn_widget(uid, GEN, env("flapping"))
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
        /// The delay the ladder must have climbed to before the refresh.
        const CLIMBED: Duration = Duration::from_secs(1);
        /// Comfortably inside the climbed delay, so a retry landing within it
        /// cannot be that timer elapsing, yet loose enough for a loaded runner.
        const RETRY_BOUND: Duration = Duration::from_millis(500);

        let uid = Uuid::new_v4();
        let policy = RestartPolicy {
            initial: Duration::from_millis(1),
            max: Duration::from_secs(30),
            healthy_uptime: Duration::from_hours(1),
        };
        let (manager, mut events) = manager_with_policy(uid, "true", &[], policy);
        manager
            .spawn_widget(uid, GEN, env("flapping"))
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
        let crashed_at = Instant::now();

        manager
            .refresh()
            .await
            .expect("BUG: static registry refresh failed");

        let respawn = timeout(EVENT_TIMEOUT, events.recv()).await;
        assert!(
            matches!(respawn, Ok(Some(WidgetEvent::Respawned { .. }))),
            "a refresh must retry the pending respawn, got {respawn:?}"
        );
        // Timed from the crash, not from the refresh: a runner slow enough
        // that the climbed timer elapses on its own must fail here,
        // rather than certify a retry it never observed.
        let elapsed = crashed_at.elapsed();
        assert!(
            elapsed < RETRY_BOUND,
            "the retry must land inside {RETRY_BOUND:?} of the crash, took {elapsed:?}"
        );
    }

    /// The prompt retry a re-scan buys is one attempt, not a forgiven ladder.
    /// A widget the apply did not fix must drop straight back to the rung
    /// it had earned, so repeated unrelated applies cannot defeat the ceiling.
    #[tokio::test]
    async fn a_refresh_keeps_the_rung_it_climbed() {
        const INITIAL: Duration = Duration::from_millis(10);
        /// The ladder's ceiling here, a few crashes up from INITIAL.
        const CEILING: Duration = Duration::from_millis(400);
        /// Below the rung, so receive latency cannot fail the assertion,
        /// and far above the delay a forgiven ladder would have used.
        const RUNG_FLOOR: Duration = Duration::from_millis(200);

        let uid = Uuid::new_v4();
        let policy = RestartPolicy {
            initial: INITIAL,
            max: CEILING,
            healthy_uptime: Duration::from_hours(1),
        };
        let (manager, mut events) = manager_with_policy(uid, "true", &[], policy);
        manager
            .spawn_widget(uid, GEN, env("flapping"))
            .await
            .expect("BUG: test spawn failed");

        let mut delay = INITIAL;
        while delay < CEILING {
            assert!(matches!(
                next_event(&mut events).await,
                WidgetEvent::Exited { .. }
            ));
            assert!(matches!(
                next_event(&mut events).await,
                WidgetEvent::Respawned { .. }
            ));
            delay = next_backoff(delay, CEILING);
        }
        // This crash leaves a wait of CEILING pending.
        assert!(matches!(
            next_event(&mut events).await,
            WidgetEvent::Exited { .. }
        ));

        manager
            .refresh()
            .await
            .expect("BUG: static registry refresh failed");
        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Respawned { .. }),
            "the re-scan must buy a prompt attempt"
        );

        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Exited { .. }),
            "the attempt must fail again, since nothing actually changed"
        );
        let crashed_at = Instant::now();
        assert!(matches!(
            next_event(&mut events).await,
            WidgetEvent::Respawned { .. }
        ));
        // A lower bound, so a loaded runner can only overshoot it.
        let waited = crashed_at.elapsed();
        assert!(
            waited >= RUNG_FLOOR,
            "the next wait must be the earned rung near {CEILING:?}, waited {waited:?}"
        );
    }

    /// A params or credential push may be the fix for whatever keeps killing
    /// the widget, so it gets tried now rather than after the climbed delay —
    /// and the rung survives, exactly as a registry re-scan leaves it.
    #[tokio::test]
    async fn a_retry_brings_a_pending_respawn_forward_and_keeps_the_rung() {
        const INITIAL: Duration = Duration::from_millis(10);
        const CEILING: Duration = Duration::from_millis(400);
        /// Splits a prompt retry from one that sat out the rung. Comfortably
        /// inside CEILING either way, so neither bound rides on scheduling.
        const RUNG_THRESHOLD: Duration = Duration::from_millis(200);

        let uid = Uuid::new_v4();
        let policy = RestartPolicy {
            initial: INITIAL,
            max: CEILING,
            healthy_uptime: Duration::from_hours(1),
        };
        let (manager, mut events) = manager_with_policy(uid, "true", &[], policy);
        manager
            .spawn_widget(uid, GEN, env("flapping"))
            .await
            .expect("BUG: test spawn failed");

        let mut delay = INITIAL;
        while delay < CEILING {
            assert!(matches!(
                next_event(&mut events).await,
                WidgetEvent::Exited { .. }
            ));
            assert!(matches!(
                next_event(&mut events).await,
                WidgetEvent::Respawned { .. }
            ));
            delay = next_backoff(delay, CEILING);
        }
        // This crash leaves a wait of CEILING pending.
        assert!(matches!(
            next_event(&mut events).await,
            WidgetEvent::Exited { .. }
        ));

        let retried_at = Instant::now();
        manager.retry_pending("flapping").await;
        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Respawned { .. }),
            "the retry must respawn the instance"
        );
        let promptly = retried_at.elapsed();
        assert!(
            promptly < RUNG_THRESHOLD,
            "the push must be tried without waiting out the climbed delay, waited {promptly:?}"
        );

        assert!(matches!(
            next_event(&mut events).await,
            WidgetEvent::Exited { .. }
        ));
        let crashed_at = Instant::now();
        assert!(matches!(
            next_event(&mut events).await,
            WidgetEvent::Respawned { .. }
        ));
        let waited = crashed_at.elapsed();
        assert!(
            waited >= RUNG_THRESHOLD,
            "the next wait must still be the earned rung, waited {waited:?}"
        );
    }

    /// A retry must not resurrect a widget an explicit stop ended,
    /// nor disturb one that is running perfectly well.
    #[tokio::test]
    async fn a_retry_does_nothing_for_a_running_or_stopped_instance() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(uid, GEN, env("healthy"))
            .await
            .expect("BUG: test spawn failed");

        manager.retry_pending("healthy").await;
        assert!(
            matches!(
                manager.observe_child("healthy").await,
                ChildObservation::Running(_)
            ),
            "a running instance must be left alone"
        );

        manager.stop_widget("healthy").await;
        manager.retry_pending("healthy").await;
        assert_eq!(
            manager.observe_child("healthy").await,
            ChildObservation::Missing,
            "a stopped instance must stay stopped"
        );
        assert_no_event(&mut events, "neither retry may emit an event");
    }

    #[tokio::test]
    async fn spawn_unknown_widget_fails() {
        let (manager, _events) = WidgetManager::init(Vec::new(), false).await;
        let result = manager
            .spawn_widget(Uuid::new_v4(), GEN, env("test-instance"))
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
            .spawn_widget(uid, GEN, env("crashy"))
            .await
            .expect("BUG: test spawn failed");

        match next_event(&mut events).await {
            WidgetEvent::Exited {
                instance_id,
                generation,
                pid: exited_pid,
            } => {
                assert_eq!(instance_id, "crashy");
                assert_eq!(exited_pid, pid, "the exit must report the crashed pid");
                assert_eq!(generation, GEN, "the exit must report the spawn's stamp");
            }
            WidgetEvent::Respawned { .. } | WidgetEvent::Abandoned { .. } => {
                panic!("a crash must be reported before the respawn")
            }
        }

        match next_event(&mut events).await {
            WidgetEvent::Respawned {
                instance_id,
                generation,
                pid: new_pid,
            } => {
                assert_eq!(instance_id, "crashy");
                assert_ne!(new_pid, pid, "the respawn must carry the new pid");
                assert_eq!(
                    generation, GEN,
                    "a respawn stays on the registration it was spawned for"
                );
            }
            WidgetEvent::Exited { .. } | WidgetEvent::Abandoned { .. } => {
                panic!("expected a respawn after the crash")
            }
        }
    }

    /// A package replacement lands while the widget is crash-looping,
    /// so the respawn comes back on the new build.
    /// The observation must follow the process, not the spawn that started the ladder:
    /// the reload path decides from it what to restart, and would otherwise restart
    /// a widget that is already up to date.
    #[tokio::test]
    async fn a_respawn_reports_the_build_it_came_back_on() {
        /// Room to swap the registry entry after the crash, before the timer fires.
        const RESPAWN_DELAY: Duration = Duration::from_millis(500);

        let uid = Uuid::new_v4();
        let policy = RestartPolicy {
            initial: RESPAWN_DELAY,
            max: RESPAWN_DELAY,
            healthy_uptime: Duration::from_hours(1),
        };
        let (manager, mut events, launched) = manager_with_launch_log(uid, "true", &[], policy);
        manager
            .spawn_widget(uid, GEN, env("upgraded"))
            .await
            .expect("BUG: test spawn failed");

        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Exited { .. }),
            "the crash must be reported first"
        );
        manager.registry().replace(test_widget_build(
            test_widget_info(uid).manifest,
            "/test/widgets/test-widget-v2",
        ));

        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Respawned { .. }),
            "the widget must come back on the replaced build"
        );
        let observation = manager.observe_child("upgraded").await;
        assert!(
            matches!(&observation, ChildObservation::Running(identity)
                if identity.canonical_dir == Path::new("/test/widgets/test-widget-v2")),
            "the respawn must report the build it was launched from, got {observation:?}"
        );
        let launched = launched
            .lock()
            .expect("BUG: launch log lock poisoned")
            .clone();
        assert_eq!(
            launched,
            [
                PathBuf::from("/test/widgets/test-widget/bin/widget"),
                PathBuf::from("/test/widgets/test-widget-v2/bin/widget")
            ],
            "the respawn must launch the replaced build, not re-run the first one"
        );
    }

    #[tokio::test]
    async fn uninstalled_widget_type_ends_supervision() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "true", &[]);
        manager
            .spawn_widget(uid, GEN, env("uninstalled"))
            .await
            .expect("BUG: test spawn failed");

        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Exited { .. }),
            "the crash must be reported first"
        );
        manager.registry().remove(&uid);

        match next_event(&mut events).await {
            WidgetEvent::Abandoned {
                instance_id,
                generation,
            } => {
                assert_eq!(instance_id, "uninstalled");
                assert_eq!(generation, GEN, "the abandon must name the registration");
            }
            WidgetEvent::Exited { .. } | WidgetEvent::Respawned { .. } => {
                panic!("an uninstalled widget type must end supervision, not respawn")
            }
        }
    }

    /// The token makes an obsolete RestartDue harmless on arrival,
    /// but a task left sleeping still holds its delay —
    /// five minutes at the ceiling, once per re-schedule.
    #[tokio::test]
    async fn a_re_scheduled_respawn_aborts_the_timer_it_replaces() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let (internal_tx, mut internal_rx) = mpsc::unbounded_channel();
        let respawn = || Respawn {
            instance_id: "flapping".to_owned(),
            widget_uid: uid,
            generation: WidgetGeneration(1),
            env: env("flapping"),
        };

        let about_to_fire = Duration::from_millis(10);
        let ceiling = RESTART_BACKOFF_MAX;
        actor.schedule_restart(respawn(), about_to_fire, ceiling, &internal_tx);
        actor.schedule_restart(respawn(), ceiling, ceiling, &internal_tx);

        assert!(
            timeout(about_to_fire * 20, internal_rx.recv())
                .await
                .is_err(),
            "the delay the re-schedule cut short must not fire"
        );
    }

    #[tokio::test]
    async fn stopped_widget_is_not_respawned() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(uid, GEN, env("stoppable"))
            .await
            .expect("BUG: test spawn failed");

        manager.stop_widget("stoppable").await;

        assert_eq!(
            manager.observe_child("stoppable").await,
            ChildObservation::Missing,
            "an external stop must end supervision, leaving nothing to respawn"
        );
        assert_no_event(
            &mut events,
            "an externally stopped widget must emit no events",
        );
    }

    #[tokio::test]
    async fn stop_cancels_a_pending_respawn() {
        let uid = Uuid::new_v4();
        let (manager, mut events) = manager_with(uid, "true", &[]);
        manager
            .spawn_widget(uid, GEN, env("flappy"))
            .await
            .expect("BUG: test spawn failed");

        assert!(
            matches!(next_event(&mut events).await, WidgetEvent::Exited { .. }),
            "the crash must be reported first"
        );
        manager.stop_widget("flappy").await;

        assert_eq!(
            manager.observe_child("flappy").await,
            ChildObservation::Missing,
            "a stop during the respawn window must cancel the respawn"
        );
        assert_no_event(&mut events, "a cancelled respawn must emit no events");
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
            .spawn_widget(uid, GEN, env("doubled"))
            .await
            .expect("BUG: test spawn failed");
        assert!(
            await_file(&ready).await,
            "the trap must be installed before the re-spawn, or SIGTERM meets the default disposition"
        );

        let second = manager
            .spawn_widget(uid, GEN, env("doubled"))
            .await
            .expect("BUG: test re-spawn failed");
        assert_ne!(first, second, "the re-spawn must be a new process");

        assert!(
            await_file(&terminated).await,
            "the replaced widget must receive SIGTERM, not be dropped onto kill_on_drop"
        );

        assert!(
            matches!(
                manager.observe_child("doubled").await,
                ChildObservation::Running(_)
            ),
            "the replacement must be the process supervision now tracks"
        );
        assert_no_event(
            &mut events,
            "replacing a live instance is not a crash, so it must emit no events",
        );
    }

    #[tokio::test]
    async fn stop_all_returns_every_instance_id() {
        let uid = Uuid::new_v4();
        let (manager, _events) = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(uid, GEN, env("one"))
            .await
            .expect("BUG: test spawn failed");
        manager
            .spawn_widget(uid, GEN, env("two"))
            .await
            .expect("BUG: test spawn failed");

        let mut ids = manager.stop_all().await;
        ids.sort();
        assert_eq!(ids, ["one", "two"]);
    }
}
