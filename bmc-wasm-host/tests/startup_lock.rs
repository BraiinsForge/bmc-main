// Copyright (C) 2026  Braiins Systems s.r.o.

use std::fs::OpenOptions;
use std::os::fd::AsFd as _;

use bmc_wasm_host::startup::validate_release_lock_fd;
use tempfile::tempdir;

#[test]
fn release_lock_fd_must_point_at_selected_lockfile() {
    let dir = tempdir().expect("BUG: tempdir should be available");
    let lock = dir.path().join("host.lock");
    let other = dir.path().join("other.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock)
        .expect("BUG: lock open should succeed");
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&other)
        .expect("BUG: other lock open should succeed");

    validate_release_lock_fd(lock_file.as_fd(), &lock)
        .expect("BUG: fd pointing at lockfile should validate");
    let err = validate_release_lock_fd(lock_file.as_fd(), &other)
        .expect_err("fd pointing at another inode must fail");
    assert!(err.to_string().contains("release-lock-fd"));
}

#[test]
fn bind_loser_drops_release_lock_and_reports_existing_host() {
    use std::os::fd::IntoRawFd as _;

    let dir = tempdir().expect("BUG: tempdir should be available");
    let socket = dir.path().join("host.sock");
    let lock = dir.path().join("host.lock");
    let _existing_host = std::os::unix::net::UnixListener::bind(&socket)
        .expect("BUG: fake existing host should bind socket");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock)
        .expect("BUG: lock open should succeed");
    rustix::fs::flock(&lock_file, rustix::fs::FlockOperation::LockExclusive)
        .expect("BUG: parent should hold LOCK_EX before spawning waiter");
    let release_lock_fd = lock_file.into_raw_fd();

    let waiter_lock = lock.clone();
    let waiter = std::thread::spawn(move || {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&waiter_lock)
            .expect("BUG: waiter lock open should succeed");
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockShared)
            .expect("BUG: waiter should acquire shared lock after release-lock-fd drop");
    });

    let decision = bmc_wasm_host::startup::prepare_listener(&socket, Some(release_lock_fd))
        .expect("BUG: bind loser should be reported as a non-fatal startup decision");
    assert!(matches!(
        decision,
        bmc_wasm_host::startup::StartupDecision::AnotherHostAlive
    ));
    waiter
        .join()
        .expect("BUG: waiter should observe lock release");
    std::os::unix::net::UnixStream::connect(&socket)
        .expect("BUG: waiting thin should be able to connect to existing host");
}
