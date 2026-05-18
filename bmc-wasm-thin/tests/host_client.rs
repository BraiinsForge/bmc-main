// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use bmc_wasm_thin::host_client::send_load_and_wait_ack;
use bmc_wasm_thin_protocol::{AckMsg, HelloMsg, recv_hello_with_fd, write_ack};

fn socketpair() -> (UnixStream, UnixStream) {
    UnixStream::pair().expect("BUG: UnixStream::pair must work in Unix tests")
}

#[test]
fn sends_hello_with_one_wayland_fd_and_reads_ok_ack() {
    let (control_client, control_server) = socketpair();
    let (wayland_client, wayland_server) = socketpair();
    let server = thread::spawn(move || {
        let (msg, fd) =
            recv_hello_with_fd(&control_server).expect("BUG: fake host should receive Hello");
        match msg {
            HelloMsg::Load { wasm_path } => assert_eq!(wasm_path, "/tmp/widget.wasm"),
        }
        assert!(fd.as_raw_fd() >= 0);
        write_ack(&control_server, &AckMsg::Ok).expect("BUG: fake host should write Ack::Ok");
    });

    send_load_and_wait_ack(
        control_client,
        Path::new("/tmp/widget.wasm"),
        wayland_client,
        Duration::from_secs(1),
    )
    .expect("BUG: Ack::Ok should succeed");
    drop(wayland_server);
    server.join().expect("BUG: fake host thread should finish");
}

#[test]
fn ack_err_is_a_load_error() {
    let (control_client, control_server) = socketpair();
    let (wayland_client, _wayland_server) = socketpair();
    let server = thread::spawn(move || {
        let (_msg, _fd) =
            recv_hello_with_fd(&control_server).expect("BUG: fake host should receive Hello");
        write_ack(&control_server, &AckMsg::Err("bad wasm".into()))
            .expect("BUG: fake host should write Ack::Err");
    });
    let err = send_load_and_wait_ack(
        control_client,
        Path::new("/tmp/bad.wasm"),
        wayland_client,
        Duration::from_secs(1),
    )
    .expect_err("Ack::Err should fail thin startup");
    assert!(err.to_string().contains("bad wasm"));
    server.join().expect("BUG: fake host thread should finish");
}

#[test]
fn silent_host_times_out_without_busy_looping() {
    let (control_client, control_server) = socketpair();
    let (wayland_client, _wayland_server) = socketpair();
    let _server = thread::spawn(move || {
        let (_msg, _fd) =
            recv_hello_with_fd(&control_server).expect("BUG: fake host should receive Hello");
        thread::sleep(Duration::from_millis(250));
    });
    let start = Instant::now();
    let err = send_load_and_wait_ack(
        control_client,
        Path::new("/tmp/widget.wasm"),
        wayland_client,
        Duration::from_millis(50),
    )
    .expect_err("silent host must time out");
    assert!(err.to_string().contains("timed out"));
    assert!(start.elapsed() >= Duration::from_millis(45));
}

#[test]
fn eof_before_ack_is_startup_error() {
    let (control_client, control_server) = socketpair();
    let (wayland_client, _wayland_server) = socketpair();
    let server = thread::spawn(move || {
        let (_msg, _fd) =
            recv_hello_with_fd(&control_server).expect("BUG: fake host should receive Hello");
        drop(control_server);
    });
    let err = send_load_and_wait_ack(
        control_client,
        Path::new("/tmp/widget.wasm"),
        wayland_client,
        Duration::from_secs(1),
    )
    .expect_err("EOF before Ack must fail");
    assert!(err.to_string().contains("ack") || err.to_string().contains("EOF"));
    server.join().expect("BUG: fake host thread should finish");
}
