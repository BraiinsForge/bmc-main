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

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use bmc_grpc::web::scene_management_service_client::SceneManagementServiceClient;
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

const SCENARIO: &str = r#"{"firmware":"up-to-date","packages":"unavailable"}"#;
const SIGNAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct CookieAuth(MetadataValue<tonic::metadata::Ascii>);

impl tonic::service::Interceptor for CookieAuth {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, tonic::Status> {
        request.metadata_mut().insert("cookie", self.0.clone());
        Ok(request)
    }
}

type SceneClient = SceneManagementServiceClient<
    tonic::service::interceptor::InterceptedService<Channel, CookieAuth>,
>;

async fn widget_names(client: &mut SceneClient) -> Vec<String> {
    client
        .get_available_widgets(())
        .await
        .expect("BUG: list available widgets")
        .into_inner()
        .widgets
        .into_iter()
        .map(|widget| widget.name)
        .collect()
}

async fn wait_for_names(client: &mut SceneClient, expected: &[&str]) {
    let deadline = Instant::now() + SIGNAL_TIMEOUT;
    loop {
        let mut names = widget_names(client).await;
        names.sort();
        let mut expected: Vec<_> = expected.iter().map(|name| (*name).to_owned()).collect();
        expected.sort();
        if names == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "widget names did not become {expected:?}: {names:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn write_widget(root: &Path) {
    let widget_dir = root.join("signal-test");
    std::fs::create_dir_all(widget_dir.join("bin")).expect("BUG: create signal widget");
    let binary = widget_dir.join("bin/signal-test");
    std::fs::write(&binary, "#!/bin/sh\n").expect("BUG: write signal widget binary");
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
        .expect("BUG: chmod signal widget binary");
    std::fs::write(
        widget_dir.join("manifest.json"),
        r#"{"uid":"71bfce7e-72c7-43ac-8221-45f680693938","version":"1.0.0","name":"Signal Test","description":"Signal-only refresh fixture","binary":"bin/signal-test","supported_viewports":[{"type":"rectangular","min_width":317,"max_width":1280,"min_height":238,"max_height":480}]}"#,
    )
    .expect("BUG: write signal widget manifest");
}

#[derive(Clone, Copy)]
enum UserSignal {
    Usr1,
    Usr2,
    Winch,
}

impl UserSignal {
    fn argument(self) -> &'static str {
        match self {
            Self::Usr1 => "-USR1",
            Self::Usr2 => "-USR2",
            Self::Winch => "-WINCH",
        }
    }
}

fn signal_dispositions(pid: u32) -> (u64, u64) {
    let status =
        std::fs::read_to_string(format!("/proc/{pid}/status")).expect("BUG: read mock status");
    let field = |name: &str| -> u64 {
        let line = status
            .lines()
            .find(|line| line.starts_with(name))
            .expect("BUG: signal mask field in /proc status");
        u64::from_str_radix(
            line.split_whitespace()
                .nth(1)
                .expect("BUG: signal mask value"),
            16,
        )
        .expect("BUG: signal mask parses as hex")
    };
    (field("SigCgt:"), field("SigIgn:"))
}

fn sig_bit(signal: i32) -> u64 {
    1 << (signal - 1)
}

fn send_signal(mock: &common::MockInstance, signal: UserSignal) {
    let status = Command::new("kill")
        .args([signal.argument(), &mock.child.id().to_string()])
        .status()
        .expect("BUG: invoke kill");
    assert!(status.success(), "failed to signal bmc-mock child");
}

#[tokio::test]
async fn user_signals_are_drained_and_winch_alone_refreshes_the_registry() {
    let mut mock = common::spawn_mock(SCENARIO);
    let (channel, cookie) = common::authenticated_channel(&mut mock).await;
    let mut client = SceneManagementServiceClient::with_interceptor(channel, CookieAuth(cookie));
    wait_for_names(&mut client, &["Flip Clock", "Weather"]).await;

    // A SIG_IGN disposition would survive exec into widget children;
    // an installed handler resets to default there. Lock the kernel-visible
    // shape: all three signals caught, none ignored.
    let (caught, ignored) = signal_dispositions(mock.child.id());
    for signal in [libc::SIGWINCH, libc::SIGUSR1, libc::SIGUSR2] {
        assert_ne!(
            caught & sig_bit(signal),
            0,
            "signal {signal} must have an installed handler"
        );
        assert_eq!(
            ignored & sig_bit(signal),
            0,
            "signal {signal} must not be SIG_IGN"
        );
    }

    send_signal(&mock, UserSignal::Usr1);
    send_signal(&mock, UserSignal::Usr2);
    wait_for_names(&mut client, &["Flip Clock", "Weather"]).await;
    assert!(mock.child.try_wait().expect("BUG: inspect child").is_none());

    let staging = mock.mockfs.join("tmp/staged-widgets");
    write_widget(&staging);
    tokio::time::sleep(Duration::from_millis(200)).await;
    wait_for_names(&mut client, &["Flip Clock", "Weather"]).await;

    send_signal(&mock, UserSignal::Winch);
    wait_for_names(&mut client, &["Flip Clock", "Signal Test", "Weather"]).await;
    send_signal(&mock, UserSignal::Winch);
    wait_for_names(&mut client, &["Flip Clock", "Signal Test", "Weather"]).await;

    std::fs::remove_dir_all(staging.join("signal-test")).expect("BUG: remove signal widget");
    send_signal(&mock, UserSignal::Winch);
    wait_for_names(&mut client, &["Flip Clock", "Weather"]).await;

    std::fs::remove_dir_all(&staging).expect("BUG: empty staged widget tree");
    send_signal(&mock, UserSignal::Winch);
    wait_for_names(&mut client, &[]).await;
}
