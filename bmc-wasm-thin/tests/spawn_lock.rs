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

use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bmc_wasm_thin::args::Config;
use bmc_wasm_thin::ownership::{CompositorIdentity, commit_record, current_compositor_identity};
use bmc_wasm_thin::spawn::{
    ForeignHostTerminator, HostLauncher, SignalForeignHostTerminator, connect_or_spawn,
    connect_or_spawn_with_launcher, connect_or_spawn_with_launcher_and_terminator,
    foreign_host_signal_waits, spawn_daemon_with_env, wait_for_readiness,
};
use tempfile::TempDir;

fn make_config(td: &TempDir, name: &str) -> Config {
    let host_socket = td.path().join(format!("{name}.sock"));
    let lockfile = td.path().join(format!("{name}.lock"));
    Config {
        wasm: PathBuf::from("/dev/null"),
        host_socket,
        lockfile,
        owner_record: td.path().join(format!("{name}.owner")),
        host_bin: PathBuf::from(env!("CARGO_BIN_EXE_bmc-wasm-thin-fakehost")),
        host_wait: Duration::from_secs(5),
        ack_wait: Duration::from_secs(5),
    }
}

struct ForeignHost {
    child: Option<Child>,
}

impl ForeignHost {
    fn wait(mut self) -> ExitStatus {
        self.child
            .take()
            .expect("BUG: foreign host child")
            .wait()
            .expect("BUG: wait for foreign host")
    }
}

impl Drop for ForeignHost {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TermBehavior {
    Exit,
    Ignore,
}

enum InitialRecord {
    Missing,
    Matching,
}

fn start_foreign_host(config: &Config, term_behavior: TermBehavior) -> ForeignHost {
    start_foreign_host_with_env(config, term_behavior, &[])
}

fn start_foreign_host_with_env(
    config: &Config,
    term_behavior: TermBehavior,
    extra_env: &[(&str, String)],
) -> ForeignHost {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bmc-wasm-thin-fakehost"));
    command
        .env(
            "BMC_THIN_FAKE_HOST_SOCKET",
            config.host_socket.display().to_string(),
        )
        .env("BMC_THIN_FAKE_HOST_RELEASE_LOCK_FD", "-1")
        .env("BMC_THIN_FAKE_HOST_ACCEPTS", "1")
        .env("BMC_THIN_FAKE_HOST_HOLD", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if matches!(term_behavior, TermBehavior::Ignore) {
        command.env("BMC_THIN_FAKE_HOST_IGNORE_TERM", "1");
    }
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("BUG: spawn foreign fake host");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !config.host_socket.exists() {
        assert!(
            child.try_wait().expect("BUG: poll foreign host").is_none(),
            "foreign host exited before binding"
        );
        assert!(Instant::now() < deadline, "foreign host did not bind");
        std::thread::sleep(Duration::from_millis(10));
    }
    ForeignHost { child: Some(child) }
}

fn replacement_launcher(counter: PathBuf) -> FakeLauncher {
    FakeLauncher {
        extra_env: vec![],
        counter: Some(counter),
    }
}

fn assert_current_record(config: &Config) {
    let current = current_compositor_identity().expect("BUG: read current compositor identity");
    let record = std::fs::read_to_string(&config.owner_record)
        .expect("ownership record should be committed")
        .parse::<CompositorIdentity>()
        .expect("ownership record should parse");
    assert_eq!(record, current);
}

fn assert_spawned_once(counter: &PathBuf) {
    assert_eq!(
        std::fs::read_to_string(counter).expect("replacement spawn counter"),
        "spawn\n"
    );
}

#[test]
fn matching_identity_reuses_live_host_without_spawning() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "matching");
    let current = current_compositor_identity().expect("BUG: read current compositor identity");
    commit_record(&config.owner_record, &current).expect("BUG: commit matching record");
    let mut foreign = start_foreign_host(&config, TermBehavior::Exit);
    let counter = td.path().join("spawn_counter");

    let stream = connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
        .expect("matching host should be reused");
    drop(stream);

    assert!(
        !counter.exists(),
        "matching host must not spawn a replacement"
    );
    assert!(
        foreign
            .child
            .as_mut()
            .expect("BUG: foreign host child")
            .try_wait()
            .expect("BUG: poll matching host")
            .is_none(),
        "matching host must remain alive"
    );
}

#[test]
fn different_caller_reuses_host_owned_by_live_compositor() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "live-other-compositor");
    let caller = current_compositor_identity().expect("BUG: read caller identity");
    let pid = libc::pid_t::try_from(std::process::id()).expect("BUG: test pid does not fit pid_t");
    let stat = std::fs::read_to_string("/proc/self/stat").expect("BUG: read test process stat");
    let recorded = CompositorIdentity {
        boot_id: caller.boot_id,
        pid,
        starttime: bmc_wasm_thin::ownership::parse_proc_stat_starttime(&stat)
            .expect("BUG: parse test process stat"),
    };
    assert_ne!(recorded.pid, caller.pid, "test needs a different caller");
    commit_record(&config.owner_record, &recorded).expect("BUG: commit live owner record");
    let mut host = start_foreign_host(&config, TermBehavior::Exit);
    let counter = td.path().join("spawn_counter");

    let stream = connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
        .expect("host owned by a live compositor should be reused");
    drop(stream);

    assert!(
        !counter.exists(),
        "live compositor's host must not spawn a replacement"
    );
    assert!(
        host.child
            .as_mut()
            .expect("BUG: foreign host child")
            .try_wait()
            .expect("BUG: poll live compositor host")
            .is_none(),
        "live compositor's host must remain alive"
    );
}

#[test]
fn missing_record_terminates_live_host_and_spawns_replacement() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "missing-record");
    let foreign = start_foreign_host(&config, TermBehavior::Exit);
    let counter = td.path().join("spawn_counter");

    let stream = connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
        .expect("foreign host should be replaced");
    drop(stream);

    assert_spawned_once(&counter);
    assert_current_record(&config);
    assert_eq!(
        foreign.wait().signal(),
        Some(libc::SIGTERM),
        "foreign host should exit from SIGTERM"
    );
}

// The real daemon launcher must surface the grandchild's exec failure:
// a missing host binary fails the spawn loudly and removes the record,
// rather than leaving it stale behind a later misleading connect error.
#[test]
fn missing_host_binary_fails_spawn_and_removes_the_record() {
    let td = TempDir::new().expect("BUG: tempdir");
    let mut config = make_config(&td, "missing-binary");
    config.host_bin = td.path().join("no-such-host");

    let error = connect_or_spawn(&config).expect_err("spawn of a missing host binary must fail");

    let chain = format!("{error:#}");
    assert!(
        chain.contains("spawn bmc-wasm-host"),
        "failure must point at the spawn, not a later connect: {chain}"
    );
    assert!(
        !config.owner_record.exists(),
        "failed spawn must remove the ownership record"
    );
}

// A live host closes a hello-less probe after its hello timeout,
// err-ack in flight. A thin descheduled past that timeout sees
// data-then-EOF on the held connection; that must read as alive —
// treating it as death would adopt the live foreign host as ours.
#[test]
fn probe_closed_with_pending_err_ack_still_terminates_live_host() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "err-ack-drop");
    let marker = td.path().join("err-ack-drop-marker");
    let foreign = start_foreign_host_with_env(
        &config,
        TermBehavior::Exit,
        &[(
            "BMC_THIN_FAKE_HOST_ERR_ACK_DROP",
            marker.display().to_string(),
        )],
    );
    let stream = UnixStream::connect(&config.host_socket).expect("BUG: probe connect");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "fake host did not err-ack-drop");
        std::thread::sleep(Duration::from_millis(10));
    }

    let (term_wait, kill_wait) = foreign_host_signal_waits(config.host_wait);
    SignalForeignHostTerminator
        .terminate(
            stream,
            &config.host_socket,
            Instant::now() + config.host_wait,
            term_wait,
            kill_wait,
        )
        .expect("live host behind a closed probe should be terminated");

    assert_eq!(
        foreign.wait().signal(),
        Some(libc::SIGTERM),
        "host that closed the probe while alive must still be signaled"
    );
}

#[test]
fn mismatched_identity_fields_each_replace_live_host() {
    let td = TempDir::new().expect("BUG: tempdir");
    let current = current_compositor_identity().expect("BUG: read current compositor identity");
    let cases = [
        (
            "pid",
            CompositorIdentity {
                pid: current.pid.checked_add(1).expect("BUG: pid increment"),
                ..current.clone()
            },
        ),
        (
            "starttime",
            CompositorIdentity {
                starttime: current
                    .starttime
                    .checked_add(1)
                    .expect("BUG: starttime increment"),
                ..current.clone()
            },
        ),
        (
            "boot",
            CompositorIdentity {
                boot_id: format!("{}-other", current.boot_id),
                ..current
            },
        ),
    ];

    for (name, recorded) in cases {
        let config = make_config(&td, name);
        commit_record(&config.owner_record, &recorded).expect("BUG: commit mismatched record");
        let foreign = start_foreign_host(&config, TermBehavior::Exit);
        let counter = td.path().join(format!("{name}-counter"));

        let stream =
            connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
                .expect("mismatched host should be replaced");
        drop(stream);

        assert_spawned_once(&counter);
        assert_current_record(&config);
        assert_eq!(foreign.wait().signal(), Some(libc::SIGTERM));
    }
}

#[test]
fn malformed_and_unreadable_records_replace_live_host() {
    let td = TempDir::new().expect("BUG: tempdir");
    for (name, contents) in [("truncated", "boot 1"), ("malformed", "not-an-identity")] {
        let config = make_config(&td, name);
        std::fs::write(&config.owner_record, contents).expect("BUG: write corrupt record");
        let foreign = start_foreign_host(&config, TermBehavior::Exit);
        let counter = td.path().join(format!("{name}-counter"));

        let stream =
            connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
                .expect("corrupt claim should replace host");
        drop(stream);

        assert_spawned_once(&counter);
        assert_current_record(&config);
        assert_eq!(foreign.wait().signal(), Some(libc::SIGTERM));
    }

    let config = make_config(&td, "unreadable");
    std::fs::write(&config.owner_record, "old 1 2\n").expect("BUG: write unreadable record");
    std::fs::set_permissions(&config.owner_record, std::fs::Permissions::from_mode(0o000))
        .expect("BUG: make record unreadable");
    let foreign = start_foreign_host(&config, TermBehavior::Exit);
    let counter = td.path().join("unreadable-counter");

    let stream = connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
        .expect("unreadable claim should replace host");
    drop(stream);

    assert_spawned_once(&counter);
    assert_current_record(&config);
    assert_eq!(foreign.wait().signal(), Some(libc::SIGTERM));
}

#[test]
fn stale_socket_and_matching_record_without_listener_each_spawn() {
    let td = TempDir::new().expect("BUG: tempdir");
    for (name, initial_record) in [
        ("stale", InitialRecord::Missing),
        ("claimed", InitialRecord::Matching),
    ] {
        let config = make_config(&td, name);
        if matches!(initial_record, InitialRecord::Matching) {
            let current =
                current_compositor_identity().expect("BUG: read current compositor identity");
            commit_record(&config.owner_record, &current).expect("BUG: commit matching record");
        }
        let stale_listener =
            UnixListener::bind(&config.host_socket).expect("BUG: bind stale socket");
        drop(stale_listener);
        let counter = td.path().join(format!("{name}-counter"));

        let stream =
            connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
                .expect("socket without listener should spawn");
        drop(stream);

        assert_spawned_once(&counter);
        assert_current_record(&config);
    }
}

struct FailingLauncher;

impl HostLauncher for FailingLauncher {
    fn spawn_host(&self, _config: &Config, _release_lock_fd: i32) -> anyhow::Result<()> {
        anyhow::bail!("injected launcher failure")
    }
}

#[test]
fn record_commit_failure_spawns_nothing_and_launcher_failure_removes_claim() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "commit-failure");
    std::fs::create_dir(&config.owner_record).expect("BUG: create blocking owner directory");
    let counter = td.path().join("commit-failure-counter");
    let error = connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
        .expect_err("record commit failure must stop before spawn");
    assert!(format!("{error:#}").contains("commit bmc-wasm-host ownership record"));
    assert!(!counter.exists(), "commit failure must not invoke launcher");
    assert!(
        !config.host_socket.exists(),
        "commit failure must not create socket"
    );

    let config = make_config(&td, "spawn-failure");
    let error = connect_or_spawn_with_launcher(&config, &FailingLauncher)
        .expect_err("launcher failure must fail the widget spawn");
    assert!(format!("{error:#}").contains("injected launcher failure"));
    assert!(
        !config.owner_record.exists(),
        "launcher failure must remove committed claim"
    );
}

#[test]
fn term_timeout_escalates_to_kill_before_replacement() {
    let td = TempDir::new().expect("BUG: tempdir");
    let mut config = make_config(&td, "ignore-term");
    config.host_wait = Duration::from_millis(800);
    let foreign = start_foreign_host(&config, TermBehavior::Ignore);
    let counter = td.path().join("ignore-term-counter");

    let stream = connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
        .expect("TERM-ignoring host should be killed and replaced");
    drop(stream);

    assert_spawned_once(&counter);
    assert_current_record(&config);
    assert_eq!(
        foreign.wait().signal(),
        Some(libc::SIGKILL),
        "TERM-ignoring host should require SIGKILL"
    );
}

#[test]
fn closed_probe_from_term_ignoring_host_escalates_to_kill() {
    let td = TempDir::new().expect("BUG: tempdir");
    let mut config = make_config(&td, "ignore-term-closed-probe");
    config.host_wait = Duration::from_millis(800);
    let marker = td.path().join("err-ack-drop-marker");
    let foreign = start_foreign_host_with_env(
        &config,
        TermBehavior::Ignore,
        &[(
            "BMC_THIN_FAKE_HOST_ERR_ACK_DROP",
            marker.display().to_string(),
        )],
    );
    let counter = td.path().join("ignore-term-closed-probe-counter");

    let stream = connect_or_spawn_with_launcher(&config, &replacement_launcher(counter.clone()))
        .expect("TERM-ignoring host behind a closed probe should be killed and replaced");
    drop(stream);

    assert_spawned_once(&counter);
    assert_current_record(&config);
    assert_eq!(
        foreign.wait().signal(),
        Some(libc::SIGKILL),
        "closed control probe must not be mistaken for host exit"
    );
}

#[derive(Default)]
struct SurvivingTerminator {
    observed: Mutex<Option<(Duration, Duration)>>,
}

impl ForeignHostTerminator for SurvivingTerminator {
    fn terminate(
        &self,
        _stream: UnixStream,
        _socket: &std::path::Path,
        lifecycle_deadline: Instant,
        term_wait: Duration,
        kill_wait: Duration,
    ) -> anyhow::Result<()> {
        assert!(lifecycle_deadline > Instant::now());
        *self.observed.lock().expect("BUG: terminator observation") = Some((term_wait, kill_wait));
        anyhow::bail!("injected host survived SIGKILL")
    }
}

struct ExpiringLifecycleTerminator;

impl ForeignHostTerminator for ExpiringLifecycleTerminator {
    fn terminate(
        &self,
        stream: UnixStream,
        socket: &std::path::Path,
        lifecycle_deadline: Instant,
        term_wait: Duration,
        kill_wait: Duration,
    ) -> anyhow::Result<()> {
        SignalForeignHostTerminator.terminate(
            stream,
            socket,
            lifecycle_deadline,
            term_wait,
            kill_wait,
        )?;
        std::thread::sleep(lifecycle_deadline.saturating_duration_since(Instant::now()));
        while Instant::now() <= lifecycle_deadline {
            std::hint::spin_loop();
        }
        Ok(())
    }
}

#[test]
fn replacement_start_gets_full_wait_after_lifecycle_deadline() {
    let td = TempDir::new().expect("BUG: tempdir");
    let mut config = make_config(&td, "expired-lifecycle");
    config.host_wait = Duration::from_millis(400);
    let foreign = start_foreign_host(&config, TermBehavior::Exit);
    let counter = td.path().join("expired-lifecycle-counter");

    let stream = connect_or_spawn_with_launcher_and_terminator(
        &config,
        &replacement_launcher(counter.clone()),
        &ExpiringLifecycleTerminator,
    )
    .expect("replacement startup should receive a fresh wait budget");
    drop(stream);

    assert_spawned_once(&counter);
    assert_current_record(&config);
    assert_eq!(foreign.wait().signal(), Some(libc::SIGTERM));
}

#[test]
fn post_kill_survivor_fails_without_claim_or_spawn() {
    let td = TempDir::new().expect("BUG: tempdir");
    let mut config = make_config(&td, "survivor");
    config.host_wait = Duration::from_millis(800);
    let _foreign = start_foreign_host(&config, TermBehavior::Exit);
    let counter = td.path().join("survivor-counter");
    let terminator = SurvivingTerminator::default();

    let error = connect_or_spawn_with_launcher_and_terminator(
        &config,
        &replacement_launcher(counter.clone()),
        &terminator,
    )
    .expect_err("SIGKILL survivor must fail");

    assert!(format!("{error:#}").contains("survived SIGKILL"));
    assert_eq!(
        *terminator.observed.lock().expect("BUG: terminator result"),
        Some((Duration::from_millis(400), Duration::from_millis(200)))
    );
    assert!(!counter.exists(), "survivor must not invoke launcher");
    assert!(
        !config.owner_record.exists(),
        "survivor must not commit claim"
    );
}

#[test]
fn racing_workers_replace_foreign_host_once() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "foreign-race");
    let foreign = start_foreign_host(&config, TermBehavior::Exit);
    let counter = td.path().join("foreign-race-counter");
    let cfg1 = config.clone();
    let cfg2 = config.clone();
    let counter1 = counter.clone();
    let counter2 = counter.clone();

    let first = std::thread::spawn(move || {
        let launcher = FakeLauncher {
            extra_env: vec![("BMC_THIN_FAKE_HOST_ACCEPTS", "2".to_owned())],
            counter: Some(counter1),
        };
        connect_or_spawn_with_launcher(&cfg1, &launcher).expect("first worker should connect");
    });
    let second = std::thread::spawn(move || {
        let launcher = FakeLauncher {
            extra_env: vec![("BMC_THIN_FAKE_HOST_ACCEPTS", "2".to_owned())],
            counter: Some(counter2),
        };
        connect_or_spawn_with_launcher(&cfg2, &launcher).expect("second worker should connect");
    });
    first.join().expect("BUG: join first worker");
    second.join().expect("BUG: join second worker");

    assert_spawned_once(&counter);
    assert_current_record(&config);
    assert_eq!(foreign.wait().signal(), Some(libc::SIGTERM));
}

struct FakeLauncher {
    extra_env: Vec<(&'static str, String)>,
    counter: Option<PathBuf>,
}

impl HostLauncher for FakeLauncher {
    fn spawn_host(&self, config: &Config, release_lock_fd: i32) -> anyhow::Result<()> {
        if let Some(counter) = &self.counter {
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(counter)
                .expect("BUG: open counter file");
            IoWrite::write_all(&mut f, b"spawn\n").expect("BUG: write counter");
        }
        let mut env = vec![
            (
                "BMC_THIN_FAKE_HOST_SOCKET",
                config.host_socket.display().to_string(),
            ),
            (
                "BMC_THIN_FAKE_HOST_RELEASE_LOCK_FD",
                release_lock_fd.to_string(),
            ),
        ];
        env.extend(self.extra_env.iter().map(|(k, v)| (*k, v.clone())));
        spawn_daemon_with_env(config, release_lock_fd, &env)
    }
}

#[test]
fn missing_socket_spawn_owner_transfers_lock_fd_and_connects_after_ready() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "miss");
    let launcher = FakeLauncher {
        extra_env: vec![],
        counter: None,
    };
    let start = Instant::now();
    let stream = connect_or_spawn_with_launcher(&config, &launcher)
        .expect("BUG: connect_or_spawn must succeed");
    let elapsed = start.elapsed();
    drop(stream);
    assert!(
        elapsed >= Duration::from_millis(80),
        "connection should not happen before fake host releases the lock (elapsed={elapsed:?})",
    );
    // give the fake host a moment to exit
    std::thread::sleep(Duration::from_millis(200));
}

#[test]
fn racing_workers_start_only_one_fake_host() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "race");
    let counter = td.path().join("spawn_counter");
    let cfg1 = config.clone();
    let cfg2 = config.clone();
    let counter1 = counter.clone();
    let counter2 = counter.clone();
    let t1 = std::thread::spawn(move || {
        let launcher = FakeLauncher {
            extra_env: vec![("BMC_THIN_FAKE_HOST_ACCEPTS", "2".to_owned())],
            counter: Some(counter1),
        };
        connect_or_spawn_with_launcher(&cfg1, &launcher).expect("BUG: worker 1 connect");
    });
    let t2 = std::thread::spawn(move || {
        let launcher = FakeLauncher {
            extra_env: vec![("BMC_THIN_FAKE_HOST_ACCEPTS", "2".to_owned())],
            counter: Some(counter2),
        };
        connect_or_spawn_with_launcher(&cfg2, &launcher).expect("BUG: worker 2 connect");
    });
    t1.join().expect("BUG: worker 1 join");
    t2.join().expect("BUG: worker 2 join");
    let contents = std::fs::read_to_string(&counter).expect("BUG: read counter");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one fake host should be spawned, got: {contents:?}"
    );
    std::thread::sleep(Duration::from_millis(200));
}

#[test]
fn readiness_timeout_leaves_lockfile_path_on_disk() {
    let td = TempDir::new().expect("BUG: tempdir");
    let lockfile = td.path().join("readiness.lock");
    // pre-create the lock file and hold LOCK_EX on it
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&lockfile)
        .expect("BUG: open lock");
    rustix::fs::flock(&f, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .expect("BUG: failed to take LOCK_EX in test");
    let err = wait_for_readiness(&lockfile, Duration::from_millis(50))
        .expect_err("must time out waiting");
    let msg = format!("{err}");
    assert!(
        msg.contains("timed out"),
        "expected 'timed out' in error, got: {msg}",
    );
    assert!(
        lockfile.exists(),
        "lockfile must not be unlinked after timeout",
    );
    // release the lock by dropping
    drop(f);
}

#[test]
fn daemonized_fake_host_inherits_only_release_lock_fd() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "fd");
    let report = td.path().join("fd_report");
    // Open an extra, non-CLOEXEC fd in the parent that must NOT leak to the daemon.
    // Using libc::open without O_CLOEXEC.
    let extra_path = td.path().join("extra_file");
    let c_extra =
        std::ffi::CString::new(extra_path.as_os_str().as_encoded_bytes()).expect("BUG: cstring");
    let extra_fd = unsafe { libc::open(c_extra.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600) };
    assert!(extra_fd >= 0, "BUG: open extra fd");
    let launcher = FakeLauncher {
        extra_env: vec![("BMC_THIN_FAKE_HOST_FD_REPORT", report.display().to_string())],
        counter: None,
    };
    let stream =
        connect_or_spawn_with_launcher(&config, &launcher).expect("BUG: fd_test connect_or_spawn");
    drop(stream);
    // Wait for the report file
    let start = Instant::now();
    while !report.exists() {
        assert!(
            start.elapsed() <= Duration::from_secs(3),
            "fd report not written",
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Give it a moment to fully write
    std::thread::sleep(Duration::from_millis(50));
    let contents = std::fs::read_to_string(&report).expect("BUG: read fd report");
    let mut lock_fd: Option<i32> = None;
    let mut app_fds: Vec<i32> = Vec::new();
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("LOCK=") {
            lock_fd = Some(rest.parse().expect("BUG: parse lock fd"));
        } else if let Ok(fd) = line.parse::<i32>() {
            app_fds.push(fd);
        }
    }
    let lock = lock_fd.expect("BUG: lock fd line missing");
    let extras: Vec<i32> = app_fds.into_iter().filter(|&fd| fd != lock).collect();
    assert!(
        extras.is_empty(),
        "unexpected inherited fds above stdio: {extras:?} (lock_fd={lock})",
    );
    unsafe {
        libc::close(extra_fd);
    }
}
