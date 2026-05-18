// Copyright (C) 2026  Braiins Systems s.r.o.

use std::io::Write;
use std::os::unix::net::UnixStream;

#[test]
fn idle_loop_errors_on_unsolicited_byte_from_host() {
    let (thin_side, host_side) =
        UnixStream::pair().expect("BUG: test fixture requires UnixStream::pair() to succeed");

    let handle = std::thread::spawn(move || bmc_wasm_thin::host_client::idle_until_exit(thin_side));

    std::thread::sleep(std::time::Duration::from_millis(50));
    (&host_side)
        .write_all(&[0x42])
        .expect("BUG: local socketpair write_all must succeed");

    let res = handle
        .join()
        .expect("BUG: idle_until_exit thread must not panic");
    assert!(
        res.is_err(),
        "expected idle loop to fail on unsolicited byte, got {res:?}"
    );
}
