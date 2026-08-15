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
                    if renderer.evict_prefix_requires_gpu_access(&namespaced) {
                        super::require_renderer_gpu_access(state)?;
                    }
                    let rend = renderer.evict_prefix(&namespaced);
                    state.renderer_assets.remove_prefix(&prefix);
                    Ok((audio, rend))
                })
                .and_then(std::convert::identity)?;
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
                    if renderer.evict_prefix_requires_gpu_access(&ns) {
                        super::require_renderer_gpu_access(state)?;
                    }
                    let rend = renderer.evict_prefix(&ns);
                    state.renderer_assets.clear();
                    Ok((audio, rend))
                })
                .and_then(std::convert::identity)?;
            Ok(u32::try_from(audio_evicted + renderer_evicted).unwrap_or(u32::MAX))
        },
    )?;
    Ok(())
}
