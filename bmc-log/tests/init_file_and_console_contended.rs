// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::fd::AsRawFd as _;

const CLI_TARGET: &str = "test_console_target";

fn lock_file(path: &std::path::Path) -> std::fs::File {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("BUG: create lock parent");
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("BUG: open lock");
    // SAFETY: file.as_raw_fd() is a valid open file descriptor for this call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(rc, 0, "BUG: lock should be available");
    file
}

#[test]
fn file_and_console_skips_file_when_lock_is_held() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let log_path = td.path().join("var/log/bmc/bmc-nix-cli.log");
    let lock_path = td.path().join("var/log/bmc/bmc-nix-cli.log.lock");
    let _held_lock = lock_file(&lock_path);

    // SAFETY: this is the only test in this binary, so no other thread
    // accesses the environment concurrently.
    unsafe { std::env::remove_var("RUST_LOG") };
    let _guard = bmc_log::init_file_and_console(&log_path, CLI_TARGET);

    tracing::info!(target: CLI_TARGET, "stderr-only");
    assert!(
        !log_path.exists(),
        "contended logger must not open the rotated log"
    );
}
