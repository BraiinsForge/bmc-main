// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared bmc-mock process harness for the gRPC integration tests.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bmc_grpc::web::LoginRequest;
use bmc_grpc::web::authentication_service_client::AuthenticationServiceClient;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

pub struct MockInstance {
    pub child: Child,
    pub port: u16,
    pub mockfs: PathBuf,
    pub dir: tempfile::TempDir,
    pub password: String,
}

impl Drop for MockInstance {
    fn drop(&mut self) {
        _ = self.child.kill();
        _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("BUG: bind for port pick")
        .local_addr()
        .expect("BUG: local_addr")
        .port()
}

fn spawn_child(
    dir: &tempfile::TempDir,
    mockfs: &std::path::Path,
    port: u16,
    password: &str,
) -> Child {
    Command::new(env!("CARGO_BIN_EXE_bmc-mock"))
        .args([
            format!("--address=127.0.0.1:{port}"),
            format!("--mockfs-path={}", mockfs.display()),
            format!(
                "--mockfs-template={}",
                dir.path().join("template").display()
            ),
            format!("--www-path={}", dir.path().join("www").display()),
            format!("--sounds-dir={}", dir.path().join("sounds").display()),
            format!("--widgets-path={}", dir.path().join("widgets").display()),
            format!("--system-password={password}"),
            "--fast-upgrades".to_owned(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("BUG: spawn bmc-mock")
}

pub fn spawn_mock(scenario_json: &str) -> MockInstance {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let template = dir.path().join("template/etc");
    std::fs::create_dir_all(&template).expect("BUG: create template");
    std::fs::write(template.join("upgrade-scenario.json"), scenario_json)
        .expect("BUG: write scenario");
    let mockfs = dir.path().join("mockfs");
    let port = free_port();
    // Per-instance password (the unique tempdir name), so logging in on a
    // foreign mock that stole our port fails instead of succeeding.
    let password = format!(
        "pw-{}",
        dir.path()
            .file_name()
            .expect("BUG: tempdir has a name")
            .to_string_lossy()
    );
    let child = spawn_child(&dir, &mockfs, port, &password);

    MockInstance {
        child,
        port,
        mockfs,
        dir,
        password,
    }
}

// free_port() picks a port by bind-and-drop, so another process can steal
// it before the child binds. connect() detects the resulting child death
// and respawns on a fresh port instead of hanging until the deadline.
async fn connect(mock: &mut MockInstance) -> Channel {
    let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
    let mut respawns = 0;
    loop {
        if let Some(status) = mock.child.try_wait().expect("BUG: try_wait failed") {
            assert!(respawns < 3, "mock kept dying at startup: {status}");
            respawns += 1;
            mock.port = free_port();
            mock.child = spawn_child(&mock.dir, &mock.mockfs, mock.port, &mock.password);
        }
        let endpoint = format!("http://127.0.0.1:{}", mock.port);
        match Channel::from_shared(endpoint)
            .expect("BUG: endpoint parse")
            .connect()
            .await
        {
            Ok(channel) => return channel,
            Err(err) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "mock did not come up: {err}"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

async fn login(
    channel: Channel,
    password: &str,
) -> Result<MetadataValue<tonic::metadata::Ascii>, tonic::Status> {
    let mut auth = AuthenticationServiceClient::new(channel);
    let response = auth
        .login(LoginRequest {
            password: password.to_owned(),
        })
        .await?
        .into_inner();
    Ok(format!("session_id={}", response.token)
        .parse()
        .expect("BUG: cookie metadata parse"))
}

// A successful connect() can still land on a foreign mock: free_port()'s
// bind-and-drop race (see connect()'s comment) can hand two tests the same
// port, and the loser dies only after we already connected to the winner.
// The per-instance password makes login fail on a foreign mock; retrying
// the whole connect+login on a fresh port then recovers.
pub async fn authenticated_channel(
    mock: &mut MockInstance,
) -> (Channel, MetadataValue<tonic::metadata::Ascii>) {
    let mut last_err = None;
    for _ in 0..4 {
        let channel = connect(mock).await;
        match login(channel.clone(), &mock.password).await {
            Ok(cookie) => return (channel, cookie),
            Err(err) => {
                last_err = Some(err);
                _ = mock.child.kill();
                _ = mock.child.wait();
                mock.port = free_port();
                mock.child = spawn_child(&mock.dir, &mock.mockfs, mock.port, &mock.password);
            }
        }
    }
    panic!("BUG: login kept failing after respawns: {last_err:?}");
}
