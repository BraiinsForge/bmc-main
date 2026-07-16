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
