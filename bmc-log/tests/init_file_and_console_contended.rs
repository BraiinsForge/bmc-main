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
