// Copyright (C) 2026  Braiins Systems s.r.o.

#[test]
fn init_file_logs_to_file() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let log_path = td.path().join("widget.log");

    // SAFETY: this is the only test in this binary, so no other thread
    // accesses the environment concurrently.
    unsafe { std::env::remove_var("RUST_LOG") };
    bmc_log::init_file(&log_path).expect("BUG: init_file");

    let contents = std::fs::read_to_string(&log_path).expect("BUG: read log file");
    assert!(contents.contains("file logging initialized"));
}
