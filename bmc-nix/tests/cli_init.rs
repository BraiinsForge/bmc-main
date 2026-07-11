// Copyright (C) 2026  Braiins Systems s.r.o.

//! End-to-end tests for `bmc-nix-cli init`.
//!
//! These invoke the actual compiled binary against a real loopback HTTP
//! server (a minimal `std::net::TcpListener` server, no new deps) serving
//! the package feed and tarball `init` fetches over real HTTP, with
//! `prepare_data_partition`'s shell-outs neutralized via PATH stubs (same
//! mechanism as the `nix-store` stub in `cli_operations.rs`). This covers
//! CLI dispatch, servers-config loading, real tar extraction, and durable
//! promotion together, plus the exit-code and stdout contracts the
//! firmware COMMAND trusts.
//!
//! Test 6 exercises the `--wipe` pipeline at the `bmc_nix::store::init_store`
//! library level instead of through the binary: `cmd_init --wipe` refuses
//! when the host's `/nix` is an active mount, which on NixOS developer
//! machines (and some nix build sandboxes) it is — a binary-level `--wipe`
//! test would be environment-dependent. The guard itself is a three-line
//! early-bail covered by code review; `init_store` is the wipe pipeline's
//! complete entry point.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use bmc_nix::feed::{PackageFeed, PackageFeedEntry};
use bmc_nix::types::{FactoryServerEntry, ServersConfig};
use serial_test::serial;
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_bmc-nix-cli")
}

// ── minimal loopback HTTP server ─────────────────────────────────────────

/// A single served route: exact request-path match -> response body.
struct Route {
    path: String,
    body: Vec<u8>,
}

/// A bound-but-not-yet-serving listener. Split from [`TestHttpServer`] so
/// callers can learn the kernel-assigned port (needed to build a package
/// feed whose `download_url` points back at this same server) before the
/// route table — which references that port — is known.
struct PendingServer {
    listener: TcpListener,
    addr: SocketAddr,
}

fn bind_server() -> PendingServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback listener");
    let addr = listener.local_addr().expect("BUG: read local addr");
    PendingServer { listener, addr }
}

impl PendingServer {
    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Start accepting connections on a background thread and serving
    /// `routes`. `init` performs exactly two sequential GETs, so handling
    /// one connection at a time is sufficient.
    fn serve(self, routes: Vec<Route>) -> TestHttpServer {
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let listener = self.listener;

        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                // Checked immediately after accept() returns, before any
                // read: the drop-time nudge connection below never sends a
                // request, so reading first here would block forever and
                // deadlock `join()`.
                if thread_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else {
                    continue;
                };
                serve_one(&stream, &routes);
            }
        });

        TestHttpServer {
            addr: self.addr,
            shutdown,
            handle: Some(handle),
        }
    }
}

/// Threaded HTTP/1.1 server for exercising `init`'s real GETs against the
/// real binary. Dropping it shuts the accept-loop thread down cleanly:
/// set the flag, nudge the blocked `accept()` with a bare connect-and-close
/// (never read by the server thread), then join.
struct TestHttpServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for TestHttpServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("BUG: HTTP server thread panicked");
        }
    }
}

/// Read the request line and headers (the body, if any, is never read —
/// `init` only ever sends GETs), then respond with the matching route's
/// body or a 404.
fn serve_one(stream: &TcpStream, routes: &[Route]) {
    let mut reader = BufReader::new(stream.try_clone().expect("BUG: clone stream for reading"));
    let mut request_line = String::new();
    if reader
        .read_line(&mut request_line)
        .expect("BUG: read request line")
        == 0
    {
        return;
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .expect("BUG: malformed request line")
        .to_owned();

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("BUG: read header line");
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut stream = reader.into_inner();
    match routes.iter().find(|route| route.path == path) {
        Some(route) => write_response(&mut stream, 200, "OK", &route.body),
        None => write_response(&mut stream, 404, "Not Found", b"not found"),
    }
}

fn write_response(stream: &mut TcpStream, status: u16, reason: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .expect("BUG: write response header");
    stream.write_all(body).expect("BUG: write response body");
}

// ── PATH stubs for `prepare_data_partition` ──────────────────────────────

/// Neutralize `prepare_data_partition`'s shell-outs so the rest of the
/// init pipeline runs for real against a tempdir: `test` pretends the
/// dummy `--data-partition` is a block device, `blkid` reports an existing
/// ext4 filesystem so `mkfs.ext4` is never reached, `e2fsck` and `mount`
/// report success. The real `tar` (needed by tarball extraction) still
/// resolves from the real PATH tail. The real `blkid`/`mkfs`/`mount`/
/// `e2fsck` decision logic is covered by `partition.rs`'s unit tests; these
/// stubs only neutralize the environment.
fn write_partition_stubs(stub_dir: &Path) {
    let scripts: &[(&str, &str)] = &[
        ("test", "#!/bin/sh\nexit 0\n"),
        ("blkid", "#!/bin/sh\necho ext4\nexit 0\n"),
        ("e2fsck", "#!/bin/sh\nexit 0\n"),
        ("mount", "#!/bin/sh\nexit 0\n"),
    ];
    for (name, script) in scripts {
        let path = stub_dir.join(name);
        std::fs::write(&path, script).expect("BUG: write stub script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: chmod stub script");
    }
}

// ── fixtures ──────────────────────────────────────────────────────────────

struct CliRun {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

struct InitEnv {
    /// Kept alive for the test's duration (RAII cleanup of the fixture
    /// tree); also read directly by tests that build tarball fixtures.
    tmp: TempDir,
    data_dir: PathBuf,
    download_dir: PathBuf,
    data_partition: PathBuf,
    servers_config: PathBuf,
    bos_version_file: PathBuf,
    path_env: String,
}

fn setup() -> InitEnv {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let data_dir = tmp.path().join("data");
    let download_dir = tmp.path().join("download");
    std::fs::create_dir_all(&data_dir).expect("BUG: mkdir data");
    std::fs::create_dir_all(&download_dir).expect("BUG: mkdir download");

    let stub_dir = tmp.path().join("stub-bin");
    std::fs::create_dir_all(&stub_dir).expect("BUG: mkdir stub-bin");
    write_partition_stubs(&stub_dir);
    let real_path = std::env::var("PATH").expect("BUG: PATH is set");
    let path_env = format!("{}:{real_path}", stub_dir.display());

    InitEnv {
        data_partition: tmp.path().join("dummy-partition"),
        servers_config: tmp.path().join("servers.json"),
        bos_version_file: tmp.path().join("bos_version"),
        data_dir,
        download_dir,
        tmp,
        path_env,
    }
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("BUG: non-utf8 test path").to_owned()
}

impl InitEnv {
    fn write_servers_config(&self, base_url: &str) {
        let config = ServersConfig {
            factory: FactoryServerEntry {
                id: "test".to_owned(),
                base_url: base_url.to_owned(),
                known_public_key: String::new(),
                priority: 0,
                enabled: true,
            },
            servers: Vec::new(),
            bootstrapped_factory: false,
        };
        let json = serde_json::to_string(&config).expect("BUG: serialize servers config");
        std::fs::write(&self.servers_config, json).expect("BUG: write servers.json");
    }

    fn write_bos_version(&self, version: &str) {
        std::fs::write(&self.bos_version_file, version).expect("BUG: write bos_version");
    }

    fn run(&self, args: &[String]) -> CliRun {
        let output = Command::new(bin())
            .args(args)
            .env("PATH", &self.path_env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("BUG: failed to spawn bmc-nix-cli");
        CliRun {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Invoke `init`, passing all five flags explicitly (never relying on
    /// the on-device defaults). `extra` appends further flags (e.g. `--wipe`).
    fn run_init(&self, extra: &[&str]) -> CliRun {
        let mut args = vec![
            "init".to_owned(),
            "--servers-config".to_owned(),
            path_arg(&self.servers_config),
            "--data-partition".to_owned(),
            path_arg(&self.data_partition),
            "--data-dir".to_owned(),
            path_arg(&self.data_dir),
            "--bos-version-file".to_owned(),
            path_arg(&self.bos_version_file),
            "--download-dir".to_owned(),
            path_arg(&self.download_dir),
        ];
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        self.run(&args)
    }

    fn run_is_initialized(&self) -> CliRun {
        self.run(&[
            "is-initialized".to_owned(),
            "--data-dir".to_owned(),
            path_arg(&self.data_dir),
        ])
    }
}

/// Build a `tar czf` fixture from `entries` under `root` (kept outside
/// `download_dir` so the happy-path cleanup assertion on `download/` can
/// never be confused by fixture files) and return its bytes, read into
/// memory before the CLI runs.
fn build_tarball(root: &Path, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let source = root.join("fixture-root");
    std::fs::create_dir_all(&source).expect("BUG: mkdir fixture-root");
    for (relative, contents) in entries {
        let path = source.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("BUG: mkdir fixture parent");
        }
        std::fs::write(&path, contents).expect("BUG: write fixture entry");
    }

    let tarball_path = root.join("fixture.tar.gz");
    let status = Command::new("tar")
        .arg("czf")
        .arg(&tarball_path)
        .arg("-C")
        .arg(&source)
        .arg(".")
        .status()
        .expect("BUG: spawn tar");
    assert!(status.success(), "BUG: tarball fixture creation failed");
    std::fs::read(&tarball_path).expect("BUG: read tarball fixture bytes")
}

fn package_feed_bytes(entries: Vec<PackageFeedEntry>) -> Vec<u8> {
    serde_json::to_vec(&PackageFeed {
        version: 1,
        entries,
    })
    .expect("BUG: serialize package feed")
}

const PROFILE_PATH: &str = "/nix/var/nix/gcroots/profiles/bmc";

// ── binary-level tests ───────────────────────────────────────────────────

#[test]
#[serial]
fn init_downloads_extracts_and_promotes() {
    let env = setup();
    let bos_version = "2026-test-1";

    let pending = bind_server();
    let base_url = pending.base_url();
    let tarball_route_path = format!("/nix-{bos_version}.tar.gz");
    let tarball_bytes = build_tarball(env.tmp.path(), &[("nix/store/testpkg/marker", b"hello")]);
    let feed_bytes = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: bos_version.to_owned(),
        download_url: format!("{base_url}{tarball_route_path}"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
    }]);
    let _server = pending.serve(vec![
        Route {
            path: "/nix-package-feed.v1.json".to_owned(),
            body: feed_bytes,
        },
        Route {
            path: tarball_route_path,
            body: tarball_bytes,
        },
    ]);

    env.write_servers_config(&base_url);
    env.write_bos_version(bos_version);

    let run = env.run_init(&[]);

    assert!(
        run.status.success(),
        "expected exit 0, got {:?}. stderr:\n{}",
        run.status.code(),
        run.stderr,
    );
    assert_eq!(run.stdout, format!("{PROFILE_PATH}\n"));
    assert!(
        env.data_dir.join("nix/store/testpkg/marker").exists(),
        "extracted store content should be promoted under data/nix"
    );
    assert!(
        !env.data_dir.join("nix.tmp").exists(),
        "staging directory must not survive a successful init"
    );
    assert!(
        !env.download_dir.join("init-tarball.tar.gz").exists(),
        "the downloaded tarball must be removed on success"
    );
}

#[test]
#[serial]
fn init_is_noop_when_already_initialized() {
    let env = setup();
    let marker = env.data_dir.join("nix/store/existing/marker");
    std::fs::create_dir_all(marker.parent().expect("BUG: marker has a parent"))
        .expect("BUG: mkdir pre-existing store");
    std::fs::write(&marker, "pre-existing").expect("BUG: write pre-existing marker");

    // Unreachable (connection refused) address: the no-op path must not
    // touch the network at all.
    env.write_servers_config("http://127.0.0.1:1");
    env.write_bos_version("irrelevant");

    let run = env.run_init(&[]);

    assert!(
        run.status.success(),
        "expected exit 0, got {:?}. stderr:\n{}",
        run.status.code(),
        run.stderr,
    );
    assert_eq!(
        run.stdout, "",
        "no-op init must print nothing on stdout (the fresh-init/no-op signal)"
    );
    assert_eq!(
        std::fs::read_to_string(&marker).expect("BUG: read marker"),
        "pre-existing",
        "the pre-existing store must be untouched"
    );
}

#[test]
#[serial]
fn init_rejects_tarball_without_nix_subtree() {
    let env = setup();
    let bos_version = "2026-test-1";

    let pending = bind_server();
    let base_url = pending.base_url();
    let tarball_route_path = format!("/nix-{bos_version}.tar.gz");
    let tarball_bytes = build_tarball(env.tmp.path(), &[("etc/marker", b"not a store")]);
    let feed_bytes = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: bos_version.to_owned(),
        download_url: format!("{base_url}{tarball_route_path}"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
    }]);
    let _server = pending.serve(vec![
        Route {
            path: "/nix-package-feed.v1.json".to_owned(),
            body: feed_bytes,
        },
        Route {
            path: tarball_route_path,
            body: tarball_bytes,
        },
    ]);

    env.write_servers_config(&base_url);
    env.write_bos_version(bos_version);

    let run = env.run_init(&[]);

    assert!(
        !run.status.success(),
        "a tarball without a nix subtree must not exit 0"
    );
    assert!(
        run.stderr.contains("nix subtree"),
        "stderr should mention the missing nix subtree: {}",
        run.stderr,
    );
    assert!(
        !env.data_dir.join("nix").exists(),
        "a rejected tarball must never be promoted"
    );

    let check = env.run_is_initialized();
    assert!(
        !check.status.success(),
        "is-initialized must report not-initialized after a rejected init"
    );
}

#[test]
#[serial]
fn init_collects_stale_leftovers() {
    let env = setup();
    let bos_version = "2026-test-1";

    std::fs::create_dir_all(env.data_dir.join("nix.tmp/garbage"))
        .expect("BUG: seed stale nix.tmp leftover");
    std::fs::create_dir_all(env.data_dir.join("nix.wiped/store/old"))
        .expect("BUG: seed stale nix.wiped leftover");

    let pending = bind_server();
    let base_url = pending.base_url();
    let tarball_route_path = format!("/nix-{bos_version}.tar.gz");
    let tarball_bytes = build_tarball(env.tmp.path(), &[("nix/store/freshpkg/marker", b"fresh")]);
    let feed_bytes = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: bos_version.to_owned(),
        download_url: format!("{base_url}{tarball_route_path}"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
    }]);
    let _server = pending.serve(vec![
        Route {
            path: "/nix-package-feed.v1.json".to_owned(),
            body: feed_bytes,
        },
        Route {
            path: tarball_route_path,
            body: tarball_bytes,
        },
    ]);

    env.write_servers_config(&base_url);
    env.write_bos_version(bos_version);

    let run = env.run_init(&[]);

    assert!(
        run.status.success(),
        "expected exit 0, got {:?}. stderr:\n{}",
        run.status.code(),
        run.stderr,
    );
    assert_eq!(run.stdout, format!("{PROFILE_PATH}\n"));
    assert!(
        !env.data_dir.join("nix.tmp").exists(),
        "stale nix.tmp leftover must be collected"
    );
    assert!(
        !env.data_dir.join("nix.wiped").exists(),
        "stale nix.wiped leftover must be collected"
    );
    assert!(
        env.data_dir.join("nix/store/freshpkg/marker").exists(),
        "a fresh store must be promoted alongside leftover cleanup"
    );
}

#[test]
#[serial]
fn init_fails_when_bos_version_not_in_index() {
    let env = setup();
    let indexed_version = "2026-test-1";
    let requested_version = "2026-does-not-exist";

    let pending = bind_server();
    let base_url = pending.base_url();
    let feed_bytes = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: indexed_version.to_owned(),
        download_url: format!("{base_url}/nix-{indexed_version}.tar.gz"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
    }]);
    let _server = pending.serve(vec![Route {
        path: "/nix-package-feed.v1.json".to_owned(),
        body: feed_bytes,
    }]);

    env.write_servers_config(&base_url);
    env.write_bos_version(requested_version);

    let run = env.run_init(&[]);

    assert!(
        !run.status.success(),
        "an unindexed BOS version must not exit 0"
    );
    assert!(
        run.stderr.contains("no package feed entry for BOS version"),
        "stderr should name the missing-version error: {}",
        run.stderr,
    );
    assert!(
        run.stderr.contains(requested_version),
        "stderr should name the requested version: {}",
        run.stderr,
    );
    assert!(
        !env.data_dir.join("nix").exists(),
        "nothing must be promoted when the BOS version is unindexed"
    );
}

// ── library-level test ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn init_store_wipe_replaces_existing_store() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let stage_dir = tmp.path().join("stage");
    let download_dir = tmp.path().join("download");
    std::fs::create_dir_all(stage_dir.join("nix/store/old")).expect("BUG: seed pre-existing store");
    std::fs::write(stage_dir.join("nix/store/old/marker"), "old")
        .expect("BUG: write pre-existing marker");

    let bos_version = "2026-test-1";
    let pending = bind_server();
    let base_url = pending.base_url();
    let tarball_route_path = format!("/nix-{bos_version}.tar.gz");
    let tarball_bytes = build_tarball(tmp.path(), &[("nix/store/newpkg/marker", b"new")]);
    let feed_bytes = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: bos_version.to_owned(),
        download_url: format!("{base_url}{tarball_route_path}"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
    }]);
    let _server = pending.serve(vec![
        Route {
            path: "/nix-package-feed.v1.json".to_owned(),
            body: feed_bytes,
        },
        Route {
            path: tarball_route_path,
            body: tarball_bytes,
        },
    ]);

    let factory_server = FactoryServerEntry {
        id: "test".to_owned(),
        base_url,
        known_public_key: String::new(),
        priority: 0,
        enabled: true,
    };
    let client = reqwest::Client::new();

    let result = bmc_nix::store::init_store(
        &client,
        &factory_server,
        bos_version,
        &download_dir,
        &stage_dir,
        true,
        None,
    )
    .await
    .expect("BUG: wipe-and-replace init_store should succeed");

    assert_eq!(result.profile_path, PathBuf::from(PROFILE_PATH));
    assert!(
        stage_dir.join("nix/store/newpkg/marker").exists(),
        "the new store must be promoted"
    );
    assert!(
        !stage_dir.join("nix/store/old").exists(),
        "the old store content must be gone"
    );
    assert!(
        !stage_dir.join("nix.wiped").exists(),
        "the demoted store must be fully collected, not just renamed"
    );
}
