// Copyright (C) 2026  Braiins Systems s.r.o.

//! Guest import registration for the WASM runtime.

mod audio;
mod data;
mod eviction;
mod guards;
mod led;
mod network;
pub(crate) mod params;
mod render;
mod system;

use anyhow::Result;
use bmc_render::renderer::Renderer;
use wasmi::{Caller, Linker};

use crate::host_api::HostState;

pub(super) fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
    render::register(linker)?;
    network::register(linker)?;
    data::register(linker)?;
    system::register(linker)?;
    audio::register(linker)?;
    led::register(linker)?;
    eviction::register(linker)?;
    params::register(linker)?;
    Ok(())
}

/// Reborrow the renderer parked on `HostState::renderer_ptr` for the duration
/// of `f`. Returns a wasmi trap if the pointer is absent — i.e. the host import
/// was called from outside a `WasmWidgetRuntime::with_renderer` scope.
///
/// # Safety
/// Callers must ensure no other `&mut Renderer` to the same renderer is live
/// while this reborrow exists. The single-threaded wasmi dispatch and the
/// no-async-yield-inside-host-fn rule make this true by construction: only one
/// host import can run at a time on the parking thread.
#[expect(
    dead_code,
    reason = "wired up in the constructor-cutover commit that replaces direct `state.renderer.*` access"
)]
pub(crate) fn with_renderer<R>(
    caller: &mut Caller<'_, HostState>,
    f: impl FnOnce(&mut dyn Renderer) -> R,
) -> Result<R, wasmi::Error> {
    let Some(mut ptr) = caller.data_mut().renderer_ptr else {
        return Err(wasmi::Error::new(
            "renderer accessed outside render scope (host import called from \
             init or on_params_update?)",
        ));
    };
    // SAFETY: `ptr` was installed by `WasmWidgetRuntime::with_renderer` on this
    // same thread; the wasmi dispatch guarantees no other host fn runs
    // concurrently, so no other `&mut Renderer` exists during this reborrow.
    let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
    Ok(f(renderer))
}
