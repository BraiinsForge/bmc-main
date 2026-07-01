// Copyright (C) 2026  Braiins Systems s.r.o.

//! Cross-registry eviction primitive exposed to guests.
//!
//! `host_evict_prefix(prefix)` drops every icon, bitmap, mesh, and audio
//! sample whose tag starts with the given prefix, plus any active rodio
//! sinks for evicted audio IDs.
//! `host_evict_all()` sweeps the widget's whole namespace in one call.

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
            let (audio_evicted, renderer_evicted) =
                super::with_renderer_and_state(&mut caller, |renderer, state| {
                    let namespaced = state.namespaced_tag(&prefix);
                    let audio = state.evict_audio_prefix(&namespaced);
                    let rend = renderer.evict_prefix(&namespaced);
                    (audio, rend)
                })?;
            Ok(u32::try_from(audio_evicted + renderer_evicted).unwrap_or(u32::MAX))
        },
    )?;
    linker.func_wrap(
        "env",
        "host_evict_all",
        |mut caller: Caller<'_, HostState>| -> Result<u32, wasmi::Error> {
            let (audio_evicted, renderer_evicted) =
                super::with_renderer_and_state(&mut caller, |renderer, state| {
                    let ns = state.instance_namespace().to_owned();
                    let audio = state.evict_audio_prefix(&ns);
                    let rend = renderer.evict_prefix(&ns);
                    (audio, rend)
                })?;
            Ok(u32::try_from(audio_evicted + renderer_evicted).unwrap_or(u32::MAX))
        },
    )?;
    Ok(())
}
