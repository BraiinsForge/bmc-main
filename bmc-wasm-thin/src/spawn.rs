// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::unix::net::UnixStream;

use anyhow::Result;

use crate::args::Config;

pub fn connect_or_spawn(_config: &Config) -> Result<UnixStream> {
    anyhow::bail!("host connect/spawn is implemented in Task 3")
}
