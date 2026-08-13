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

//! Guest import registration for the WASM runtime.

mod audio;
pub(crate) mod credentials;
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
use bmc_wasm_protocol::{PACKAGE_ASSET_REF_LEN, PackageAssetId, PackageAssetKind, PackageAssetRef};
use wasmi::{Caller, Linker};

use crate::host_api::HostState;

use super::memory::read_bytes;

pub(super) fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
    render::register(linker)?;
    network::register(linker)?;
    data::register(linker)?;
    system::register(linker)?;
    audio::register(linker)?;
    led::register(linker)?;
    eviction::register(linker)?;
    params::register(linker)?;
    credentials::register(linker)?;
    Ok(())
}

fn read_package_ref(
    caller: &Caller<'_, HostState>,
    ptr: u32,
    expected_kind: PackageAssetKind,
) -> Option<PackageAssetId> {
    let bytes = read_bytes(
        caller,
        ptr,
        u32::try_from(PACKAGE_ASSET_REF_LEN).expect("BUG: package reference length fits u32"),
    )?;
    let package_ref = PackageAssetRef::try_from(bytes.as_slice()).ok()?;
    (package_ref.kind() == expected_kind).then(|| package_ref.id())
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
/// dereferenced only by the helpers in this module. Single-threaded wasmi dispatch plus
/// the no-async-yield-inside-host-fn rule ensure only one host import runs
/// at a time on the parking thread, so no other `&mut Renderer` exists
/// during this reborrow.
pub(crate) fn with_renderer<R>(
    caller: &mut Caller<'_, HostState>,
    f: impl FnOnce(&mut dyn Renderer) -> R,
) -> Result<R, wasmi::Error> {
    let state = caller.data_mut();
    let Some(mut ptr) = state.renderer_ptr else {
        return Err(wasmi::Error::new(
            "renderer accessed outside render scope (host import called from \
             init or on_params_update?)",
        ));
    };
    state.mark_renderer_accessed();
    // SAFETY: `ptr` was installed by `WasmWidgetRuntime::with_renderer` on this
    // same thread; the wasmi dispatch guarantees no other host fn runs
    // concurrently, so no other `&mut Renderer` exists during this reborrow.
    let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
    Ok(f(renderer))
}

/// Reborrow the parked renderer for a query that cannot issue GPU work.
pub(crate) fn with_renderer_readonly<R>(
    caller: &mut Caller<'_, HostState>,
    f: impl FnOnce(&dyn Renderer) -> R,
) -> Result<R, wasmi::Error> {
    let state = caller.data_mut();
    let Some(ptr) = state.renderer_ptr else {
        return Err(wasmi::Error::new(
            "renderer accessed outside render scope (host import called from \
             init or on_params_update?)",
        ));
    };
    // SAFETY: the same invariants as `with_renderer` apply. The shared
    // reborrow prevents this helper's closure from mutating the renderer.
    let renderer: &dyn Renderer = unsafe { ptr.as_ref() };
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
    state.mark_renderer_accessed();
    // SAFETY: `ptr` was installed by `WasmWidgetRuntime::with_renderer`; the
    // materialized `&mut dyn Renderer` originates from the caller-owned
    // renderer, not from `state`, so the two `&mut` references do not alias.
    let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
    Ok(f(renderer, state))
}
