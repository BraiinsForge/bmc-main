// Copyright (C) 2026  Braiins Systems s.r.o.

const CLI_TARGET: &str = "test_console_target";

#[test]
fn console_target_reaches_file_even_when_rust_log_filters_it_out() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let log_path = td.path().join("var/log/bmc/bmc-nix-cli.log");

    // SAFETY: this is the only test in this binary, so no other thread
    // accesses the environment concurrently.
    unsafe { std::env::set_var("RUST_LOG", "off") };
    let _guard = bmc_log::init_file_and_console(&log_path, CLI_TARGET);

    tracing::info!(target: CLI_TARGET, "diagnostic line");
    tracing::info!("library event");

    let contents = std::fs::read_to_string(&log_path).expect("BUG: read log");
    assert!(
        contents.contains("diagnostic line"),
        "console target must reach the file regardless of RUST_LOG"
    );
    assert!(
        !contents.contains("library event"),
        "RUST_LOG=off must still suppress untargeted events"
    );
}
