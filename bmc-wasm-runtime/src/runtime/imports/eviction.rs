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
        |mut caller: Caller<'_, HostState>,
         prefix_ptr: u32,
         prefix_len: u32|
         -> Result<u32, wasmi::Error> {
            let Some(prefix) = read_string(&caller, prefix_ptr, prefix_len) else {
                return Ok(0);
            };
            // Trap up front when called outside a render scope so the audio
            // and renderer halves stay consistent (otherwise the audio half
            // would mutate before the renderer half traps).
            if caller.data().renderer_ptr.is_none() {
                return Err(wasmi::Error::new(
                    "host_evict_prefix called outside render scope (called from \
                     init or on_params_update?)",
                ));
            }
            let namespaced = caller.data().namespaced_tag(&prefix);
            let audio_evicted = caller.data_mut().evict_audio_prefix(&namespaced);
            let renderer_evicted =
                super::with_renderer(&mut caller, |renderer| renderer.evict_prefix(&namespaced))?;
            Ok(u32::try_from(audio_evicted + renderer_evicted).unwrap_or(u32::MAX))
        },
    )?;
    Ok(())
}
