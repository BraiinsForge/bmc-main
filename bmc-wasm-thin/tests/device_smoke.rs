// Copyright (C) 2026  Braiins Systems s.r.o.

#[test]
#[ignore = "requires real compositor, Wayland peer credentials, DRM/EGL, and a built WASM widget"]
fn device_smoke_thin_receives_ack_ok() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let wasm = std::env::var("BMC_STAGE6_TEST_WASM")
        .expect("BUG: set BMC_STAGE6_TEST_WASM to a device-local .wasm path");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bmc-wasm-thin"));
    cmd.arg("--wasm")
        .arg(wasm)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Ok(socket) = std::env::var("BMC_STAGE6_TEST_HOST_SOCKET") {
        cmd.arg("--host-socket").arg(socket);
    }
    if let Ok(host_bin) = std::env::var("BMC_STAGE6_TEST_HOST_BIN") {
        cmd.arg("--host-bin").arg(host_bin);
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
