// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::unix::net::UnixStream;

use anyhow::Result;

pub fn connect_from_env() -> Result<UnixStream> {
    anyhow::bail!("Wayland fd connection is implemented in Task 2")
}
