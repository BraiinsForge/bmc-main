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

use std::io;

use bmc_wasm_host::slot::{ControlSocketStatus, classify_control_socket_read};

#[test]
fn unsolicited_byte_is_classified_as_protocol_violation() {
    let status = classify_control_socket_read(Ok(1), 0x42);
    assert!(matches!(status, ControlSocketStatus::UnsolicitedByte(0x42)));
}

#[test]
fn peer_close_is_clean_teardown() {
    let status = classify_control_socket_read(Ok(0), 0);
    assert!(matches!(status, ControlSocketStatus::PeerClosed));
}

#[test]
fn would_block_is_passthrough() {
    let status = classify_control_socket_read(Err(io::Error::from(io::ErrorKind::WouldBlock)), 0);
    assert!(matches!(status, ControlSocketStatus::WouldBlock));
}
