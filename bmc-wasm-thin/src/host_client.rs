// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

pub fn send_load_and_wait_ack(
    _control: UnixStream,
    _wasm: &Path,
    _wayland: UnixStream,
    _ack_wait: Duration,
) -> Result<UnixStream> {
    anyhow::bail!("host client handshake is implemented in Task 2")
}
