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
