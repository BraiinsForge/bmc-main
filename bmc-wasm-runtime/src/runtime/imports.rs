// Copyright (C) 2026  Braiins Systems s.r.o.

//! Guest import registration for the WASM runtime.

mod audio;
mod data;
mod eviction;
mod led;
mod network;
mod render;
mod system;

use anyhow::Result;
use wasmi::Linker;

use crate::host_api::HostState;

pub(super) fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
    render::register(linker)?;
    network::register(linker)?;
    data::register(linker)?;
    system::register(linker)?;
    audio::register(linker)?;
    led::register(linker)?;
    eviction::register(linker)?;
    Ok(())
}
