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

/// The libraries log through the `log` crate, so the lines go in through its bridge,
/// under the targets that print a dialled URL or the bytes sent.
#[test]
fn rust_log_cannot_raise_the_credential_libraries_past_info() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let log_path = td.path().join("var/log/bmc/bmc-wasm-host.log");

    // SAFETY: this is the only test in this binary, so no other thread
    // accesses the environment concurrently.
    unsafe {
        std::env::set_var(
            "RUST_LOG",
            "trace,tungstenite=trace,ureq=trace,ureq_proto=trace",
        );
    }
    bmc_log::init_file(&log_path).expect("BUG: init_file");

    log::debug!(target: "tungstenite::handshake::client", "request carrying a secret");
    log::trace!(target: "tungstenite::protocol", "frame carrying a secret");
    log::debug!(target: "ureq::run", "uri carrying a secret");
    log::trace!(target: "ureq_proto::util", "bytes carrying a secret");
    log::info!(target: "tungstenite::protocol", "tungstenite info");
    log::info!(target: "ureq::run", "ureq info");
    tracing::trace!("own trace");

    let contents = std::fs::read_to_string(&log_path).expect("BUG: read log");
    assert!(
        !contents.contains("carrying a secret"),
        "a directive naming a credential library must not lift it past INFO: {contents}"
    );
    for line in ["tungstenite info", "ureq info"] {
        assert!(
            contents.contains(line),
            "the ceiling must not silence the libraries altogether: {contents}"
        );
    }
    assert!(
        contents.contains("own trace"),
        "RUST_LOG must still raise everything else"
    );
}
