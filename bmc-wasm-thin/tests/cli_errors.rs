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
