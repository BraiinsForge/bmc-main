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

#[test]
#[ignore = "requires real compositor, Wayland peer credentials, DRM/EGL, and a built WASM widget"]
fn device_smoke_thin_receives_ack_ok() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use bmc_widget_protocol::BMC_WIDGET_KEY_ENV;

    let wasm = std::env::var("BMC_STAGE6_TEST_WASM")
        .expect("BUG: set BMC_STAGE6_TEST_WASM to a device-local .wasm path");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bmc-wasm-thin"));
    cmd.arg("--wasm")
        .arg(wasm)
        .env(BMC_WIDGET_KEY_ENV, "550e8400-e29b-41d4-a716-446655440000")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Ok(socket) = std::env::var("BMC_STAGE6_TEST_HOST_SOCKET") {
        cmd.arg("--host-socket").arg(socket);
    }
    let mut child = cmd.spawn().expect("BUG: spawn bmc-wasm-thin");

    std::thread::sleep(Duration::from_secs(2));
    assert!(
        child.try_wait().expect("BUG: poll thin child").is_none(),
        "thin exited before external termination"
    );
    unsafe {
        libc::kill(
            i32::try_from(child.id()).expect("BUG: child pid fits pid_t on Linux"),
            libc::SIGTERM,
        );
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .expect("BUG: poll thin child after SIGTERM")
        {
            break status;
        }
        assert!(Instant::now() < deadline, "thin did not exit after SIGTERM");
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(status.success(), "thin should exit 0 after SIGTERM");
}
