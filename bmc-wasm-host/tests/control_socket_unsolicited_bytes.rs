// Copyright (C) 2026  Braiins Systems s.r.o.

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
