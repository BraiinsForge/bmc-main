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

use std::os::unix::net::{UnixListener, UnixStream};

use bmc_wasm_host::startup::prepare_listener;

#[test]
fn creates_missing_socket_directory() {
    let directory = tempfile::tempdir().expect("BUG: create temporary directory");
    let socket = directory.path().join("missing").join("host.sock");

    let listener = prepare_listener(&socket).expect("host must create its socket directory");

    UnixStream::connect(&socket).expect("the host socket must accept connections");
    drop(listener);
}

#[test]
fn rejects_a_second_live_host() {
    let directory = tempfile::tempdir().expect("BUG: create temporary directory");
    let socket = directory.path().join("host.sock");
    let existing = UnixListener::bind(&socket).expect("BUG: bind existing host");

    let error = prepare_listener(&socket).expect_err("a live host must retain socket ownership");

    assert!(
        error
            .root_cause()
            .to_string()
            .contains("another bmc-wasm-host is alive")
    );
    drop(existing);
}

#[test]
fn replaces_a_stale_socket_after_host_crash() {
    let directory = tempfile::tempdir().expect("BUG: create temporary directory");
    let socket = directory.path().join("host.sock");
    drop(UnixListener::bind(&socket).expect("BUG: bind stale host socket"));

    let listener = prepare_listener(&socket).expect("a stale socket must not block host restart");

    UnixStream::connect(&socket).expect("the restarted host must accept connections");
    drop(listener);
}
