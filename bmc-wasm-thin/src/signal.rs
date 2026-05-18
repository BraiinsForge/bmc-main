// Copyright (C) 2026  Braiins Systems s.r.o.

use anyhow::Result;

pub struct SignalPipe;

impl std::fmt::Debug for SignalPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalPipe").finish_non_exhaustive()
    }
}

impl SignalPipe {
    pub fn new() -> Result<Self> {
        anyhow::bail!("signal self-pipe is implemented in Task 4")
    }
}
