// Copyright (C) 2026  Braiins Systems s.r.o.

#[test]
fn init_fails_when_log_parent_cannot_be_created() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let file_parent = td.path().join("not-a-directory");
    std::fs::write(&file_parent, b"occupied").expect("BUG: write file parent");

    let err = bmc_wasm_host::logging::init(&file_parent.join("host.log"))
        .expect_err("file logging failure must fail host startup");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
}
