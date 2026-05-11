// Copyright (C) 2026  Braiins Systems s.r.o.

//! Cross-registry eviction primitive exposed to guests.
//!
//! `host_evict_prefix(prefix)` drops every icon, bitmap, mesh, and audio
//! sample whose tag starts with the given prefix, plus any active rodio
//! sinks for evicted audio IDs.

use anyhow::Result;
use wasmi::{Caller, Linker};

use crate::host_api::HostState;

use super::super::memory::read_string;

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_evict_prefix",
        |mut caller: Caller<'_, HostState>, prefix_ptr: u32, prefix_len: u32| -> u32 {
            let Some(prefix) = read_string(&caller, prefix_ptr, prefix_len) else {
                return 0;
            };
            let evicted = caller.data_mut().evict_prefix(&prefix);
            u32::try_from(evicted).unwrap_or(u32::MAX)
        },
    )?;
    Ok(())
}
