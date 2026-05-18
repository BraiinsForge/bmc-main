// Copyright (C) 2026  Braiins Systems s.r.o.

use clap::Parser as _;

use bmc_wasm_thin::args::{Config, RawArgs};

#[test]
fn missing_wasm_is_rejected_by_clap() {
    let err = RawArgs::try_parse_from(["bmc-wasm-thin"]).expect_err("missing --wasm must fail");
    assert!(err.to_string().contains("--wasm"));
}

#[test]
fn invalid_env_override_is_reported() {
    let raw = RawArgs::try_parse_from(["bmc-wasm-thin", "--wasm", "/tmp/widget.wasm"])
        .expect("BUG: --wasm is enough for raw parse");
    let err =
        Config::from_raw_with_env(raw, &[("BMC_WASM_HOST_WAIT_MS", "not-a-number".to_owned())])
            .expect_err("invalid wait env must fail");
    assert!(err.to_string().contains("BMC_WASM_HOST_WAIT_MS"));
}
