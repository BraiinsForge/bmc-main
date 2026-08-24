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
use std::io::Error;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use tokio::process::Child;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{info, warn};
use uuid::Uuid;

use super::{SpawnError, WaylandSpawner, WidgetEnv, WidgetIdentity, WidgetInfo, WidgetRegistry};
use crate::scene::SceneId;

const DEFAULT_XDG_RUNTIME_DIR: &str = "/tmp/run";

/// Grace period for widget processes to clean up after SIGTERM before
/// resorting to SIGKILL. Widgets that hold GPU resources (GEM/DMA-BUF)
/// need time to run destructors so the kernel can reclaim CMA memory.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ManagerMode {
    Running,
    Paused,
    ShuttingDown,
}

impl ManagerMode {
    fn from_u8(value: u8) -> Self {
        match value {
            value if value == Self::Running as u8 => Self::Running,
            value if value == Self::Paused as u8 => Self::Paused,
            value if value == Self::ShuttingDown as u8 => Self::ShuttingDown,
            _ => unreachable!("BUG: invalid widget manager mode"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WidgetConfigKey {
    pub(crate) scene_id: SceneId,
    pub(crate) instance_id: Uuid,
    pub(crate) widget_uid: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WidgetLaunch {
    pub(crate) config_key: WidgetConfigKey,
    pub(crate) env: WidgetEnv,
    _sealed: (),
}

impl WidgetLaunch {
    pub(crate) fn new(
        scene_id: SceneId,
        widget_key: Uuid,
        widget_uid: Uuid,
        wayland_display: String,
    ) -> Self {
        Self {
            config_key: WidgetConfigKey {
                scene_id,
                instance_id: widget_key,
                widget_uid,
            },
            env: WidgetEnv { wayland_display },
            _sealed: (),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedWidgetState {
    Running,
    Stopping,
    PendingRestart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedWidgetSnapshot {
    pub(crate) launch: WidgetLaunch,
    pub(crate) identity: WidgetIdentity,
    pub(crate) state: ManagedWidgetState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagerSnapshot {
    pub(crate) mode: ManagerMode,
    pub(crate) widgets: Vec<ManagedWidgetSnapshot>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum StartError {
    #[error("widget instance {0} is already running or stopping")]
    Occupied(String),
    #[error("widget manager is {0:?}")]
    Mode(ManagerMode),
    #[error("widget registry changed before spawn")]
    RegistryChanged,
    #[error("widget start was superseded by a later lifecycle operation")]
    Superseded,
    #[error("widget process failed to spawn; restart remains pending: {0}")]
    PendingRestart(SpawnError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartPermit(u64);

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
    mode: Arc<AtomicU8>,
}

enum Command {
    PrepareStart {
        instance_id: String,
        expected_mode: ManagerMode,
        reply: oneshot::Sender<Result<StartPermit, StartError>>,
    },
    Spawn {
        launch: WidgetLaunch,
        identity: WidgetIdentity,
        permit: StartPermit,
        reply: oneshot::Sender<SpawnReply>,
    },
    Stop {
        instance_id: String,
        reply: oneshot::Sender<TerminationHandle>,
    },
    Pause {
        reply: oneshot::Sender<StopAllHandle>,
    },
    Shutdown {
        reply: oneshot::Sender<StopAllHandle>,
    },
    Resume {
        reply: oneshot::Sender<ManagerMode>,
    },
    RetryPending {
        instance_id: String,
        reply: oneshot::Sender<bool>,
    },
    RegistryRefreshed {
        reply: oneshot::Sender<()>,
    },
    Snapshot {
        reply: oneshot::Sender<ManagerSnapshot>,
    },
}

type SpawnReply = Result<(), StartError>;

pub(crate) struct StartHandle {
    reply: oneshot::Receiver<SpawnReply>,
}

impl StartHandle {
    pub(crate) async fn join(self) -> SpawnReply {
        self.reply
            .await
            .expect("BUG: widget child actor dropped a spawn reply")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TerminationHandle {
    done: watch::Receiver<bool>,
}

impl TerminationHandle {
    pub(crate) async fn join(mut self) {
        if *self.done.borrow() {
            return;
        }
        let _ = self.done.wait_for(|done| *done).await;
    }
}

pub(crate) struct StopAllHandle {
    instance_ids: Vec<String>,
    terminations: Vec<TerminationHandle>,
}

impl StopAllHandle {
    pub(crate) fn instance_ids(&self) -> &[String] {
        &self.instance_ids
    }

    pub(crate) async fn join(self) {
        futures::future::join_all(self.terminations.into_iter().map(TerminationHandle::join)).await;
    }
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
        Self::with_parts(registry, Box::new(spawner), RestartPolicy::default())
    }

    fn with_parts(
        registry: Arc<WidgetRegistry>,
        spawner: Box<dyn WidgetSpawn>,
        policy: RestartPolicy,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let mode = Arc::new(AtomicU8::new(ManagerMode::Running as u8));
        let actor = Actor {
            registry: registry.clone(),
            spawner,
            children: HashMap::new(),
            prepared_starts: HashMap::new(),
            mode: Arc::clone(&mode),
            next_start_token: 0,
            next_restart_token: 0,
            policy,
        };
        tokio::spawn(actor.run(cmd_rx));

        Self {
            registry,
            cmd_tx,
            mode,
        }
    }

    /// Get a shared reference to the widget registry.
    #[must_use]
    pub fn registry(&self) -> Arc<WidgetRegistry> {
        self.registry.clone()
    }

    pub(crate) fn mode(&self) -> ManagerMode {
        ManagerMode::from_u8(self.mode.load(Ordering::Acquire))
    }

    /// Bring a pending crash respawn forward to the initial delay while keeping
    /// its earned rung. A running or already-stopped instance is untouched.
    ///
    /// Callers push something that may be the fix — new params, a re-resolved
    /// credential — and this asks the widget to try it now rather than sit out
    /// a delay it earned against the configuration that was just replaced.
    pub(crate) async fn retry_pending(&self, instance_id: &str) -> bool {
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
            .expect("BUG: widget child actor dropped a retry-pending reply")
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

    /// A crashed widget is respawned automatically with backoff until it is
    /// stopped or its type leaves the registry.
    #[cfg(test)]
    pub(crate) async fn spawn_widget(&self, launch: WidgetLaunch) -> SpawnReply {
        let Some(identity) = self
            .registry
            .get(&launch.config_key.widget_uid)
            .map(|widget| widget.identity)
        else {
            return Err(StartError::RegistryChanged);
        };
        let permit = self
            .prepare_start(
                &launch.config_key.instance_id.to_string(),
                ManagerMode::Running,
            )
            .await?;
        self.enqueue_spawn_widget(launch, identity, permit)
            .await
            .join()
            .await
    }

    pub(crate) async fn prepare_start(
        &self,
        instance_id: &str,
        expected_mode: ManagerMode,
    ) -> Result<StartPermit, StartError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::PrepareStart {
                instance_id: instance_id.to_owned(),
                expected_mode,
                reply: reply_tx,
            })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a start-permit reply")
    }

    /// Enqueue a widget start and return a handle for its initial spawn result.
    pub(crate) async fn enqueue_spawn_widget(
        &self,
        launch: WidgetLaunch,
        identity: WidgetIdentity,
        permit: StartPermit,
    ) -> StartHandle {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Spawn {
                launch,
                identity,
                permit,
                reply: reply_tx,
            })
            .await
            .expect("BUG: widget child actor terminated");
        StartHandle { reply: reply_rx }
    }

    /// Request a stop and return a handle that joins the current child.
    /// A pending crash respawn is cancelled and returns an already-ready handle.
    pub(crate) async fn stop_widget(&self, instance_id: &str) -> TerminationHandle {
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
            .expect("BUG: widget child actor dropped a stop reply")
    }

    /// Begin a mode transition and return the affected instance IDs
    /// together with handles for pending process terminations.
    async fn begin_transition(&self, shutdown: bool) -> StopAllHandle {
        let (reply_tx, reply_rx) = oneshot::channel();
        let command = if shutdown {
            Command::Shutdown { reply: reply_tx }
        } else {
            Command::Pause { reply: reply_tx }
        };
        self.cmd_tx
            .send(command)
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a mode-transition reply")
    }

    pub(crate) async fn begin_pause(&self) -> StopAllHandle {
        self.begin_transition(false).await
    }

    pub(crate) async fn begin_shutdown(&self) -> StopAllHandle {
        self.begin_transition(true).await
    }

    #[cfg(test)]
    async fn transition_and_stop(&self, shutdown: bool) -> Vec<String> {
        let handle = self.begin_transition(shutdown).await;
        let instance_ids = handle.instance_ids.clone();
        handle.join().await;
        instance_ids
    }

    #[cfg(test)]
    pub(crate) async fn pause(&self) -> Vec<String> {
        self.transition_and_stop(false).await
    }

    #[cfg(test)]
    pub(crate) async fn shutdown(&self) -> Vec<String> {
        self.transition_and_stop(true).await
    }

    pub(crate) async fn resume(&self) -> ManagerMode {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Resume { reply: reply_tx })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a resume reply")
    }

    pub(crate) async fn snapshot(&self) -> ManagerSnapshot {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Snapshot { reply: reply_tx })
            .await
            .expect("BUG: widget child actor terminated");
        reply_rx
            .await
            .expect("BUG: widget child actor dropped a snapshot reply")
    }
}

/// Injected so supervision tests do not launch real widget binaries.
trait WidgetSpawn: Send + 'static {
    fn spawn(
        &self,
        widget: &WidgetInfo,
        widget_key: Uuid,
        env: &WidgetEnv,
        xdg_runtime_dir: &str,
    ) -> Result<Child, SpawnError>;
}

impl WidgetSpawn for WaylandSpawner {
    fn spawn(
        &self,
        widget: &WidgetInfo,
        widget_key: Uuid,
        env: &WidgetEnv,
        xdg_runtime_dir: &str,
    ) -> Result<Child, SpawnError> {
        WaylandSpawner::spawn(self, widget, widget_key, env, xdg_runtime_dir)
    }
}

enum WidgetState {
    Running(RunningWidget),
    Stopping(StoppingWidget),
    PendingRestart(PendingRestart),
}

/// A live child. The `Child` itself is owned by the instance's
/// [`run_child`] task; the actor holds the channels to reach it.
struct RunningWidget {
    pid: u32,
    /// The build this process was launched from, which the registry may since
    /// have replaced.
    identity: WidgetIdentity,
    stop_tx: oneshot::Sender<()>,
    done_tx: watch::Sender<bool>,
    termination: TerminationHandle,
    launch: WidgetLaunch,
    spawned_at: Instant,
    /// Delay before the respawn if this process crashes.
    backoff: Duration,
}

/// What a respawn needs to re-exec an instance, independent of when it fires.
struct Respawn {
    launch: WidgetLaunch,
    identity: WidgetIdentity,
}

/// A crashed instance waiting out its respawn delay.
struct PendingRestart {
    launch: WidgetLaunch,
    identity: WidgetIdentity,
    /// Delay before the respawn after this one, should it crash again.
    backoff: Duration,
    /// Matches the queued [`Internal::RestartDue`] to this crash;
    /// a stop or an explicit start replaces the entry,
    /// orphaning the token and cancelling the respawn.
    token: u64,
    timer: tokio::task::JoinHandle<()>,
}

struct StoppingWidget {
    launch: WidgetLaunch,
    identity: WidgetIdentity,
    done_tx: watch::Sender<bool>,
    termination: TerminationHandle,
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
    Exit(ChildExit),
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

/// The actor task: sole owner of every process lifecycle state.
struct Actor {
    registry: Arc<WidgetRegistry>,
    spawner: Box<dyn WidgetSpawn>,
    children: HashMap<String, WidgetState>,
    prepared_starts: HashMap<String, StartPermit>,
    mode: Arc<AtomicU8>,
    next_start_token: u64,
    next_restart_token: u64,
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
            Command::PrepareStart {
                instance_id,
                expected_mode,
                reply,
            } => {
                let _ = reply.send(self.prepare_start(instance_id, expected_mode));
            }
            Command::Spawn {
                launch,
                identity,
                permit,
                reply,
            } => {
                let _ = reply.send(self.start_expected(launch, &identity, permit, internal_tx));
            }
            Command::Stop { instance_id, reply } => {
                self.prepared_starts.remove(&instance_id);
                self.stop(&instance_id, reply);
            }
            Command::Pause { reply } => {
                self.prepared_starts.clear();
                let _ = reply.send(self.transition_and_stop(ManagerMode::Paused));
            }
            Command::Shutdown { reply } => {
                self.prepared_starts.clear();
                let _ = reply.send(self.transition_and_stop(ManagerMode::ShuttingDown));
            }
            Command::Resume { reply } => {
                let mode = if self.current_mode() == ManagerMode::ShuttingDown {
                    ManagerMode::ShuttingDown
                } else {
                    self.publish_mode(ManagerMode::Running);
                    ManagerMode::Running
                };
                let _ = reply.send(mode);
            }
            Command::RetryPending { instance_id, reply } => {
                let _ = reply.send(self.retry_pending(&instance_id, internal_tx));
            }
            Command::RegistryRefreshed { reply } => {
                self.reset_restart_backoff(internal_tx);
                let _ = reply.send(());
            }
            Command::Snapshot { reply } => {
                let _ = reply.send(self.snapshot());
            }
        }
    }

    fn current_mode(&self) -> ManagerMode {
        ManagerMode::from_u8(self.mode.load(Ordering::Acquire))
    }

    fn publish_mode(&self, mode: ManagerMode) {
        self.mode.store(mode as u8, Ordering::Release);
    }

    fn prepare_start(
        &mut self,
        instance_id: String,
        expected_mode: ManagerMode,
    ) -> Result<StartPermit, StartError> {
        if self.current_mode() != expected_mode {
            return Err(StartError::Mode(self.current_mode()));
        }
        let token = self.next_start_token;
        self.next_start_token = self
            .next_start_token
            .checked_add(1)
            .expect("BUG: widget start-permit token space exhausted");
        let permit = StartPermit(token);
        self.prepared_starts.insert(instance_id, permit);
        Ok(permit)
    }

    fn start_expected(
        &mut self,
        launch: WidgetLaunch,
        expected_identity: &WidgetIdentity,
        permit: StartPermit,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) -> SpawnReply {
        let instance_id = launch.config_key.instance_id.to_string();
        if self.prepared_starts.get(&instance_id) != Some(&permit) {
            return Err(StartError::Superseded);
        }
        self.prepared_starts.remove(&instance_id);
        if self.current_mode() != ManagerMode::Running {
            return Err(StartError::Mode(self.current_mode()));
        }
        let Some(widget) = self.registry.get(&launch.config_key.widget_uid) else {
            return Err(StartError::RegistryChanged);
        };
        if widget.identity != *expected_identity {
            return Err(StartError::RegistryChanged);
        }

        let pending = match self.children.remove(&instance_id) {
            Some(WidgetState::PendingRestart(pending)) => Some(pending),
            Some(WidgetState::Running(widget)) => {
                self.children
                    .insert(instance_id.clone(), WidgetState::Running(widget));
                return Err(StartError::Occupied(instance_id));
            }
            Some(WidgetState::Stopping(widget)) => {
                self.children
                    .insert(instance_id.clone(), WidgetState::Stopping(widget));
                return Err(StartError::Occupied(instance_id));
            }
            None => None,
        };
        let backoff = pending
            .as_ref()
            .map_or(self.policy.initial, |pending| pending.backoff);

        match self.spawn_selected_process(launch.clone(), backoff, &widget, internal_tx) {
            Ok(()) => Ok(()),
            Err((error, identity)) => {
                self.schedule_restart(
                    Respawn { launch, identity },
                    backoff,
                    next_backoff(backoff, self.policy.max),
                    internal_tx,
                );
                Err(StartError::PendingRestart(error))
            }
        }
    }

    #[cfg(test)]
    fn start(
        &mut self,
        launch: WidgetLaunch,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) -> SpawnReply {
        let identity = self
            .registry
            .get(&launch.config_key.widget_uid)
            .expect("BUG: test start requires an installed widget")
            .identity;
        let permit = self.prepare_start(
            launch.config_key.instance_id.to_string(),
            ManagerMode::Running,
        )?;
        self.start_expected(launch, &identity, permit, internal_tx)
    }

    fn spawn_selected_process(
        &mut self,
        launch: WidgetLaunch,
        backoff: Duration,
        widget: &WidgetInfo,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) -> Result<(), (SpawnError, WidgetIdentity)> {
        let identity = widget.identity.clone();
        let instance_id = launch.config_key.instance_id.to_string();

        info!(
            "spawning widget '{}' instance {}",
            widget.manifest.name, instance_id
        );

        let xdg_runtime_dir =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| DEFAULT_XDG_RUNTIME_DIR.to_owned());

        let mut child = self
            .spawner
            .spawn(
                widget,
                launch.config_key.instance_id,
                &launch.env,
                &xdg_runtime_dir,
            )
            .map_err(|error| (error, identity.clone()))?;
        let pid = child.id().ok_or_else(|| {
            (
                SpawnError::SpawnProcess(Error::other(
                    "spawned child has no pid (already exited?)",
                )),
                identity.clone(),
            )
        })?;

        let short_instance_id = instance_id.get(..8).unwrap_or(&instance_id).to_owned();
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
        let (done_tx, done) = watch::channel(false);
        let termination = TerminationHandle { done };
        self.children.insert(
            instance_id.clone(),
            WidgetState::Running(RunningWidget {
                pid,
                identity,
                stop_tx,
                done_tx,
                termination,
                launch,
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

        Ok(())
    }

    fn completed_termination() -> TerminationHandle {
        let (_done_tx, done) = watch::channel(true);
        TerminationHandle { done }
    }

    fn stop(&mut self, instance_id: &str, reply: oneshot::Sender<TerminationHandle>) {
        match self.children.remove(instance_id) {
            Some(WidgetState::Running(widget)) => {
                let termination = self.begin_stop(instance_id.to_owned(), widget);
                let _ = reply.send(termination);
            }
            Some(WidgetState::Stopping(widget)) => {
                let termination = widget.termination.clone();
                self.children
                    .insert(instance_id.to_owned(), WidgetState::Stopping(widget));
                let _ = reply.send(termination);
            }
            Some(WidgetState::PendingRestart(_)) => {
                let _ = reply.send(Self::completed_termination());
            }
            None => {
                warn!("attempted to stop unknown widget instance {}", instance_id);
                let _ = reply.send(Self::completed_termination());
            }
        }
    }

    fn begin_stop(&mut self, instance_id: String, widget: RunningWidget) -> TerminationHandle {
        let termination = widget.termination.clone();
        let _ = widget.stop_tx.send(());
        self.children.insert(
            instance_id,
            WidgetState::Stopping(StoppingWidget {
                launch: widget.launch,
                identity: widget.identity,
                done_tx: widget.done_tx,
                termination: termination.clone(),
            }),
        );
        termination
    }

    fn transition_and_stop(&mut self, requested: ManagerMode) -> StopAllHandle {
        let mode = if self.current_mode() == ManagerMode::ShuttingDown {
            ManagerMode::ShuttingDown
        } else {
            requested
        };
        self.publish_mode(mode);

        let mut ids = Vec::with_capacity(self.children.len());
        let mut terminations = Vec::with_capacity(self.children.len());
        let children = self.children.drain().collect::<Vec<_>>();
        for (id, state) in children {
            match state {
                WidgetState::Running(widget) => {
                    let termination = self.begin_stop(id.clone(), widget);
                    terminations.push(termination);
                }
                WidgetState::Stopping(widget) => {
                    terminations.push(widget.termination.clone());
                    self.children
                        .insert(id.clone(), WidgetState::Stopping(widget));
                }
                WidgetState::PendingRestart(_) => {}
            }
            ids.push(id);
        }
        StopAllHandle {
            instance_ids: ids,
            terminations,
        }
    }

    /// Take the entry only if it is still waiting out `token`'s respawn delay.
    /// A stop or an explicit start replaces the entry and orphans the token,
    /// so a mismatch means the timer lost its race and must do nothing.
    fn take_pending_restart(&mut self, instance_id: &str, token: u64) -> Option<PendingRestart> {
        match self.children.remove(instance_id)? {
            WidgetState::PendingRestart(pending) if pending.token == token => Some(pending),
            other @ (WidgetState::Running(_)
            | WidgetState::Stopping(_)
            | WidgetState::PendingRestart(_)) => {
                self.children.insert(instance_id.to_owned(), other);
                None
            }
        }
    }

    fn handle_exit(&mut self, exit: ChildExit, internal_tx: &mpsc::UnboundedSender<Internal>) {
        let Some(state) = self.children.remove(&exit.instance_id) else {
            return;
        };
        let widget = match state {
            WidgetState::Stopping(widget) => {
                let _ = widget.done_tx.send(true);
                return;
            }
            WidgetState::Running(widget) => widget,
            pending @ WidgetState::PendingRestart(_) => {
                self.children.insert(exit.instance_id, pending);
                return;
            }
        };
        assert_eq!(
            widget.pid, exit.pid,
            "BUG: current running child must be the only child able to exit"
        );

        if self.current_mode() != ManagerMode::Running {
            return;
        }

        let delay = restart_delay(widget.spawned_at.elapsed(), widget.backoff, &self.policy);
        let cause = exit
            .status
            .map_or_else(|| "wait failed".to_owned(), |status| status.to_string());
        // Scheduling, not promising:
        // the respawn re-checks the registry when the timer fires,
        // and gives up if the widget type has left it.
        warn!(
            "widget {} (pid={}) died unexpectedly ({cause}); scheduling a respawn in {:?}",
            exit.instance_id, exit.pid, delay
        );
        self.schedule_restart(
            Respawn {
                launch: widget.launch,
                identity: widget.identity,
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
        if self.current_mode() != ManagerMode::Running {
            return;
        }
        let instance_id = respawn.launch.config_key.instance_id.to_string();
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
                launch: respawn.launch,
                identity: respawn.identity,
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
        if self.current_mode() != ManagerMode::Running {
            return;
        }
        let pending: Vec<String> = self
            .children
            .iter()
            .filter_map(|(instance_id, state)| match state {
                WidgetState::PendingRestart(_) => Some(instance_id.clone()),
                WidgetState::Running(_) | WidgetState::Stopping(_) => None,
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
    fn retry_pending(
        &mut self,
        instance_id: &str,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) -> bool {
        let Some(WidgetState::PendingRestart(pending)) = self.children.get(instance_id) else {
            return false;
        };
        let respawn = Respawn {
            launch: pending.launch.clone(),
            identity: pending.identity.clone(),
        };
        let earned = pending.backoff;
        self.schedule_restart(respawn, self.policy.initial, earned, internal_tx);
        true
    }

    fn handle_restart_due(
        &mut self,
        instance_id: &str,
        token: u64,
        internal_tx: &mpsc::UnboundedSender<Internal>,
    ) {
        if self.current_mode() != ManagerMode::Running {
            let _ = self.take_pending_restart(instance_id, token);
            return;
        }
        // A mismatched token means the instance was stopped or replaced
        // while the timer ran; the respawn is cancelled.
        let Some(pending) = self.take_pending_restart(instance_id, token) else {
            return;
        };

        let Some(installed) = self.registry.get(&pending.launch.config_key.widget_uid) else {
            warn!(
                "widget type {} has left the registry; not respawning instance {}",
                pending.launch.config_key.widget_uid, instance_id
            );
            return;
        };
        if installed.identity != pending.identity {
            self.schedule_restart(
                Respawn {
                    launch: pending.launch.clone(),
                    identity: pending.identity.clone(),
                },
                pending.backoff,
                pending.backoff,
                internal_tx,
            );
            return;
        }

        match self.spawn_selected_process(
            pending.launch.clone(),
            pending.backoff,
            &installed,
            internal_tx,
        ) {
            Ok(()) => {}
            Err((e, identity)) => {
                warn!(
                    "failed to respawn widget {}: {}; retrying in {:?}",
                    instance_id, e, pending.backoff
                );
                self.schedule_restart(
                    Respawn {
                        launch: pending.launch.clone(),
                        identity,
                    },
                    pending.backoff,
                    next_backoff(pending.backoff, self.policy.max),
                    internal_tx,
                );
            }
        }
    }

    fn snapshot(&self) -> ManagerSnapshot {
        let mut widgets = self
            .children
            .values()
            .map(|state| match state {
                WidgetState::Running(widget) => ManagedWidgetSnapshot {
                    launch: widget.launch.clone(),
                    identity: widget.identity.clone(),
                    state: ManagedWidgetState::Running,
                },
                WidgetState::PendingRestart(widget) => ManagedWidgetSnapshot {
                    launch: widget.launch.clone(),
                    identity: widget.identity.clone(),
                    state: ManagedWidgetState::PendingRestart,
                },
                WidgetState::Stopping(widget) => ManagedWidgetSnapshot {
                    launch: widget.launch.clone(),
                    identity: widget.identity.clone(),
                    state: ManagedWidgetState::Stopping,
                },
            })
            .collect::<Vec<_>>();
        widgets.sort_by(|left, right| {
            left.launch
                .config_key
                .instance_id
                .cmp(&right.launch.config_key.instance_id)
        });
        ManagerSnapshot {
            mode: self.current_mode(),
            widgets,
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
    mut stop_rx: oneshot::Receiver<()>,
    internal_tx: mpsc::UnboundedSender<Internal>,
) {
    let mut stopped = false;
    let wait_result: Option<std::io::Result<ExitStatus>> = tokio::select! {
        status = child.wait() => Some(status),
        stop = &mut stop_rx => {
            stopped = stop.is_ok();
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
            if stopped {
                graceful_stop(instance_id.clone(), child).await;
                let _ = internal_tx.send(Internal::Exit(ChildExit {
                    instance_id,
                    pid,
                    status: None,
                }));
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
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    fn widget_key(label: &str) -> Uuid {
        let mut first = DefaultHasher::new();
        label.hash(&mut first);
        let mut second = DefaultHasher::new();
        (label, "widget-instance").hash(&mut second);
        Uuid::from_u64_pair(first.finish(), second.finish())
    }

    fn instance_id(label: &str) -> String {
        widget_key(label).to_string()
    }

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

    async fn spawn(manager: &WidgetManager, instance_id: &str) {
        let uid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: widget uid");
        manager
            .spawn_widget(launch(uid, instance_id))
            .await
            .expect("BUG: spawn widget");
    }

    async fn wait_for_pending_restart(manager: &WidgetManager, instance_id: &str) {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            let snapshot = snapshot_widget(manager, instance_id).await;
            if matches!(
                snapshot,
                Some(ManagedWidgetSnapshot {
                    state: ManagedWidgetState::PendingRestart,
                    ..
                })
            ) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "crashed child never reached a pending restart: {snapshot:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn snapshot_widget(
        manager: &WidgetManager,
        instance_id: &str,
    ) -> Option<ManagedWidgetSnapshot> {
        manager
            .snapshot()
            .await
            .widgets
            .into_iter()
            .find(|widget| widget.launch.config_key.instance_id == widget_key(instance_id))
    }

    #[tokio::test]
    async fn snapshots_distinguish_running_pending_and_missing() {
        let temp = tempfile::TempDir::new().expect("BUG: tempdir");
        let running_dir = temp.path().join("running");
        write_widget(&running_dir, "#!/bin/sh\nwhile :; do sleep 1; done\n");
        let running_manager = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        spawn(&running_manager, "running").await;
        let observation = snapshot_widget(&running_manager, "running").await;
        assert!(
            matches!(&observation, Some(ManagedWidgetSnapshot { identity, state: ManagedWidgetState::Running, .. })
                if identity.canonical_dir == running_dir),
            "a live child must report the build it was spawned from, got {observation:?}"
        );
        running_manager
            .stop_widget(&instance_id("running"))
            .await
            .join()
            .await;
        assert_eq!(
            snapshot_widget(&running_manager, "running").await,
            None,
            "an external stop ends supervision, so nothing is tracked"
        );

        std::fs::remove_dir_all(&running_dir).expect("BUG: remove running widget");
        let exited_dir = temp.path().join("exited");
        write_widget(&exited_dir, "#!/bin/sh\nexit 0\n");
        let exited_manager = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        spawn(&exited_manager, "exited").await;
        wait_for_pending_restart(&exited_manager, "exited").await;
    }

    #[tokio::test]
    async fn widget_process_receives_its_stable_key() {
        let temp = tempfile::TempDir::new().expect("BUG: tempdir");
        let output = temp.path().join("widget-key");
        let widget_dir = temp.path().join("widget");
        write_widget(
            &widget_dir,
            &format!(
                "#!/bin/sh\nprintf '%s' \"$BMC_WIDGET_KEY\" > {}\n",
                output.display()
            ),
        );
        let manager = WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let widget_uid =
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: widget uid");
        let widget_key = Uuid::new_v4();
        let expected_key = widget_key.to_string();
        let widget_launch = WidgetLaunch::new(
            SceneId::generate(),
            widget_key,
            widget_uid,
            "wayland-test".to_owned(),
        );

        manager
            .spawn_widget(widget_launch)
            .await
            .expect("BUG: test spawn failed");
        assert!(
            await_file_content(&output, &expected_key).await,
            "the widget must record its complete launch key"
        );
        manager.stop_widget(&expected_key).await.join().await;
    }

    const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

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

    struct FailOnceSpawner {
        fail_next: std::sync::atomic::AtomicBool,
    }

    impl WidgetSpawn for FailOnceSpawner {
        fn spawn(
            &self,
            _widget: &WidgetInfo,
            _widget_key: Uuid,
            _env: &WidgetEnv,
            _xdg_runtime_dir: &str,
        ) -> Result<Child, SpawnError> {
            if self.fail_next.swap(false, Ordering::SeqCst) {
                return Err(SpawnError::SpawnProcess(Error::other(
                    "simulated transient spawn failure",
                )));
            }
            let mut command = tokio::process::Command::new("sleep");
            command
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            command.spawn().map_err(SpawnError::SpawnProcess)
        }
    }

    impl WidgetSpawn for FakeSpawner {
        fn spawn(
            &self,
            widget: &WidgetInfo,
            _widget_key: Uuid,
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

    fn manager_with(uid: Uuid, program: &str, args: &[&str]) -> WidgetManager {
        manager_with_policy(uid, program, args, RestartPolicy::default())
    }

    fn manager_with_policy(
        uid: Uuid,
        program: &str,
        args: &[&str],
        policy: RestartPolicy,
    ) -> WidgetManager {
        let (manager, _launched) = manager_with_launch_log(uid, program, args, policy);
        manager
    }

    fn manager_with_launch_log(
        uid: Uuid,
        program: &str,
        args: &[&str],
        policy: RestartPolicy,
    ) -> (WidgetManager, LaunchLog) {
        let registry = Arc::new(WidgetRegistry::new([test_widget_info(uid)]));
        let launched = LaunchLog::default();
        let manager = WidgetManager::with_parts(
            registry,
            Box::new(FakeSpawner {
                program: program.to_owned(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                launched: Arc::clone(&launched),
            }),
            policy,
        );
        (manager, launched)
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
        Actor {
            registry: Arc::new(WidgetRegistry::new([test_widget_info(uid)])),
            spawner: Box::new(FakeSpawner {
                program: "true".to_owned(),
                args: Vec::new(),
                launched: LaunchLog::default(),
            }),
            children: HashMap::new(),
            prepared_starts: HashMap::new(),
            mode: Arc::new(AtomicU8::new(ManagerMode::Running as u8)),
            next_start_token: 0,
            next_restart_token: 0,
            policy: RestartPolicy::default(),
        }
    }

    fn launch(widget_uid: Uuid, instance_id: &str) -> WidgetLaunch {
        WidgetLaunch::new(
            SceneId::generate(),
            widget_key(instance_id),
            widget_uid,
            "wayland-test".to_owned(),
        )
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

    async fn await_file_content(path: &Path, expected: &str) -> bool {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            if std::fs::read_to_string(path).is_ok_and(|content| content == expected) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_launch_count(launched: &LaunchLog, expected: usize) {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            let count = launched
                .lock()
                .expect("BUG: launch log lock poisoned")
                .len();
            if count >= expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for launch {expected}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
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
        let (manager, launched) = manager_with_launch_log(uid, "true", &[], fast_policy());
        manager
            .spawn_widget(launch(uid, "flapping"))
            .await
            .expect("BUG: test spawn failed");

        wait_for_launch_count(&launched, 5).await;
    }

    /// A package replacement leaves affected widgets crash-looping
    /// while their files are swapped,
    /// and the reload that follows hands every non-running instance
    /// to supervision rather than replacing it.
    /// None of them may then serve out a delay
    /// earned against a build that has since been replaced.
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
        let (manager, launched) = manager_with_launch_log(uid, "true", &[], policy);
        manager
            .spawn_widget(launch(uid, "flapping"))
            .await
            .expect("BUG: test spawn failed");

        let mut delay = policy.initial;
        let mut launches = 1;
        while delay < CLIMBED {
            wait_for_pending_restart(&manager, "flapping").await;
            launches += 1;
            wait_for_launch_count(&launched, launches).await;
            delay = next_backoff(delay, policy.max);
        }
        wait_for_pending_restart(&manager, "flapping").await;
        let crashed_at = Instant::now();

        manager
            .refresh()
            .await
            .expect("BUG: static registry refresh failed");

        wait_for_launch_count(&launched, launches + 1).await;
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
        let (manager, launched) = manager_with_launch_log(uid, "true", &[], policy);
        manager
            .spawn_widget(launch(uid, "flapping"))
            .await
            .expect("BUG: test spawn failed");

        let mut delay = INITIAL;
        let mut launches = 1;
        while delay < CEILING {
            wait_for_pending_restart(&manager, "flapping").await;
            launches += 1;
            wait_for_launch_count(&launched, launches).await;
            delay = next_backoff(delay, CEILING);
        }
        wait_for_pending_restart(&manager, "flapping").await;

        manager
            .refresh()
            .await
            .expect("BUG: static registry refresh failed");
        launches += 1;
        wait_for_launch_count(&launched, launches).await;
        wait_for_pending_restart(&manager, "flapping").await;
        let crashed_at = Instant::now();
        wait_for_launch_count(&launched, launches + 1).await;
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
        let (manager, launched) = manager_with_launch_log(uid, "true", &[], policy);
        manager
            .spawn_widget(launch(uid, "flapping"))
            .await
            .expect("BUG: test spawn failed");

        let mut delay = INITIAL;
        let mut launches = 1;
        while delay < CEILING {
            wait_for_pending_restart(&manager, "flapping").await;
            launches += 1;
            wait_for_launch_count(&launched, launches).await;
            delay = next_backoff(delay, CEILING);
        }
        wait_for_pending_restart(&manager, "flapping").await;

        let retried_at = Instant::now();
        assert!(manager.retry_pending(&instance_id("flapping")).await);
        launches += 1;
        wait_for_launch_count(&launched, launches).await;
        let promptly = retried_at.elapsed();
        assert!(
            promptly < RUNG_THRESHOLD,
            "the push must be tried without waiting out the climbed delay, waited {promptly:?}"
        );

        wait_for_pending_restart(&manager, "flapping").await;
        let crashed_at = Instant::now();
        wait_for_launch_count(&launched, launches + 1).await;
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
        let manager = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(launch(uid, "healthy"))
            .await
            .expect("BUG: test spawn failed");

        assert!(!manager.retry_pending(&instance_id("healthy")).await);
        assert!(
            matches!(
                snapshot_widget(&manager, "healthy").await,
                Some(ManagedWidgetSnapshot {
                    state: ManagedWidgetState::Running,
                    ..
                })
            ),
            "a running instance must be left alone"
        );

        manager
            .stop_widget(&instance_id("healthy"))
            .await
            .join()
            .await;
        assert!(!manager.retry_pending(&instance_id("healthy")).await);
        assert_eq!(
            snapshot_widget(&manager, "healthy").await,
            None,
            "a stopped instance must stay stopped"
        );
    }

    #[tokio::test]
    async fn spawn_unknown_widget_fails() {
        let manager = WidgetManager::init(Vec::new(), false).await;
        let result = manager
            .spawn_widget(launch(Uuid::new_v4(), "test-instance"))
            .await;
        assert!(result.is_err(), "spawn must fail for an unknown widget");
    }

    #[tokio::test]
    async fn stop_unknown_widget_returns() {
        let manager = WidgetManager::init(Vec::new(), false).await;
        manager.stop_widget("no-such-instance").await.join().await;
    }

    #[tokio::test]
    async fn pause_without_widgets_returns_no_ids() {
        let manager = WidgetManager::init(Vec::new(), false).await;
        assert!(manager.pause().await.is_empty());
    }

    #[tokio::test]
    async fn crashed_widget_respawns_without_waiting_for_external_coordination() {
        let uid = Uuid::new_v4();
        let (manager, launched) = manager_with_launch_log(uid, "true", &[], fast_policy());
        manager
            .spawn_widget(launch(uid, "crashy"))
            .await
            .expect("BUG: test spawn failed");

        wait_for_launch_count(&launched, 2).await;
    }

    #[tokio::test]
    async fn a_widget_type_gone_from_the_registry_ends_supervision() {
        let uid = Uuid::new_v4();
        let manager = manager_with(uid, "true", &[]);
        manager
            .spawn_widget(launch(uid, "skipped"))
            .await
            .expect("BUG: test spawn failed");

        wait_for_pending_restart(&manager, "skipped").await;
        manager.registry().remove(&uid);
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while snapshot_widget(&manager, "skipped").await.is_some() {
            assert!(Instant::now() < deadline, "supervision did not end");
            tokio::time::sleep(Duration::from_millis(5)).await;
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
            launch: launch(uid, "flapping"),
            identity: test_widget_info(uid).identity,
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
        let manager = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(launch(uid, "stoppable"))
            .await
            .expect("BUG: test spawn failed");

        manager
            .stop_widget(&instance_id("stoppable"))
            .await
            .join()
            .await;

        assert_eq!(
            snapshot_widget(&manager, "stoppable").await,
            None,
            "an external stop must end supervision, leaving nothing to respawn"
        );
    }

    #[tokio::test]
    async fn stop_cancels_a_pending_respawn() {
        let uid = Uuid::new_v4();
        let manager = manager_with(uid, "true", &[]);
        manager
            .spawn_widget(launch(uid, "flappy"))
            .await
            .expect("BUG: test spawn failed");

        wait_for_pending_restart(&manager, "flappy").await;
        manager
            .stop_widget(&instance_id("flappy"))
            .await
            .join()
            .await;

        assert_eq!(
            snapshot_widget(&manager, "flappy").await,
            None,
            "a stop during the respawn window must cancel the respawn"
        );
    }

    #[tokio::test]
    async fn start_rejects_a_running_instance_until_it_is_reaped() {
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
        let manager = manager_with(uid, "sh", &["-c", &script]);

        manager
            .spawn_widget(launch(uid, "doubled"))
            .await
            .expect("BUG: test spawn failed");
        assert!(
            await_file(&ready).await,
            "the trap must be installed before stop, or SIGTERM meets the default disposition"
        );

        let occupied = manager.spawn_widget(launch(uid, "doubled")).await;
        assert!(
            matches!(occupied, Err(StartError::Occupied(ref instance)) if instance == &instance_id("doubled")),
            "a start must not replace a running predecessor: {occupied:?}"
        );

        manager
            .stop_widget(&instance_id("doubled"))
            .await
            .join()
            .await;

        assert!(
            await_file(&terminated).await,
            "the stopped widget must receive SIGTERM, not be dropped onto kill_on_drop"
        );

        manager
            .spawn_widget(launch(uid, "doubled"))
            .await
            .expect("BUG: start after reap failed");

        assert!(
            matches!(
                snapshot_widget(&manager, "doubled").await,
                Some(ManagedWidgetSnapshot {
                    state: ManagedWidgetState::Running,
                    ..
                })
            ),
            "the successor must be the process supervision now tracks"
        );
    }

    #[tokio::test]
    async fn pause_returns_every_instance_id() {
        let uid = Uuid::new_v4();
        let manager = manager_with(uid, "sleep", &["30"]);
        manager
            .spawn_widget(launch(uid, "one"))
            .await
            .expect("BUG: test spawn failed");
        manager
            .spawn_widget(launch(uid, "two"))
            .await
            .expect("BUG: test spawn failed");

        let mut ids = manager.pause().await;
        ids.sort();
        assert_eq!(ids, [instance_id("one"), instance_id("two")]);
    }

    #[tokio::test]
    async fn explicit_start_failure_keeps_one_pending_timer_and_the_earned_rung() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        actor.spawner = Box::new(FakeSpawner {
            program: "/definitely/missing/widget".to_owned(),
            args: Vec::new(),
            launched: LaunchLog::default(),
        });
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();

        assert!(matches!(
            actor.start(launch(uid, "pending"), &internal_tx),
            Err(StartError::PendingRestart(_))
        ));
        let first = match actor.children.get(&instance_id("pending")) {
            Some(WidgetState::PendingRestart(pending)) => (pending.backoff, pending.token),
            Some(WidgetState::Running(_) | WidgetState::Stopping(_)) | None => {
                panic!("spawn failure must remain supervised")
            }
        };

        assert!(matches!(
            actor.start(launch(uid, "pending"), &internal_tx),
            Err(StartError::PendingRestart(_))
        ));
        let second = match actor.children.get(&instance_id("pending")) {
            Some(WidgetState::PendingRestart(pending)) => (pending.backoff, pending.token),
            Some(WidgetState::Running(_) | WidgetState::Stopping(_)) | None => {
                panic!("retry failure must remain supervised")
            }
        };
        assert_eq!(second.0, next_backoff(first.0, actor.policy.max));
        assert_ne!(second.1, first.1, "the old timer token must be replaced");
        assert_eq!(actor.children.len(), 1, "only one timer may remain armed");
        let snapshot = actor.snapshot();
        assert!(matches!(
            snapshot.widgets.as_slice(),
            [ManagedWidgetSnapshot {
                launch: WidgetLaunch {
                    config_key: WidgetConfigKey { widget_uid, .. },
                    ..
                },
                identity,
                state: ManagedWidgetState::PendingRestart,
            }] if *widget_uid == uid
                && identity.canonical_dir == Path::new("/test/widgets/test-widget")
        ));
    }

    #[tokio::test]
    async fn transient_explicit_spawn_failure_recovers_without_registry_activity() {
        let uid = Uuid::new_v4();
        let registry = Arc::new(WidgetRegistry::new([test_widget_info(uid)]));
        let manager = WidgetManager::with_parts(
            registry,
            Box::new(FailOnceSpawner {
                fail_next: std::sync::atomic::AtomicBool::new(true),
            }),
            fast_policy(),
        );

        assert!(matches!(
            manager.spawn_widget(launch(uid, "recovering")).await,
            Err(StartError::PendingRestart(_))
        ));
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            if matches!(
                snapshot_widget(&manager, "recovering").await,
                Some(ManagedWidgetSnapshot {
                    state: ManagedWidgetState::Running,
                    ..
                })
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "widget did not recover");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn repeated_stop_joins_one_stopping_child_while_actor_responds() {
        let dir = tempfile::tempdir().expect("BUG: test tempdir");
        let ready = dir.path().join("ready");
        let ready = ready.to_str().expect("BUG: tempdir path is not UTF-8");
        let script = format!(
            "trap 'sleep 1; exit 0' TERM; touch {ready}; while :; do sleep 30 & wait; done"
        );
        let uid = Uuid::new_v4();
        let manager = manager_with(uid, "sh", &["-c", &script]);
        manager
            .spawn_widget(launch(uid, "slow"))
            .await
            .expect("BUG: test spawn failed");
        assert!(await_file(ready).await, "the signal trap must be installed");

        let first = manager.stop_widget(&instance_id("slow")).await;
        let second = manager.stop_widget(&instance_id("slow")).await;
        let snapshot = timeout(Duration::from_millis(200), manager.snapshot())
            .await
            .expect("the actor must answer while graceful termination is pending");
        assert!(matches!(
            snapshot.widgets.as_slice(),
            [ManagedWidgetSnapshot {
                launch: WidgetLaunch {
                    config_key: WidgetConfigKey { widget_uid, .. },
                    ..
                },
                identity,
                state: ManagedWidgetState::Stopping,
                ..
            }] if *widget_uid == uid
                && identity.canonical_dir == Path::new("/test/widgets/test-widget")
        ));
        assert!(matches!(
            manager.spawn_widget(launch(uid, "slow")).await,
            Err(StartError::Occupied(_))
        ));

        timeout(
            EVENT_TIMEOUT,
            futures::future::join(first.join(), second.join()),
        )
        .await
        .expect("every stop waiter must resolve after reap");
        assert!(manager.snapshot().await.widgets.is_empty());
    }

    #[tokio::test]
    async fn closed_stop_channel_waits_for_the_actor_to_process_exit() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let widget_launch = launch(uid, "queued-exit");
        let (stop_tx, stop_rx) = oneshot::channel();
        drop(stop_rx);
        let (done_tx, done) = watch::channel(false);
        actor.children.insert(
            instance_id("queued-exit"),
            WidgetState::Running(RunningWidget {
                pid: 42,
                identity: test_widget_info(uid).identity,
                stop_tx,
                done_tx,
                termination: TerminationHandle { done },
                launch: widget_launch,
                spawned_at: Instant::now(),
                backoff: actor.policy.initial,
            }),
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        actor.stop(&instance_id("queued-exit"), reply_tx);
        let handle = reply_rx.await.expect("BUG: stop reply");
        assert!(
            !*handle.done.borrow(),
            "a closed stop channel does not prove the child has been reaped"
        );

        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        actor.handle_exit(
            ChildExit {
                instance_id: instance_id("queued-exit"),
                pid: 42,
                status: None,
            },
            &internal_tx,
        );
        assert!(
            *handle.done.borrow(),
            "the handle becomes ready only after the actor removes Stopping"
        );
        assert!(actor.children.is_empty());
    }

    #[tokio::test]
    async fn pause_cancels_pending_work_and_terminal_shutdown_absorbs_resume() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        actor.spawner = Box::new(FakeSpawner {
            program: "/definitely/missing/widget".to_owned(),
            args: Vec::new(),
            launched: LaunchLog::default(),
        });
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        assert!(actor.start(launch(uid, "pending"), &internal_tx).is_err());
        let stale_token = match actor.children.get(&instance_id("pending")) {
            Some(WidgetState::PendingRestart(pending)) => pending.token,
            Some(WidgetState::Running(_) | WidgetState::Stopping(_)) | None => {
                panic!("spawn failure must be pending")
            }
        };

        let paused = actor.transition_and_stop(ManagerMode::Paused);
        assert_eq!(paused.instance_ids, [instance_id("pending")]);
        actor.handle_restart_due(&instance_id("pending"), stale_token, &internal_tx);
        assert!(
            actor.children.is_empty(),
            "paused stale work must stay inert"
        );
        assert!(matches!(
            actor.start(launch(uid, "pending"), &internal_tx),
            Err(StartError::Mode(ManagerMode::Paused))
        ));

        actor.transition_and_stop(ManagerMode::ShuttingDown);
        assert_eq!(
            actor.current_mode(),
            ManagerMode::ShuttingDown,
            "terminal transition must publish before it returns"
        );

        let manager = manager_with(uid, "sleep", &["30"]);
        let shutdown = manager.begin_shutdown().await;
        assert_eq!(manager.mode(), ManagerMode::ShuttingDown);
        assert_eq!(manager.resume().await, ManagerMode::ShuttingDown);
        shutdown.join().await;
        assert!(matches!(
            manager.spawn_widget(launch(uid, "stale")).await,
            Err(StartError::Mode(ManagerMode::ShuttingDown))
        ));
    }

    #[tokio::test]
    async fn paused_manager_resumes_and_accepts_upgrade_recovery_starts() {
        let uid = Uuid::new_v4();
        let manager = manager_with(uid, "sleep", &["30"]);

        assert!(manager.pause().await.is_empty());
        assert_eq!(manager.mode(), ManagerMode::Paused);
        assert!(matches!(
            manager.spawn_widget(launch(uid, "paused")).await,
            Err(StartError::Mode(ManagerMode::Paused))
        ));

        assert_eq!(manager.resume().await, ManagerMode::Running);
        manager
            .spawn_widget(launch(uid, "recovered"))
            .await
            .expect("a failed upgrade must be able to restart widgets");
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn queued_exit_keeps_completed_child_occupied_until_the_actor_handles_it() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let (internal_tx, mut internal_rx) = mpsc::unbounded_channel();
        actor
            .start(launch(uid, "queued"), &internal_tx)
            .expect("BUG: test child must start");
        let Internal::Exit(exit) = internal_rx.recv().await.expect("child exit") else {
            panic!("BUG: child must report Exit")
        };

        assert!(matches!(
            actor.start(launch(uid, "queued"), &internal_tx),
            Err(StartError::Occupied(_))
        ));
        actor.handle_exit(exit, &internal_tx);
        assert!(!matches!(
            actor.children.get(&instance_id("queued")),
            Some(WidgetState::Running(_) | WidgetState::Stopping(_))
        ));
    }

    #[test]
    fn start_rejects_registration_for_a_different_registry_identity() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let mut stale = actor.registry.get(&uid).expect("BUG: test widget").identity;
        stale.version = semver::Version::new(0, 0, 0);
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        let launch = launch(uid, "stale-build");
        let permit = actor
            .prepare_start(
                launch.config_key.instance_id.to_string(),
                ManagerMode::Running,
            )
            .expect("BUG: test actor is running");

        assert!(matches!(
            actor.start_expected(launch, &stale, permit, &internal_tx),
            Err(StartError::RegistryChanged)
        ));
        assert!(actor.children.is_empty());
    }

    #[tokio::test]
    async fn stale_start_does_not_consume_a_newer_permit() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let launch = launch(uid, "overlapping-starts");
        let identity = actor.registry.get(&uid).expect("BUG: test widget").identity;
        let stale = actor
            .prepare_start(
                launch.config_key.instance_id.to_string(),
                ManagerMode::Running,
            )
            .expect("BUG: test actor is running");
        let current = actor
            .prepare_start(
                launch.config_key.instance_id.to_string(),
                ManagerMode::Running,
            )
            .expect("BUG: test actor is running");
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();

        assert!(matches!(
            actor.start_expected(launch.clone(), &identity, stale, &internal_tx),
            Err(StartError::Superseded)
        ));
        assert!(
            actor
                .start_expected(launch, &identity, current, &internal_tx)
                .is_ok(),
            "the current permit must remain usable after rejecting its predecessor"
        );
    }

    #[tokio::test]
    async fn resume_preserves_a_prepared_paused_start() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let launch = launch(uid, "upgrade-resume");
        let identity = actor.registry.get(&uid).expect("BUG: test widget").identity;
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        let (pause_reply, _pause_reply_rx) = oneshot::channel();
        actor.handle_command(Command::Pause { reply: pause_reply }, &internal_tx);
        let permit = actor
            .prepare_start(
                launch.config_key.instance_id.to_string(),
                ManagerMode::Paused,
            )
            .expect("BUG: test actor is paused");
        let (resume_reply, _resume_reply_rx) = oneshot::channel();

        actor.handle_command(
            Command::Resume {
                reply: resume_reply,
            },
            &internal_tx,
        );

        assert_eq!(actor.current_mode(), ManagerMode::Running);
        assert!(
            actor
                .start_expected(launch, &identity, permit, &internal_tx)
                .is_ok(),
            "resume must preserve a start prepared while paused"
        );
    }

    #[test]
    fn stop_invalidates_a_prepared_start_without_a_child() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let launch = launch(uid, "stopped-before-spawn");
        let identity = actor.registry.get(&uid).expect("BUG: test widget").identity;
        let permit = actor
            .prepare_start(
                launch.config_key.instance_id.to_string(),
                ManagerMode::Running,
            )
            .expect("BUG: test actor is running");
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        let (reply, _reply_rx) = oneshot::channel();

        actor.handle_command(
            Command::Stop {
                instance_id: launch.config_key.instance_id.to_string(),
                reply,
            },
            &internal_tx,
        );

        assert!(matches!(
            actor.start_expected(launch, &identity, permit, &internal_tx),
            Err(StartError::Superseded)
        ));
        assert!(actor.children.is_empty());
    }

    #[test]
    fn pause_invalidates_prepared_running_starts() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let launch = launch(uid, "prepared-before-pause");
        let identity = actor.registry.get(&uid).expect("BUG: test widget").identity;
        let permit = actor
            .prepare_start(
                launch.config_key.instance_id.to_string(),
                ManagerMode::Running,
            )
            .expect("BUG: test actor is running");
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        let (reply, _reply_rx) = oneshot::channel();

        actor.handle_command(Command::Pause { reply }, &internal_tx);

        assert_eq!(actor.current_mode(), ManagerMode::Paused);
        assert!(matches!(
            actor.start_expected(launch, &identity, permit, &internal_tx),
            Err(StartError::Superseded)
        ));
        assert!(actor.children.is_empty());
    }

    #[test]
    fn shutdown_invalidates_prepared_running_starts() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let launch = launch(uid, "prepared-before-shutdown");
        let identity = actor.registry.get(&uid).expect("BUG: test widget").identity;
        let permit = actor
            .prepare_start(
                launch.config_key.instance_id.to_string(),
                ManagerMode::Running,
            )
            .expect("BUG: test actor is running");
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        let (reply, _reply_rx) = oneshot::channel();

        actor.handle_command(Command::Shutdown { reply }, &internal_tx);

        assert_eq!(actor.current_mode(), ManagerMode::ShuttingDown);
        assert!(matches!(
            actor.start_expected(launch, &identity, permit, &internal_tx),
            Err(StartError::Superseded)
        ));
        assert!(actor.children.is_empty());
    }

    #[tokio::test]
    async fn pending_restart_defers_when_the_registry_points_at_another_build() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        let launched = LaunchLog::default();
        actor.spawner = Box::new(FakeSpawner {
            program: "/definitely/missing/widget".to_owned(),
            args: Vec::new(),
            launched: Arc::clone(&launched),
        });
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        assert!(matches!(
            actor.start(launch(uid, "build-swap"), &internal_tx),
            Err(StartError::PendingRestart(_))
        ));
        let (token, identity, backoff) = match actor.children.get(&instance_id("build-swap")) {
            Some(WidgetState::PendingRestart(pending)) => {
                (pending.token, pending.identity.clone(), pending.backoff)
            }
            _ => panic!("spawn failure must remain pending"),
        };
        launched
            .lock()
            .expect("BUG: launch log lock poisoned")
            .clear();

        let mut manifest = actor.registry.get(&uid).expect("BUG: test widget").manifest;
        manifest.version = semver::Version::new(2, 0, 0);
        actor
            .registry
            .replace(test_widget_build(manifest, "/test/widgets/build-b"));
        actor.spawner = Box::new(FakeSpawner {
            program: "true".to_owned(),
            args: Vec::new(),
            launched: Arc::clone(&launched),
        });

        actor.handle_restart_due(&instance_id("build-swap"), token, &internal_tx);

        assert!(
            launched
                .lock()
                .expect("BUG: launch log lock poisoned")
                .is_empty(),
            "the replacement build must wait for a matching retained registration"
        );
        assert!(matches!(
            actor.children.get(&instance_id("build-swap")),
            Some(WidgetState::PendingRestart(pending))
                if pending.identity == identity
                    && pending.backoff == backoff
                    && pending.token != token
        ));
    }

    #[tokio::test]
    async fn queued_exit_after_pause_or_shutdown_cannot_create_successor_work() {
        for mode in [ManagerMode::Paused, ManagerMode::ShuttingDown] {
            let uid = Uuid::new_v4();
            let mut actor = bare_actor(uid);
            let (internal_tx, mut internal_rx) = mpsc::unbounded_channel();
            actor
                .start(launch(uid, "queued"), &internal_tx)
                .expect("BUG: test child must start");
            let Internal::Exit(exit) = internal_rx.recv().await.expect("child exit") else {
                panic!("BUG: child must report Exit")
            };
            actor.publish_mode(mode);
            actor.handle_exit(exit, &internal_tx);
            assert!(actor.children.is_empty());
        }
    }

    #[tokio::test]
    async fn stale_restart_due_cannot_replace_an_explicit_successor() {
        let uid = Uuid::new_v4();
        let mut actor = bare_actor(uid);
        actor.spawner = Box::new(FakeSpawner {
            program: "/definitely/missing/widget".to_owned(),
            args: Vec::new(),
            launched: LaunchLog::default(),
        });
        let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
        assert!(matches!(
            actor.start(launch(uid, "stale"), &internal_tx),
            Err(StartError::PendingRestart(_))
        ));
        let token = match actor.children.get(&instance_id("stale")) {
            Some(WidgetState::PendingRestart(pending)) => pending.token,
            _ => panic!("spawn failure must remain pending"),
        };
        actor.spawner = Box::new(FakeSpawner {
            program: "sleep".to_owned(),
            args: vec!["30".to_owned()],
            launched: LaunchLog::default(),
        });
        actor
            .start(launch(uid, "stale"), &internal_tx)
            .expect("explicit retry must start");
        let pid = match actor.children.get(&instance_id("stale")) {
            Some(WidgetState::Running(widget)) => widget.pid,
            _ => panic!("explicit retry must leave a running child"),
        };

        actor.handle_restart_due(&instance_id("stale"), token, &internal_tx);
        assert!(matches!(
            actor.children.get(&instance_id("stale")),
            Some(WidgetState::Running(widget)) if widget.pid == pid
        ));
    }
}
