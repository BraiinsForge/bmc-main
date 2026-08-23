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

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use std::io::Read;

use bmc_wasm_thin_protocol::{AckDecoder, AckMsg, HelloMsg, send_hello_with_fd};

fn wait_for_socket(path: &std::path::Path, deadline: Instant) {
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("host socket never appeared at {}", path.display());
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // The host's main loop initializes DRM/EGL, so this test is `#[ignore]`-d in CI
        // and owns its child process cleanup explicitly.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "Task 9 real main initializes SharedHost/EGL and needs /dev/dri/renderD128; run on device in Stage 6"]
fn handshake_with_nonwayland_fd_returns_err_ack() {
    let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed for handshake test");
    let socket_path = tmp.path().join("host.sock");

    let child = ChildGuard(Command::new(env!("CARGO_BIN_EXE_bmc-wasm-host"))
        .arg("--host-socket").arg(&socket_path)
        .stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn()
        .expect("BUG: test fixture expects CARGO_BIN_EXE_bmc-wasm-host to be buildable and spawnable"));

    wait_for_socket(&socket_path, Instant::now() + Duration::from_secs(5));

    let client = UnixStream::connect(&socket_path).expect(
        "BUG: wait_for_socket observed host.sock, so connect() must succeed before deadline",
    );
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("BUG: setting read timeout on local UnixStream must succeed");
    let (fd_keep, fd_to_send) = UnixStream::pair()
        .expect("BUG: test fixture requires local socketpair to provide fake wayland fd");

    send_hello_with_fd(
        &client,
        &HelloMsg::Load {
            widget_key: "550e8400-e29b-41d4-a716-446655440000".into(),
            wasm_path: "/nonexistent/widget.wasm".into(),
            asset_root: Some("/nonexistent/assets".into()),
        },
        fd_to_send.as_fd(),
    )
    .expect(
        "BUG: test fixture expects send_hello_with_fd to succeed on connected local socket pair",
    );
    drop(fd_to_send);
    drop(fd_keep);

    let ack = {
        let mut dec = AckDecoder::new();
        let mut buf = [0_u8; 64];
        let mut sock_r = &client;
        loop {
            let n = sock_r
                .read(&mut buf)
                .expect("BUG: host handshake contract requires an Ack after Hello");
            if let Some(msg) = dec
                .push(&buf[..n])
                .expect("BUG: host handshake contract requires a valid Ack frame")
            {
                break msg;
            }
        }
    };
    match ack {
        AckMsg::Err(_) => {}
        AckMsg::Ok => panic!("expected Err Ack, got Ok"),
    }

    drop(client);
    drop(child);
}
