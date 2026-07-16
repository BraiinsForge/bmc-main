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

use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use bmc_wasm_thin::host_client::{IdleExit, idle_until_exit};

// Serialize signal-delivery tests: under `cargo test`, all tests share the
// same process, and a SIGTERM/SIGINT sent for one test would also wake the
// other test's `poll`. nextest runs each test in its own process and does
// not need this guard, but the lock is harmless there.
static SIGNAL_TEST_LOCK: Mutex<()> = Mutex::new(());

fn socketpair() -> (UnixStream, UnixStream) {
    UnixStream::pair().expect("BUG: UnixStream::pair must work in Unix tests")
}

#[test]
fn control_socket_eof_exits_cleanly() {
    let (client, server) = socketpair();
    drop(server);
    assert_eq!(
        idle_until_exit(client).expect("BUG: EOF should be clean"),
        IdleExit::Clean
    );
}

#[test]
fn sigterm_wakes_poll_and_exits_cleanly() {
    let _guard = SIGNAL_TEST_LOCK.lock().expect("BUG: signal test lock");
    let (client, _server) = socketpair();
    let handle =
        thread::spawn(move || idle_until_exit(client).expect("BUG: SIGTERM should be clean"));
    thread::sleep(Duration::from_millis(50));
    unsafe {
        libc::kill(libc::getpid(), libc::SIGTERM);
    }
    let start = Instant::now();
    loop {
        if handle.is_finished() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "SIGTERM did not wake poll"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        handle.join().expect("BUG: idle thread should join"),
        IdleExit::Signal
    );
}

#[test]
fn sigint_wakes_poll_and_exits_cleanly() {
    let _guard = SIGNAL_TEST_LOCK.lock().expect("BUG: signal test lock");
    let (client, _server) = socketpair();
    let handle =
        thread::spawn(move || idle_until_exit(client).expect("BUG: SIGINT should be clean"));
    thread::sleep(Duration::from_millis(50));
    unsafe {
        libc::kill(libc::getpid(), libc::SIGINT);
    }
    let start = Instant::now();
    loop {
        if handle.is_finished() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "SIGINT did not wake poll"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        handle.join().expect("BUG: idle thread should join"),
        IdleExit::Signal
    );
}

#[test]
fn idle_revents_classifier_rejects_socket_errors() {
    assert!(bmc_wasm_thin::host_client::classify_idle_revents(0).is_ok());
    assert!(bmc_wasm_thin::host_client::classify_idle_revents(libc::POLLHUP).is_ok());
    assert!(bmc_wasm_thin::host_client::classify_idle_revents(libc::POLLERR).is_err());
    assert!(bmc_wasm_thin::host_client::classify_idle_revents(libc::POLLNVAL).is_err());
}
