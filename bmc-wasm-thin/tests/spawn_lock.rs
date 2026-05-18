// Copyright (C) 2026  Braiins Systems s.r.o.

use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bmc_wasm_thin::args::Config;
use bmc_wasm_thin::spawn::{
    HostLauncher, connect_or_spawn_with_launcher, spawn_daemon_with_env, wait_for_readiness,
};
use tempfile::TempDir;

fn make_config(td: &TempDir, name: &str) -> Config {
    let host_socket = td.path().join(format!("{name}.sock"));
    let lockfile = td.path().join(format!("{name}.lock"));
    Config {
        wasm: PathBuf::from("/dev/null"),
        host_socket,
        lockfile,
        host_bin: PathBuf::from(env!("CARGO_BIN_EXE_bmc-wasm-thin-fakehost")),
        host_wait: Duration::from_secs(5),
        ack_wait: Duration::from_secs(5),
    }
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

struct PrebindLauncher {
    spawned: Mutex<bool>,
    listener: Mutex<Option<UnixListener>>,
}

impl HostLauncher for PrebindLauncher {
    fn before_spawn_owner_reconnect(&self, config: &Config) -> anyhow::Result<()> {
        let listener = UnixListener::bind(&config.host_socket)?;
        *self.listener.lock().expect("BUG: prebind lock") = Some(listener);
        Ok(())
    }

    fn spawn_host(&self, _config: &Config, _release_lock_fd: i32) -> anyhow::Result<()> {
        *self.spawned.lock().expect("BUG: spawned lock") = true;
        Ok(())
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
fn spawn_owner_reconnects_without_spawning_if_socket_appears() {
    let td = TempDir::new().expect("BUG: tempdir");
    let config = make_config(&td, "prebind");
    let launcher = PrebindLauncher {
        spawned: Mutex::new(false),
        listener: Mutex::new(None),
    };
    let stream =
        connect_or_spawn_with_launcher(&config, &launcher).expect("BUG: prebind connect_or_spawn");
    drop(stream);
    let spawned = *launcher.spawned.lock().expect("BUG: spawned read");
    assert!(!spawned, "spawn_host must not be called when pre-bind wins");
    assert!(
        launcher
            .listener
            .lock()
            .expect("BUG: listener read")
            .is_some(),
        "before_spawn_owner_reconnect should have bound the listener",
    );
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
