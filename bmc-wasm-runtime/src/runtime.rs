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

//! WASM runtime — delegates to the wasmi backend.

mod backend;
mod background;
mod delivery;
mod imports;
mod memory;
mod time;

pub(crate) use background::build_fetch_agent;

// Re-export the `ParamsSnapshot` newtype so `HostState` (in `host_api`)
// can compose it with `VersionedSnapshotCache` without `imports` itself becoming public.
//
// The underlying `encode_params` stays module-private — it's reached
// through `<ParamsSnapshot as WireEncode>::encode` from production code,
// and through `super::encode_params` from this module's tests.
pub use imports::credentials::{BoundCredential, CredentialView};
pub(crate) use imports::params::ParamsSnapshot;

pub use backend::DisplayInfo as RuntimeDisplayInfo;
pub use backend::{
    FetchInterceptor, FetchObserver, RenderStatus, RendererAssetRestorationObservation,
    RendererAssetSuspensionObservation, RuntimeConfig, RuntimeResourceLimits, WasmWidgetModule,
    WasmWidgetRuntime,
};
