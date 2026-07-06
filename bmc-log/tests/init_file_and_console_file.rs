// Copyright (C) 2026  Braiins Systems s.r.o.

const CLI_TARGET: &str = "test_console_target";

#[test]
fn file_and_console_routes_tracing_to_file() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let log_path = td.path().join("var/log/bmc/bmc-nix-cli.log");

    // SAFETY: this is the only test in this binary, so no other thread
    // accesses the environment concurrently.
    unsafe { std::env::remove_var("RUST_LOG") };
    let _guard = bmc_log::init_file_and_console(&log_path, CLI_TARGET);

    tracing::info!(target: CLI_TARGET, "Profile unchanged.");
    tracing::info!("library event");

    let contents = std::fs::read_to_string(&log_path).expect("BUG: read log");
    assert!(contents.contains("Profile unchanged."));
    assert!(contents.contains("library event"));
}
