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
/// # Invariants
/// Soundness depends on the parked pointer being valid and exclusively
/// borrowed for the duration of this call. The upstream caller of
/// `WasmWidgetRuntime::with_renderer` is on the hook for that via the
/// documented `addr_of_mut!` contract; the parked `NonNull` is only ever
/// dereferenced from here (and from the companion helper below), so this is
/// the single host-side reborrow point. Single-threaded wasmi dispatch plus
/// the no-async-yield-inside-host-fn rule ensure only one host import runs
/// at a time on the parking thread, so no other `&mut Renderer` exists
/// during this reborrow.
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

/// Reborrow the parked renderer alongside the current `&mut HostState`.
///
/// Returns the same wasmi trap as [`with_renderer`] when called outside a
/// render scope, with the same diagnostic string so the import author sees
/// the same hint regardless of which helper they reach for.
///
/// # Invariants
/// Same as [`with_renderer`]. The materialized `&mut dyn Renderer`
/// originates from the caller-owned renderer (parked via
/// `WasmWidgetRuntime::with_renderer`), so it shares no Tree-Borrows
/// ancestry with the returned `&mut HostState`.
pub(crate) fn with_renderer_and_state<R>(
    caller: &mut Caller<'_, HostState>,
    f: impl FnOnce(&mut dyn Renderer, &mut HostState) -> R,
) -> Result<R, wasmi::Error> {
    let state: &mut HostState = caller.data_mut();
    let Some(mut ptr) = state.renderer_ptr else {
        return Err(wasmi::Error::new(
            "renderer accessed outside render scope (host import called from \
             init or on_params_update?)",
        ));
    };
    // SAFETY: `ptr` was installed by `WasmWidgetRuntime::with_renderer`; the
    // materialized `&mut dyn Renderer` originates from the caller-owned
    // renderer, not from `state`, so the two `&mut` references do not alias.
    let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
    Ok(f(renderer, state))
}
