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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;

use bmc_nix::feed::{PackageFeed, PackageFeedEntry};
use bmc_nix::store::{InitStoreError, SignatureVerification};
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
        let hits = Arc::new(AtomicUsize::new(0));
        let thread_hits = Arc::clone(&hits);
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
                serve_one(&stream, &routes, &thread_hits);
            }
        });

        TestHttpServer {
            addr: self.addr,
            shutdown,
            hits,
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
    hits: Arc<AtomicUsize>,
    handle: Option<JoinHandle<()>>,
}

impl TestHttpServer {
    /// Number of requests served so far (the drop-time nudge connection
    /// sends no request line and is never counted).
    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
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
fn serve_one(stream: &TcpStream, routes: &[Route], hits: &AtomicUsize) {
    let mut reader = BufReader::new(stream.try_clone().expect("BUG: clone stream for reading"));
    let mut request_line = String::new();
    if reader
        .read_line(&mut request_line)
        .expect("BUG: read request line")
        == 0
    {
        return;
    }
    hits.fetch_add(1, Ordering::SeqCst);
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
    /// Register the factory server with the shared test keypair's
    /// public key as the trust anchor — the default for binary-level
    /// tests, which exercise `init`'s verification-on default path.
    fn write_servers_config(&self, base_url: &str) {
        let (_, public) = test_init_keypair();
        self.write_servers_config_with_key(base_url, &public);
    }

    fn write_servers_config_with_key(&self, base_url: &str, known_public_key: &str) {
        let config = ServersConfig {
            factory: FactoryServerEntry {
                id: "test".to_owned(),
                base_url: base_url.to_owned(),
                known_public_key: known_public_key.to_owned(),
                priority: 0,
                enabled: true,
            },
            servers: Vec::new(),
            bootstrapped_factory: false,
        };
        let json = serde_json::to_string(&config).expect("BUG: serialize servers config");
        std::fs::write(&self.servers_config, json).expect("BUG: write servers.json");
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
    fn run_init(&self, firmware: &str, extra: &[&str]) -> CliRun {
        let mut args = vec![
            "init".to_owned(),
            "--servers-config".to_owned(),
            path_arg(&self.servers_config),
            "--data-partition".to_owned(),
            path_arg(&self.data_partition),
            "--data-dir".to_owned(),
            path_arg(&self.data_dir),
            "--firmware".to_owned(),
            firmware.to_owned(),
            "--download-dir".to_owned(),
            path_arg(&self.download_dir),
        ];
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        self.run(&args)
    }

    /// Invoke `init --tarball --profile-path`, passing only the two
    /// partition flags plus the direct-path pair (the feed-only flags
    /// conflict with --tarball).
    fn run_init_tarball(&self, tarball: &Path, profile_path: &str) -> CliRun {
        self.run(&[
            "init".to_owned(),
            "--data-partition".to_owned(),
            path_arg(&self.data_partition),
            "--data-dir".to_owned(),
            path_arg(&self.data_dir),
            "--tarball".to_owned(),
            path_arg(tarball),
            "--profile-path".to_owned(),
            profile_path.to_owned(),
        ])
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
        signature: Some(sign_with_test_key(&tarball_bytes)),
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
    let run = env.run_init(bos_version, &[]);

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

    // A promoted store only counts as initialized with its Nix database
    // and the BMC profile gcroot alongside the store paths.
    let database = env.data_dir.join("nix/var/nix/db/db.sqlite");
    std::fs::create_dir_all(database.parent().expect("BUG: database has a parent"))
        .expect("BUG: mkdir nix database dir");
    std::fs::write(&database, "").expect("BUG: write pre-existing database");
    std::fs::create_dir_all(env.data_dir.join("nix/var/nix/gcroots/profiles/bmc"))
        .expect("BUG: mkdir bmc profile gcroot");

    // Unreachable (connection refused) address: the no-op path must not
    // touch the network at all.
    env.write_servers_config("http://127.0.0.1:1");
    let run = env.run_init("irrelevant", &[]);

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
        // Correctly signed: the missing-nix-subtree rejection under
        // test happens after verification.
        signature: Some(sign_with_test_key(&tarball_bytes)),
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
    let run = env.run_init(bos_version, &[]);

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
        signature: Some(sign_with_test_key(&tarball_bytes)),
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
    let run = env.run_init(bos_version, &[]);

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
        signature: None,
    }]);
    let _server = pending.serve(vec![Route {
        path: "/nix-package-feed.v1.json".to_owned(),
        body: feed_bytes,
    }]);

    env.write_servers_config(&base_url);
    let run = env.run_init(requested_version, &[]);

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

/// The production default: no flag passed, an unsigned feed entry must
/// fail before the tarball is ever requested.
#[test]
#[serial]
fn init_rejects_unsigned_feed_entry_by_default() {
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
        signature: None,
    }]);
    let server = pending.serve(vec![
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
    let run = env.run_init(bos_version, &[]);

    assert_eq!(
        run.status.code(),
        Some(2),
        "an unsigned feed entry must fail as a runtime error. stderr:\n{}",
        run.stderr,
    );
    assert!(
        run.stderr.contains("has no signature"),
        "stderr should name the missing signature: {}",
        run.stderr,
    );
    assert_eq!(
        server.hits(),
        1,
        "only the feed may have been fetched: the missing signature must \
         abort before the tarball download"
    );
    assert!(
        !env.data_dir.join("nix").exists(),
        "nothing must be promoted from an unsigned feed entry"
    );
}

#[test]
#[serial]
fn init_rejects_signature_by_untrusted_key_by_default() {
    let env = setup();
    let bos_version = "2026-test-1";
    let (untrusted_secret, _) = signing_keypair("braiins-init-1", &[9; 32]);

    let pending = bind_server();
    let base_url = pending.base_url();
    let tarball_route_path = format!("/nix-{bos_version}.tar.gz");
    let tarball_bytes = build_tarball(env.tmp.path(), &[("nix/store/testpkg/marker", b"hello")]);
    let feed_bytes = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: bos_version.to_owned(),
        download_url: format!("{base_url}{tarball_route_path}"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
        signature: Some(
            bmc_nix::signature::sign(&untrusted_secret, &sha256(&tarball_bytes))
                .expect("BUG: valid secret key"),
        ),
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
    let run = env.run_init(bos_version, &[]);

    assert_eq!(
        run.status.code(),
        Some(2),
        "a signature by an untrusted key must fail as a runtime error. stderr:\n{}",
        run.stderr,
    );
    assert!(
        run.stderr
            .contains("init tarball signature verification failed"),
        "stderr should name the verification failure: {}",
        run.stderr,
    );
    assert!(
        !env.data_dir.join("nix").exists(),
        "a tarball signed by an untrusted key must never be promoted"
    );
    assert!(
        !env.download_dir.join("init-tarball.tar.gz").exists(),
        "the rejected tarball must not linger at the download path"
    );
}

/// The development escape hatch: `--no-verify-signature` accepts an
/// unsigned feed entry even with no usable trust anchor configured,
/// and says loudly that verification is off.
#[test]
#[serial]
fn init_no_verify_signature_skips_verification_with_warning() {
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
        signature: None,
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

    env.write_servers_config_with_key(&base_url, "");
    let run = env.run_init(bos_version, &["--no-verify-signature"]);

    assert!(
        run.status.success(),
        "expected exit 0, got {:?}. stderr:\n{}",
        run.status.code(),
        run.stderr,
    );
    assert_eq!(run.stdout, format!("{PROFILE_PATH}\n"));
    assert!(
        run.stderr.contains("signature verification disabled"),
        "stderr should warn that verification is off: {}",
        run.stderr,
    );
    assert!(
        env.data_dir.join("nix/store/testpkg/marker").exists(),
        "the unverified store must still be promoted"
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
        signature: None,
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
        &SignatureVerification::Disabled,
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

/// The network init path must acquire `<stage_dir>/.init.lock` before
/// fetching the feed or creating the fixed-name download. Once released,
/// the feed request must proceed.
#[tokio::test]
#[serial]
async fn init_store_blocks_on_held_init_lock() {
    use std::os::fd::AsRawFd;

    let tmp = TempDir::new().expect("BUG: tempdir");
    let stage_dir = tmp.path().join("stage");
    let download_dir = tmp.path().join("download");
    std::fs::create_dir_all(&stage_dir).expect("BUG: mkdir stage");

    let bos_version = "2026-test-1";
    let pending = bind_server();
    let base_url = pending.base_url();
    let feed_without_matching_version = package_feed_bytes(vec![]);
    let server = pending.serve(vec![Route {
        path: "/nix-package-feed.v1.json".to_owned(),
        body: feed_without_matching_version,
    }]);

    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(stage_dir.join(".init.lock"))
        .expect("BUG: open .init.lock");
    // SAFETY: `lock_file` is a valid open descriptor for the whole call.
    let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
    assert_eq!(ret, 0, "BUG: test could not take the init lock");

    let factory_server = FactoryServerEntry {
        id: "test".to_owned(),
        base_url,
        known_public_key: String::new(),
        priority: 0,
        enabled: true,
    };
    let task_download_dir = download_dir.clone();
    let task_stage_dir = stage_dir.clone();
    let task = tokio::spawn(async move {
        let client = reqwest::Client::new();
        bmc_nix::store::init_store(
            &client,
            &factory_server,
            bos_version,
            &task_download_dir,
            &task_stage_dir,
            false,
            &SignatureVerification::Disabled,
            None,
        )
        .await
    });

    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            server.hits(),
            0,
            "a lock-blocked init must not have fetched anything"
        );
        assert!(
            !download_dir.join("init-tarball.tar.gz").exists(),
            "a lock-blocked init must not have created the download"
        );
    }
    assert!(
        !task.is_finished(),
        "init must block while the lock is held"
    );

    // SAFETY: same still-open descriptor as above.
    let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_UN) };
    assert_eq!(ret, 0, "BUG: test could not release the init lock");

    let err = tokio::time::timeout(std::time::Duration::from_secs(30), task)
        .await
        .expect("init must fetch the feed once the lock is released")
        .expect("BUG: init task panicked")
        .expect_err("init should reach feed selection once the lock is released");
    assert!(
        matches!(
            &err,
            InitStoreError::MissingPackageFeedEntry(version) if version == bos_version
        ),
        "init must stop at the deliberately missing feed entry: {err}"
    );
    assert_eq!(server.hits(), 1, "init must fetch the feed after unlocking");
}

// ── init tarball signature verification (library level) ─────────────────

fn sha256(bytes: &[u8]) -> [u8; 32] {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .try_into()
        .expect("BUG: SHA-256 digest is 32 bytes")
}

/// The keypair behind [`InitEnv::write_servers_config`]'s trust anchor
/// and [`sign_with_test_key`].
fn test_init_keypair() -> (String, String) {
    signing_keypair("braiins-init-1", &[7; 32])
}

/// Feed-entry signature over `tarball_bytes` by the shared test key.
fn sign_with_test_key(tarball_bytes: &[u8]) -> String {
    let (secret, _) = test_init_keypair();
    bmc_nix::signature::sign(&secret, &sha256(tarball_bytes)).expect("BUG: valid secret key")
}

/// Deterministic Ed25519 keypair in the nix line formats:
/// (`name:base64(seed ‖ public)`, `name:base64(public)`).
fn signing_keypair(name: &str, seed: &[u8; 32]) -> (String, String) {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use ring::signature::KeyPair as _;

    let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed)
        .expect("BUG: any 32-byte seed is a valid Ed25519 seed");
    let public = pair.public_key().as_ref().to_vec();
    let mut secret = seed.to_vec();
    secret.extend_from_slice(&public);
    (
        format!("{name}:{}", BASE64.encode(&secret)),
        format!("{name}:{}", BASE64.encode(&public)),
    )
}

/// Serve a feed+tarball pair for `bos_version`; the feed entry's
/// signature is whatever `make_signature` derives from the exact bytes
/// the tarball route will serve.
fn serve_init_routes(
    tmp: &TempDir,
    bos_version: &str,
    make_signature: impl FnOnce(&[u8]) -> Option<String>,
) -> (TestHttpServer, String) {
    let pending = bind_server();
    let base_url = pending.base_url();
    let tarball_route_path = format!("/nix-{bos_version}.tar.gz");
    let tarball_bytes = build_tarball(tmp.path(), &[("nix/store/signedpkg/marker", b"signed")]);
    let feed_bytes = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: bos_version.to_owned(),
        download_url: format!("{base_url}{tarball_route_path}"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
        signature: make_signature(&tarball_bytes),
    }]);
    let server = pending.serve(vec![
        Route {
            path: "/nix-package-feed.v1.json".to_owned(),
            body: feed_bytes,
        },
        Route {
            path: tarball_route_path,
            body: tarball_bytes,
        },
    ]);
    (server, base_url)
}

async fn run_init_store(
    tmp: &TempDir,
    base_url: String,
    bos_version: &str,
    verification: &SignatureVerification,
) -> Result<bmc_nix::store::InitStoreResult, bmc_nix::store::InitStoreError> {
    let factory_server = FactoryServerEntry {
        id: "test".to_owned(),
        base_url,
        known_public_key: String::new(),
        priority: 0,
        enabled: true,
    };
    bmc_nix::store::init_store(
        &reqwest::Client::new(),
        &factory_server,
        bos_version,
        &tmp.path().join("download"),
        &tmp.path().join("stage"),
        false,
        verification,
        None,
    )
    .await
}

#[tokio::test]
#[serial]
async fn init_store_accepts_correctly_signed_tarball() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let bos_version = "2026-test-1";
    let (secret, public) = signing_keypair("braiins-init-1", &[7; 32]);
    let (_server, base_url) = serve_init_routes(&tmp, bos_version, |bytes| {
        Some(bmc_nix::signature::sign(&secret, &sha256(bytes)).expect("BUG: valid secret key"))
    });

    let result = run_init_store(
        &tmp,
        base_url,
        bos_version,
        &SignatureVerification::Enabled {
            trusted_public_key: public,
        },
    )
    .await
    .expect("a correctly signed tarball must initialize");

    assert_eq!(result.profile_path, PathBuf::from(PROFILE_PATH));
    assert!(
        tmp.path().join("stage/nix/store/signedpkg/marker").exists(),
        "the verified store must be promoted"
    );
}

#[tokio::test]
#[serial]
async fn init_store_rejects_signature_over_different_content() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let bos_version = "2026-test-1";
    let (secret, public) = signing_keypair("braiins-init-1", &[7; 32]);
    let (_server, base_url) = serve_init_routes(&tmp, bos_version, |_| {
        Some(
            bmc_nix::signature::sign(&secret, &sha256(b"not the served tarball"))
                .expect("BUG: valid secret key"),
        )
    });

    let err = run_init_store(
        &tmp,
        base_url,
        bos_version,
        &SignatureVerification::Enabled {
            trusted_public_key: public,
        },
    )
    .await
    .expect_err("a signature over different content must be rejected");

    assert!(
        matches!(
            &err,
            bmc_nix::store::InitStoreError::SignatureVerificationFailed {
                source: bmc_nix::signature::SignatureError::VerificationFailed { .. },
            }
        ),
        "expected SignatureVerificationFailed, got: {err:?}"
    );
    assert!(
        !tmp.path().join("download/init-tarball.tar.gz").exists(),
        "the rejected tarball must not linger at the download path"
    );
    assert!(
        !tmp.path().join("stage/nix").exists(),
        "a rejected tarball must never be promoted"
    );
}

#[tokio::test]
#[serial]
async fn init_store_rejects_signature_by_untrusted_key() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let bos_version = "2026-test-1";
    let (untrusted_secret, _) = signing_keypair("braiins-init-1", &[9; 32]);
    let (_, trusted_public) = signing_keypair("braiins-init-1", &[7; 32]);
    let (_server, base_url) = serve_init_routes(&tmp, bos_version, |bytes| {
        Some(
            bmc_nix::signature::sign(&untrusted_secret, &sha256(bytes))
                .expect("BUG: valid secret key"),
        )
    });

    let err = run_init_store(
        &tmp,
        base_url,
        bos_version,
        &SignatureVerification::Enabled {
            trusted_public_key: trusted_public,
        },
    )
    .await
    .expect_err("a signature by an untrusted key must be rejected");

    assert!(
        matches!(
            &err,
            bmc_nix::store::InitStoreError::SignatureVerificationFailed {
                source: bmc_nix::signature::SignatureError::VerificationFailed { .. },
            }
        ),
        "expected SignatureVerificationFailed, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn init_store_requires_signature_before_download() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let bos_version = "2026-test-1";
    let (_, public) = signing_keypair("braiins-init-1", &[7; 32]);
    let (server, base_url) = serve_init_routes(&tmp, bos_version, |_| None);

    let err = run_init_store(
        &tmp,
        base_url,
        bos_version,
        &SignatureVerification::Enabled {
            trusted_public_key: public,
        },
    )
    .await
    .expect_err("an unsigned feed entry must be rejected");

    assert!(
        matches!(
            &err,
            bmc_nix::store::InitStoreError::MissingSignature(version) if version == bos_version
        ),
        "expected MissingSignature, got: {err:?}"
    );
    assert_eq!(
        server.hits(),
        1,
        "only the feed may have been fetched: the missing signature must \
         abort before the tarball download"
    );
}

#[tokio::test]
#[serial]
async fn init_store_rejects_malformed_trusted_key_before_download() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let bos_version = "2026-test-1";
    let (secret, _) = signing_keypair("braiins-init-1", &[7; 32]);
    let (server, base_url) = serve_init_routes(&tmp, bos_version, |bytes| {
        Some(bmc_nix::signature::sign(&secret, &sha256(bytes)).expect("BUG: valid secret key"))
    });

    let err = run_init_store(
        &tmp,
        base_url,
        bos_version,
        &SignatureVerification::Enabled {
            trusted_public_key: "not a nix-format key".to_owned(),
        },
    )
    .await
    .expect_err("a malformed trusted key must be rejected");

    assert!(
        matches!(
            &err,
            bmc_nix::store::InitStoreError::SignatureVerificationFailed {
                source: bmc_nix::signature::SignatureError::MalformedPublicKey(_),
            }
        ),
        "expected MalformedPublicKey, got: {err:?}"
    );
    assert_eq!(
        server.hits(),
        1,
        "only the feed may have been fetched: the malformed trust anchor \
         must abort before the tarball download"
    );
}

#[tokio::test]
#[serial]
async fn init_store_from_tarball_promotes_and_keeps_source() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let stage_dir = tmp.path().join("stage");
    let tarball_bytes = build_tarball(tmp.path(), &[("nix/store/localpkg/marker", b"local")]);
    let tarball = tmp.path().join("local-init.tar.gz");
    std::fs::write(&tarball, tarball_bytes).expect("BUG: write local tarball");

    let result = bmc_nix::store::init_store_from_tarball(
        &tarball,
        Path::new(PROFILE_PATH),
        &stage_dir,
        false,
    )
    .await
    .expect("direct-tarball init should succeed");

    assert_eq!(result.profile_path, PathBuf::from(PROFILE_PATH));
    assert!(
        stage_dir.join("nix/store/localpkg/marker").exists(),
        "the local tarball's store must be promoted"
    );
    assert!(
        tarball.exists(),
        "the caller-provided tarball must not be deleted"
    );
    assert!(
        !stage_dir.join("nix.tmp").exists(),
        "staging must not survive a successful init"
    );
}

#[tokio::test]
#[serial]
async fn init_store_from_tarball_missing_file_fails_before_staging() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let stage_dir = tmp.path().join("stage");
    let missing = tmp.path().join("no-such.tar.gz");

    let err = bmc_nix::store::init_store_from_tarball(
        &missing,
        Path::new(PROFILE_PATH),
        &stage_dir,
        false,
    )
    .await
    .expect_err("a missing tarball must fail");

    // The variant exists to preserve the offending path and I/O cause;
    // assert those, not just the rendered text.
    let bmc_nix::store::InitStoreError::TarballUnavailable { path, source } = err else {
        panic!("expected TarballUnavailable, got: {err}");
    };
    assert!(
        path.ends_with("no-such.tar.gz"),
        "error must carry the offending path: {path}"
    );
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    assert!(
        !stage_dir.join("nix").exists() && !stage_dir.join("nix.tmp").exists(),
        "nothing may be promoted or staged for a missing tarball"
    );
}

#[test]
#[serial]
fn init_from_tarball_promotes_and_prints_profile_path() {
    let env = setup();
    let tarball_bytes = build_tarball(env.tmp.path(), &[("nix/store/directpkg/marker", b"direct")]);
    let tarball = env.tmp.path().join("direct-init.tar.gz");
    std::fs::write(&tarball, tarball_bytes).expect("BUG: write direct tarball");

    let run = env.run_init_tarball(&tarball, PROFILE_PATH);

    assert!(
        run.status.success(),
        "expected exit 0, got {:?}. stderr:\n{}",
        run.status.code(),
        run.stderr,
    );
    assert_eq!(run.stdout, format!("{PROFILE_PATH}\n"));
    assert!(
        env.data_dir.join("nix/store/directpkg/marker").exists(),
        "the direct tarball's store must be promoted"
    );
    assert!(tarball.exists(), "the source tarball must be kept");
}

#[test]
#[serial]
fn init_from_missing_tarball_fails_cleanly() {
    let env = setup();
    let missing = env.tmp.path().join("no-such.tar.gz");

    let run = env.run_init_tarball(&missing, PROFILE_PATH);

    assert!(!run.status.success(), "a missing tarball must not exit 0");
    assert!(
        run.stderr.contains("unavailable"),
        "stderr should name the unavailable tarball: {}",
        run.stderr,
    );
    assert!(
        !env.data_dir.join("nix").exists(),
        "nothing may be promoted for a missing tarball"
    );
}

#[test]
#[serial]
fn init_from_corrupt_tarball_fails_without_promotion() {
    let env = setup();
    let corrupt = env.tmp.path().join("corrupt.tar.gz");
    std::fs::write(&corrupt, b"not a gzip stream").expect("BUG: write corrupt tarball");

    let run = env.run_init_tarball(&corrupt, PROFILE_PATH);

    assert!(!run.status.success(), "a corrupt tarball must not exit 0");
    assert!(
        !env.data_dir.join("nix").exists(),
        "a failed extraction must never be promoted"
    );
}
