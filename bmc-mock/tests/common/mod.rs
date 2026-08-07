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

//! Shared bmc-mock process harness for the gRPC integration tests.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bmc_grpc::web::LoginRequest;
use bmc_grpc::web::authentication_service_client::AuthenticationServiceClient;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
pub const MOCK_SERVICE_NAME: &str = "bmc-mock-test-compositor";

pub struct MockInstance {
    pub child: Child,
    pub port: u16,
    pub mockfs: PathBuf,
    pub dir: tempfile::TempDir,
    pub password: String,
    index_path: Option<PathBuf>,
    fast_upgrades: bool,
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
    index_path: Option<&std::path::Path>,
    fast_upgrades: bool,
) -> Child {
    let mut args = vec![
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
    ];
    if fast_upgrades {
        args.push("--fast-upgrades".to_owned());
    }
    if let Some(index) = index_path {
        args.push(format!("--package-index={}", index.display()));
    }
    Command::new(env!("CARGO_BIN_EXE_bmc-mock"))
        .args(args)
        .env(bmc::manager::SERVICE_NAME_ENV, MOCK_SERVICE_NAME)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("BUG: spawn bmc-mock")
}

pub fn spawn_mock(scenario_json: &str) -> MockInstance {
    spawn_mock_inner(scenario_json, None, true)
}

pub fn spawn_mock_inner(
    scenario_json: &str,
    index_json: Option<&str>,
    fast_upgrades: bool,
) -> MockInstance {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let template = dir.path().join("template/etc");
    std::fs::create_dir_all(&template).expect("BUG: create template");
    std::fs::write(template.join("upgrade-scenario.json"), scenario_json)
        .expect("BUG: write scenario");
    let index_path = index_json.map(|json| {
        let path = dir.path().join("package-index.json");
        std::fs::write(&path, json).expect("BUG: write index");
        path
    });
    let widget_dir = dir.path().join("widgets/flip-clock");
    std::fs::create_dir_all(&widget_dir).expect("BUG: create widget dir");
    std::fs::write(widget_dir.join("icon.svg"), "<svg/>").expect("BUG: write icon");
    let bin_dir = widget_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("BUG: create bin dir");
    let binary = bin_dir.join("flip-clock");
    std::fs::write(&binary, "#!/bin/sh\n").expect("BUG: write binary");
    std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("BUG: chmod binary");
    // The viewport bounds bracket the BMC100 slot descriptors (317x238 small
    // up to 1280x480 fullscreen) so gRPC tests can assert concrete supported
    // sizes; weather's higher min_width excludes the small slot.
    std::fs::write(
        widget_dir.join("manifest.json"),
        r#"{"uid":"7cb584a8-1f26-42a0-867e-955aadd2391c","version":"1.0.0",
           "name":"Flip Clock","description":"A retro split-flap clock face.",
           "binary":"bin/flip-clock","icon":"icon.svg","category":"clock",
           "supported_viewports":[{"type":"rectangular","min_width":317,
           "max_width":1280,"min_height":238,"max_height":480}]}"#,
    )
    .expect("BUG: write manifest");
    let weather_dir = dir.path().join("widgets/weather");
    std::fs::create_dir_all(&weather_dir).expect("BUG: create weather dir");
    std::fs::write(weather_dir.join("icon.svg"), "<svg/>").expect("BUG: write weather icon");
    let weather_bin = weather_dir.join("bin");
    std::fs::create_dir_all(&weather_bin).expect("BUG: create weather bin dir");
    let weather_binary = weather_bin.join("weather");
    std::fs::write(&weather_binary, "#!/bin/sh\n").expect("BUG: write weather binary");
    std::fs::set_permissions(
        &weather_binary,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("BUG: chmod weather binary");
    std::fs::write(
        weather_dir.join("manifest.json"),
        r#"{"uid":"2b1e7a6c-9d84-4f11-bb0e-3c7a1e6f92da","version":"1.0.0",
           "name":"Weather","description":"A local weather widget.",
           "binary":"bin/weather","icon":"icon.svg","category":"clock",
           "supported_viewports":[{"type":"rectangular","min_width":638,
           "max_width":1280,"min_height":238,"max_height":480}]}"#,
    )
    .expect("BUG: write weather manifest");
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
    let child = spawn_child(
        &dir,
        &mockfs,
        port,
        &password,
        index_path.as_deref(),
        fast_upgrades,
    );

    MockInstance {
        child,
        port,
        mockfs,
        dir,
        password,
        index_path,
        fast_upgrades,
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
            mock.child = spawn_child(
                &mock.dir,
                &mock.mockfs,
                mock.port,
                &mock.password,
                mock.index_path.as_deref(),
                mock.fast_upgrades,
            );
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
                mock.child = spawn_child(
                    &mock.dir,
                    &mock.mockfs,
                    mock.port,
                    &mock.password,
                    mock.index_path.as_deref(),
                    mock.fast_upgrades,
                );
            }
        }
    }
    panic!("BUG: login kept failing after respawns: {last_err:?}");
}
