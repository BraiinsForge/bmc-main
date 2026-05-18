// Copyright (C) 2026  Braiins Systems s.r.o.

use std::io::Write as _;

#[test]
fn open_log_file_creates_parent_and_appends() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let log_path = td.path().join("var/log/bmc/bmc-wasm-thin.log");

    {
        let mut file =
            bmc_wasm_thin::logging::open_log_file(&log_path).expect("BUG: open thin log file");
        writeln!(file, "first").expect("BUG: write first log line");
    }
    {
        let mut file =
            bmc_wasm_thin::logging::open_log_file(&log_path).expect("BUG: reopen thin log file");
        writeln!(file, "second").expect("BUG: write second log line");
    }

    let contents = std::fs::read_to_string(&log_path).expect("BUG: read thin log file");
    assert_eq!(contents, "first\nsecond\n");
}
