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

use bmc_wasm_host::main_loop::accept_pending;

// A thin's `connect()` into a listening-but-not-yet-accepting socket succeeds
// and parks the connection in the kernel backlog. Each listener wake must drain
// the backlog in one non-blocking pass instead of delaying sibling thins.
#[test]
fn accept_pending_drains_the_whole_backlog_without_blocking() {
    let dir = tempfile::tempdir().expect("BUG: tempdir creation must succeed");
    let path = dir.path().join("host.sock");
    let listener = UnixListener::bind(&path).expect("BUG: bind loopback unix listener");
    listener
        .set_nonblocking(true)
        .expect("BUG: set listener non-blocking");

    // Empty backlog returns immediately rather than blocking on accept().
    assert!(
        accept_pending(&listener)
            .expect("BUG: sweep of empty backlog must not fail")
            .is_empty(),
        "an empty backlog must yield no connections"
    );

    // Two thins connect before the host accepts either — both sit in the backlog.
    let _a = UnixStream::connect(&path).expect("BUG: first client connect");
    let _b = UnixStream::connect(&path).expect("BUG: second client connect");

    let swept = accept_pending(&listener).expect("BUG: backlog sweep must succeed");
    assert_eq!(
        swept.len(),
        2,
        "both queued connections must be accepted in a single sweep"
    );

    // The backlog is drained; a follow-up sweep sees nothing.
    assert!(
        accept_pending(&listener)
            .expect("BUG: post-drain sweep must not fail")
            .is_empty(),
        "a drained backlog must yield no further connections"
    );
}
