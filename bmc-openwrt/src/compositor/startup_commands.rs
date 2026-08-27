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

use std::io;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use bmc::shutdown::COMPOSITOR_COMMAND_GRACE;
use bmc_shared_utils::process_supervisor::RestartPolicy;
use serde::Deserialize;
use tokio::process::Child;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub const SYSTEM_CONFIG_PATH: &str = "/etc/bmc_system.json";

const CHILD_POLL_INTERVAL: Duration = Duration::from_secs(1);
const COMPOSITOR_COMMAND_RESTART_INITIAL: Duration = Duration::from_secs(1);
/// Match procd's service respawn delay instead of leaving every overlay and
/// WASM widget without their shared host for the widget supervisor's ceiling.
const COMPOSITOR_COMMAND_RESTART_MAX: Duration = Duration::from_secs(5);
/// A minute of uptime is comfortably beyond the roughly seven-second climb to the ceiling,
/// so the next failure can start a fresh ladder.
const COMPOSITOR_COMMAND_HEALTHY_UPTIME: Duration = Duration::from_mins(1);

const fn compositor_command_restart_policy() -> RestartPolicy {
    RestartPolicy::new(
        COMPOSITOR_COMMAND_RESTART_INITIAL,
        COMPOSITOR_COMMAND_RESTART_MAX,
        COMPOSITOR_COMMAND_HEALTHY_UPTIME,
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SystemConfig {
    compositor: CompositorConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CompositorConfig {
    commands: Vec<Vec<String>>,
}

/// Must be dropped from a plain thread rather than a Tokio task.
/// Drop blocks until every command is reaped so Wayland can be torn down safely.
#[derive(Debug)]
pub struct StartupCommands {
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    runtime: Handle,
}

impl StartupCommands {
    pub fn empty(runtime: &Handle) -> Self {
        let (stop, _) = watch::channel(false);
        Self {
            stop,
            tasks: Vec::new(),
            runtime: runtime.clone(),
        }
    }

    pub fn load_and_spawn(path: &Path, wayland_display: &str, runtime: &Handle) -> Result<Self> {
        let restart_policy = compositor_command_restart_policy();
        let config = match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str::<SystemConfig>(&contents)
                .with_context(|| format!("parse system config {}", path.display()))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "system config not found; no compositor commands configured");
                SystemConfig::default()
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read system config {}", path.display()));
            }
        };

        let mut commands = Self::empty(runtime);
        for argv in config.compositor.commands {
            if let Err(error) = validate_command(&argv) {
                tracing::error!(?argv, %error, "invalid compositor command");
                continue;
            }
            let stop = commands.stop.subscribe();
            let wayland_display = wayland_display.to_owned();
            commands.tasks.push(runtime.spawn(async move {
                supervise_command(
                    &argv,
                    &wayland_display,
                    stop,
                    restart_policy,
                    CHILD_POLL_INTERVAL,
                )
                .await;
            }));
        }
        tracing::info!(
            path = %path.display(),
            count = commands.tasks.len(),
            "supervising compositor commands"
        );
        Ok(commands)
    }
}

async fn supervise_command(
    argv: &[String],
    wayland_display: &str,
    mut stop: watch::Receiver<bool>,
    policy: RestartPolicy,
    child_poll_interval: Duration,
) {
    let mut backoff = policy.initial();
    loop {
        if *stop.borrow() {
            return;
        }
        tracing::info!(?argv, %wayland_display, "starting compositor command");
        let started = Instant::now();
        let uptime =
            match tokio::process::Command::from(build_command(argv, wayland_display)).spawn() {
                Ok(mut child) => {
                    match wait_for_child(&mut child, &mut stop, child_poll_interval).await {
                        ChildExit::Stopped => return,
                        ChildExit::Exited => started.elapsed(),
                    }
                }
                Err(error) => {
                    tracing::error!(?argv, ?error, "failed to spawn compositor command");
                    Duration::ZERO
                }
            };
        let delay = advance_restart_backoff(policy, uptime, &mut backoff);
        tracing::info!(
            ?argv,
            delay_ms = delay.as_millis(),
            "waiting to restart compositor command"
        );
        match wait_for_retry(&mut stop, delay).await {
            RetryWait::Stopped => return,
            RetryWait::Elapsed => {}
        }
    }
}

fn advance_restart_backoff(
    policy: RestartPolicy,
    uptime: Duration,
    backoff: &mut Duration,
) -> Duration {
    let delay = policy.restart_delay(uptime, *backoff);
    *backoff = policy.next_backoff(delay);
    delay
}

#[derive(Debug, PartialEq, Eq)]
enum ChildExit {
    Exited,
    Stopped,
}

async fn wait_for_child(
    child: &mut Child,
    stop: &mut watch::Receiver<bool>,
    poll_interval: Duration,
) -> ChildExit {
    loop {
        if *stop.borrow() {
            stop_child(child).await;
            return ChildExit::Stopped;
        }
        let pid = child.id().expect("BUG: running child must have pid");
        let pid_t = libc::pid_t::try_from(pid).expect("BUG: child pid fits pid_t");
        match child_exited(pid_t) {
            Ok(true) => {
                stop_process_group(pid).await;
                let status = child
                    .wait()
                    .await
                    .expect("BUG: observed child must be reapable");
                tracing::warn!(pid, %status, "compositor command exited");
                return ChildExit::Exited;
            }
            Ok(false) => {}
            Err(error) => {
                tracing::error!(pid, %error, "failed to inspect compositor command; leaving process group untouched");
                // Inspection failure leaves numeric ownership uncertain, so skip signals;
                // wait only ever matches our child.
                if let Err(error) = child.wait().await {
                    tracing::warn!(pid, %error, "failed to reap compositor command");
                }
                return ChildExit::Exited;
            }
        }
        tokio::select! {
            () = tokio::time::sleep(poll_interval) => {}
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    stop_child(child).await;
                    return ChildExit::Stopped;
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RetryWait {
    Elapsed,
    Stopped,
}

async fn wait_for_retry(stop: &mut watch::Receiver<bool>, delay: Duration) -> RetryWait {
    let deadline = Instant::now() + delay;
    loop {
        if *stop.borrow() {
            return RetryWait::Stopped;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return RetryWait::Elapsed;
        }
        tokio::select! {
            () = tokio::time::sleep(remaining) => {}
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    return RetryWait::Stopped;
                }
            }
        }
    }
}

async fn stop_child(child: &mut Child) {
    let Some(pid) = child.id() else {
        return;
    };
    let pid = libc::pid_t::try_from(pid).expect("BUG: child pid fits pid_t");
    signal_process_group(pid, libc::SIGTERM, "terminate");
    let deadline = Instant::now() + COMPOSITOR_COMMAND_GRACE;
    let mut child_has_exited = false;
    let mut child_identity_known = true;
    loop {
        match child_exited(pid) {
            Ok(false) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(exited) => {
                child_has_exited = exited;
                break;
            }
            Err(error) => {
                tracing::warn!(pid, %error, "failed to inspect compositor command");
                child_identity_known = false;
                break;
            }
        }
    }
    if child_identity_known {
        wait_for_process_group_exit(pid, deadline).await;
        if !child_has_exited {
            tracing::warn!(
                pid,
                "compositor command exceeded SIGTERM grace; killing process group"
            );
            signal_process_group(pid, libc::SIGKILL, "kill");
        }
    }
    if let Err(error) = child.wait().await {
        tracing::warn!(pid, %error, "failed to reap compositor command");
    }
}

async fn stop_process_group(pid: u32) {
    let pid = libc::pid_t::try_from(pid).expect("BUG: child pid fits pid_t");
    if !process_group_has_live_members(pid) {
        return;
    }
    signal_process_group(pid, libc::SIGTERM, "terminate");
    let deadline = Instant::now() + COMPOSITOR_COMMAND_GRACE;
    wait_for_process_group_exit(pid, deadline).await;
}

async fn wait_for_process_group_exit(pid: libc::pid_t, deadline: Instant) {
    while process_group_has_live_members(pid) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if process_group_has_live_members(pid) {
        signal_process_group(pid, libc::SIGKILL, "kill");
    }
}

fn signal_process_group(pid: libc::pid_t, signal: libc::c_int, action: &str) {
    if unsafe { libc::kill(-pid, signal) } != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::warn!(pid, %error, "failed to {action} compositor command group");
        }
    }
}

fn process_group_has_live_members(group: libc::pid_t) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return true;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<libc::pid_t>() else {
            return false;
        };
        if pid == group {
            return false;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            return false;
        };
        let Some((_, fields)) = stat.rsplit_once(") ") else {
            return false;
        };
        let mut fields = fields.split_whitespace();
        let state = fields.next();
        let _parent = fields.next();
        let process_group = fields.next().and_then(|field| field.parse().ok());
        state != Some("Z") && process_group == Some(group)
    })
}

fn child_exited(pid: libc::pid_t) -> io::Result<bool> {
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        // WNOWAIT keeps the child unreaped so its process-group id cannot be
        // reused before cleanup.
        // SAFETY: `info` points to writable storage and is zeroed because
        // WNOHANG may succeed without writing when no child is waitable.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                libc::id_t::try_from(pid).expect("BUG: positive child pid fits id_t"),
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            // SAFETY: `info` was fully initialized by zeroing before waitid.
            let info = unsafe { info.assume_init() };
            // SAFETY: WEXITED selects the SIGCHLD layout, while WNOHANG
            // preserves the zero `si_pid` when no child is waitable.
            return Ok(unsafe { info.si_pid() } != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn validate_command(argv: &[String]) -> Result<()> {
    if argv.is_empty() {
        bail!("compositor command must contain a program");
    }
    Ok(())
}

/// The returned command must be spawned from a long-lived thread:
/// `PR_SET_PDEATHSIG` follows the thread that forks, not the whole process.
fn build_command(argv: &[String], wayland_display: &str) -> Command {
    let Some((program, args)) = argv.split_first() else {
        unreachable!("BUG: compositor command must be validated before supervision");
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .env("WAYLAND_DISPLAY", wayland_display)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.process_group(0);
    let parent_pid =
        libc::pid_t::try_from(std::process::id()).expect("BUG: compositor pid fits pid_t");
    // SAFETY: the closure performs only direct syscalls and constructs
    // allocation-free errors between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != parent_pid {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    command
}

impl Drop for StartupCommands {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        let tasks = std::mem::take(&mut self.tasks);
        self.runtime.block_on(async {
            for task in tasks {
                if let Err(error) = task.await {
                    tracing::error!(%error, "compositor command supervisor failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};
    use std::time::{Duration, Instant};
    use tokio::sync::watch;

    use super::{
        CHILD_POLL_INTERVAL, COMPOSITOR_COMMAND_RESTART_MAX, Child, ChildExit, Command,
        RestartPolicy, StartupCommands, SystemConfig, advance_restart_backoff, build_command,
        child_exited, compositor_command_restart_policy, process_group_has_live_members,
        stop_child, supervise_command, validate_command, wait_for_child,
    };

    // Serialize sibling spawns so they cannot reuse a PID during
    // PID-sensitive assertions.
    static SPAWN_TEST_LOCK: Mutex<()> = Mutex::new(());
    const PARENT_DEATH_HELPER_PID_PATH: &str = "BMC_PARENT_DEATH_HELPER_PID_PATH";

    fn spawn_test_lock() -> MutexGuard<'static, ()> {
        SPAWN_TEST_LOCK
            .lock()
            .expect("BUG: startup command test lock must not be poisoned")
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().expect("BUG: test runtime must start")
    }

    fn spawn(runtime: &tokio::runtime::Runtime, command: Command) -> Child {
        let _entered = runtime.enter();
        tokio::process::Command::from(command)
            .kill_on_drop(true)
            .spawn()
            .expect("BUG: test command must spawn")
    }

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !condition() {
            assert!(Instant::now() < deadline, "condition did not become true");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn read_pid(path: &std::path::Path) -> libc::pid_t {
        let mut pid = None;
        wait_for(|| {
            pid = std::fs::read_to_string(path)
                .ok()
                .and_then(|contents| contents.trim().parse().ok());
            pid.is_some()
        });
        pid.expect("BUG: wait_for returned only after the pid parsed")
    }

    fn process_is_running(pid: libc::pid_t) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        stat.rsplit_once(") ")
            .and_then(|(_, fields)| fields.chars().next())
            .is_some_and(|state| state != 'Z')
    }

    #[test]
    fn parent_death_signal_helper() {
        let Some(pid_path) = std::env::var_os(PARENT_DEATH_HELPER_PID_PATH) else {
            return;
        };
        let runtime = runtime();
        let command = build_command(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "echo $$ > {}; exec sleep 60",
                    std::path::Path::new(&pid_path).display()
                ),
            ],
            "wayland-7",
        );
        let _child = spawn(&runtime, command);
        wait_for(|| std::path::Path::new(&pid_path).exists());
        std::process::exit(0);
    }

    #[test]
    fn command_exits_when_its_compositor_parent_dies() {
        let _guard = spawn_test_lock();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let pid_path = directory.path().join("child.pid");
        let status = std::process::Command::new(
            std::env::current_exe().expect("BUG: current test executable must be available"),
        )
        .arg("parent_death_signal_helper")
        .arg("--nocapture")
        .env(PARENT_DEATH_HELPER_PID_PATH, &pid_path)
        .status()
        .expect("BUG: parent-death helper must run");
        assert!(
            status.success(),
            "parent-death helper must exit successfully"
        );
        let child = read_pid(&pid_path);

        wait_for(|| !process_is_running(child));
    }

    #[test]
    fn configured_command_receives_wayland_display() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let config_path = directory.path().join("bmc_system.json");
        let output_path = directory.path().join("wayland-display");
        let config = serde_json::json!({
            "compositor": {
                "commands": [[
                    "sh",
                    "-c",
                    format!(
                        "printf %s \"$WAYLAND_DISPLAY\" > {}; sleep 60",
                        output_path.display()
                    ),
                ]],
            },
        });
        std::fs::write(&config_path, config.to_string())
            .expect("BUG: temporary config must be writable");

        let commands = StartupCommands::load_and_spawn(&config_path, "wayland-7", runtime.handle())
            .expect("valid configured command must spawn");
        wait_for(|| std::fs::read_to_string(&output_path).is_ok_and(|value| value == "wayland-7"));

        assert_eq!(
            std::fs::read_to_string(output_path).expect("configured command must write output"),
            "wayland-7"
        );
        drop(commands);
    }

    #[test]
    fn dropping_commands_stops_and_reaps_the_process_group() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let config_path = directory.path().join("bmc_system.json");
        let leader_path = directory.path().join("leader-pid");
        let helper_path = directory.path().join("helper-pid");
        let config = serde_json::json!({
            "compositor": {
                "commands": [[
                    "sh",
                    "-c",
                    format!(
                        "printf %s $$ > {}; sleep 60 & printf %s $! > {}; wait",
                        leader_path.display(),
                        helper_path.display(),
                    ),
                ]],
            },
        });
        std::fs::write(&config_path, config.to_string())
            .expect("BUG: temporary config must be writable");
        let commands = StartupCommands::load_and_spawn(&config_path, "wayland-7", runtime.handle())
            .expect("valid configured command must spawn");
        let leader = read_pid(&leader_path);
        let helper = read_pid(&helper_path);
        let leader_proc = std::path::PathBuf::from(format!("/proc/{leader}"));
        assert!(process_is_running(leader), "command must be running");
        assert!(process_is_running(helper), "helper must be running");
        assert!(
            process_group_has_live_members(leader),
            "command group must contain its helper"
        );

        drop(commands);

        assert!(!leader_proc.exists(), "drop must reap the command");
        assert!(!process_is_running(helper), "drop must stop its helper");
        assert!(
            !process_group_has_live_members(leader),
            "drop must empty the command process group"
        );
    }

    #[test]
    fn system_config_defaults_to_no_commands() {
        let config: SystemConfig =
            serde_json::from_str("{}").expect("BUG: empty object is valid system config");

        assert!(config.compositor.commands.is_empty());
    }

    #[test]
    fn compositor_commands_use_a_host_wide_outage_ceiling() {
        let policy = compositor_command_restart_policy();

        assert_eq!(policy.max(), COMPOSITOR_COMMAND_RESTART_MAX);
        assert!(
            policy.max() < RestartPolicy::default().max(),
            "a shared-host outage must recover sooner than one missing widget"
        );
    }

    #[test]
    fn compositor_restart_backoff_advances_and_resets() {
        let policy = RestartPolicy::new(
            Duration::from_millis(10),
            Duration::from_millis(40),
            Duration::from_secs(1),
        );
        let mut backoff = policy.initial();
        let delays = [
            advance_restart_backoff(policy, Duration::ZERO, &mut backoff),
            advance_restart_backoff(policy, Duration::ZERO, &mut backoff),
            advance_restart_backoff(policy, Duration::ZERO, &mut backoff),
            advance_restart_backoff(policy, policy.healthy_uptime(), &mut backoff),
        ];

        assert_eq!(
            (delays, backoff),
            (
                [
                    Duration::from_millis(10),
                    Duration::from_millis(20),
                    Duration::from_millis(40),
                    Duration::from_millis(10),
                ],
                Duration::from_millis(20),
            )
        );
    }

    #[test]
    fn invalid_command_does_not_prevent_later_commands_from_starting() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let config_path = directory.path().join("bmc_system.json");
        let output_path = directory.path().join("started");
        let config = serde_json::json!({
            "compositor": {
                "commands": [
                    [],
                    [directory.path().join("missing-program")],
                    ["sh", "-c", format!("touch {}", output_path.display())],
                ],
            },
        });
        std::fs::write(&config_path, config.to_string())
            .expect("BUG: temporary config must be writable");

        let commands = StartupCommands::load_and_spawn(&config_path, "wayland-7", runtime.handle())
            .expect("valid system config must load");
        wait_for(|| output_path.exists());

        assert!(output_path.exists());
        drop(commands);
    }

    #[test]
    fn missing_config_defaults_to_no_commands() {
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");

        let commands = StartupCommands::load_and_spawn(
            &directory.path().join("missing.json"),
            "wayland-7",
            runtime.handle(),
        )
        .expect("missing system config must use defaults");

        assert!(commands.tasks.is_empty());
    }

    #[test]
    fn malformed_config_is_rejected() {
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let config_path = directory.path().join("bmc_system.json");
        std::fs::write(&config_path, "{").expect("BUG: temporary config must be writable");

        let error = StartupCommands::load_and_spawn(&config_path, "wayland-7", runtime.handle())
            .expect_err("malformed system config must fail to load");

        assert!(error.to_string().contains("parse system config"));
    }

    #[test]
    fn supervisor_uses_injected_restart_policy() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let starts_path = directory.path().join("starts");
        let argv = [
            "sh".to_owned(),
            "-c".to_owned(),
            format!("printf x >> {}", starts_path.display()),
        ];
        let policy = RestartPolicy::new(
            Duration::from_millis(10),
            Duration::from_millis(40),
            CHILD_POLL_INTERVAL,
        );
        let (stop, stop_rx) = watch::channel(false);
        let started = Instant::now();
        let supervisor = runtime.spawn(async move {
            supervise_command(&argv, "wayland-7", stop_rx, policy, policy.initial() / 10).await;
        });
        wait_for(|| std::fs::read(&starts_path).is_ok_and(|starts| starts.len() >= 4));
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(70),
            "the fourth start must follow the injected 10/20/40 ms ladder"
        );
        assert!(
            elapsed < CHILD_POLL_INTERVAL,
            "the injected millisecond policy must replace the production policy"
        );
        stop.send(true)
            .expect("BUG: supervisor must retain its stop receiver");
        runtime
            .block_on(supervisor)
            .expect("compositor command supervisor must stop");
    }

    #[test]
    fn dropping_commands_interrupts_restart_backoff() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let mut commands = StartupCommands::empty(runtime.handle());
        let stop = commands.stop.subscribe();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let starts_path = directory.path().join("starts");
        let argv = [
            "sh".to_owned(),
            "-c".to_owned(),
            format!("printf %s $$ > {}", starts_path.display()),
        ];
        let policy = RestartPolicy::new(
            CHILD_POLL_INTERVAL * 2,
            CHILD_POLL_INTERVAL * 2,
            CHILD_POLL_INTERVAL,
        );
        commands.tasks.push(runtime.spawn(async move {
            supervise_command(&argv, "wayland-7", stop, policy, CHILD_POLL_INTERVAL / 100).await;
        }));
        let child = read_pid(&starts_path);
        let child_proc = std::path::PathBuf::from(format!("/proc/{child}"));
        wait_for(|| !child_proc.exists());

        let started = Instant::now();
        drop(commands);

        assert!(
            started.elapsed() < CHILD_POLL_INTERVAL,
            "drop must interrupt the remaining backoff"
        );
    }

    #[test]
    fn command_receives_wayland_display() {
        let argv = vec!["/bin/true".to_owned(), "--example".to_owned()];
        let command = build_command(&argv, "wayland-7");

        assert_eq!(command.get_program(), "/bin/true");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("--example")]
        );
        assert!(command.get_envs().any(|(name, value)| {
            name == "WAYLAND_DISPLAY" && value == Some(std::ffi::OsStr::new("wayland-7"))
        }));
    }

    #[test]
    fn empty_command_is_rejected() {
        let error = validate_command(&[]).expect_err("empty argv must fail");

        assert!(error.to_string().contains("must contain a program"));
    }

    #[test]
    fn natural_child_exit_stops_its_background_process_group() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let pid_path = directory.path().join("grandchild.pid");
        let command = format!("trap '' TERM; sleep 60 & echo $! > {}", pid_path.display());
        let command = build_command(&["sh".to_owned(), "-c".to_owned(), command], "wayland-7");
        let mut child = spawn(&runtime, command);
        let grandchild = read_pid(&pid_path);
        let (_stop, mut stop_rx) = watch::channel(false);
        assert_eq!(
            runtime.block_on(wait_for_child(
                &mut child,
                &mut stop_rx,
                CHILD_POLL_INTERVAL,
            )),
            ChildExit::Exited
        );
        wait_for(|| !process_is_running(grandchild));
    }

    #[test]
    fn explicit_stop_reaps_child_and_stops_its_background_process_group() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let pid_path = directory.path().join("grandchild.pid");
        let command = format!(
            "trap '' TERM; sleep 60 & echo $! > {}; wait",
            pid_path.display()
        );
        let command = build_command(&["sh".to_owned(), "-c".to_owned(), command], "wayland-7");
        let mut child = spawn(&runtime, command);
        let child_pid = libc::pid_t::try_from(child.id().expect("BUG: child must have pid"))
            .expect("BUG: child pid fits pid_t");
        let grandchild = read_pid(&pid_path);

        runtime.block_on(stop_child(&mut child));

        let error = child_exited(child_pid).expect_err("reaped child must no longer be waitable");
        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
        wait_for(|| !process_is_running(grandchild));
    }

    #[test]
    fn explicit_stop_gives_background_process_remaining_term_grace() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let ready_path = directory.path().join("grandchild.ready");
        let graceful_path = directory.path().join("grandchild.graceful");
        let command = format!(
            "trap 'exit 0' TERM; sh -c 'trap \"sleep 0.2; touch {}; exit 0\" TERM; touch {}; while :; do sleep 1; done' & wait",
            graceful_path.display(),
            ready_path.display()
        );
        let command = build_command(&["sh".to_owned(), "-c".to_owned(), command], "wayland-7");
        let mut child = spawn(&runtime, command);
        wait_for(|| ready_path.exists());

        runtime.block_on(stop_child(&mut child));

        assert!(
            graceful_path.exists(),
            "background process must finish its SIGTERM handler before escalation"
        );
    }

    #[test]
    fn explicit_stop_kills_term_ignoring_child_without_helpers() {
        let _guard = spawn_test_lock();
        let runtime = runtime();
        let directory = tempfile::tempdir().expect("BUG: temporary directory must be creatable");
        let ready_path = directory.path().join("child.ready");
        let command = format!(
            "trap '' TERM; touch {}; exec sleep 60",
            ready_path.display()
        );
        let command = build_command(&["sh".to_owned(), "-c".to_owned(), command], "wayland-7");
        let mut child = spawn(&runtime, command);
        let child_pid = libc::pid_t::try_from(child.id().expect("BUG: child must have pid"))
            .expect("BUG: child pid fits pid_t");
        wait_for(|| ready_path.exists());

        runtime.block_on(stop_child(&mut child));

        let error = child_exited(child_pid).expect_err("reaped child must no longer be waitable");
        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
    }
}
