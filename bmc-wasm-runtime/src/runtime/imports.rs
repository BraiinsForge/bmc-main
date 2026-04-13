// Copyright (C) 2026  Braiins Systems s.r.o.

//! Guest import registration for the WASM runtime.

mod data;
mod network;
mod render;

use anyhow::Result;
use wasmi::Linker;

use crate::host_api::HostState;

pub(super) fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
    render::register(linker)?;
    network::register(linker)?;
    data::register(linker)?;
    Ok(())
}
