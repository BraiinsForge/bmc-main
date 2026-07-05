// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::unix::net::{UnixListener, UnixStream};

use bmc_wasm_host::main_loop::accept_pending;

// A thin's `connect()` into a listening-but-not-yet-accepting socket succeeds
// and parks the connection in the kernel backlog. The host's pre-exit sweep
// must drain that backlog in one non-blocking pass; otherwise a connection that
// queued during the last slot's teardown is orphaned when the listener drops.
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
