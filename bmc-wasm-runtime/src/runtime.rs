// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime — delegates to the wasmi backend.

mod backend;
mod background;
mod delivery;
mod imports;
mod memory;
mod time;

// Re-export the `ParamsSnapshot` newtype so `HostState` (in `host_api`)
// can compose it with `VersionedSnapshotCache` without `imports` itself becoming public.
//
// The underlying `encode_params` stays module-private — it's reached
// through `<ParamsSnapshot as WireEncode>::encode` from production code,
// and through `super::encode_params` from this module's tests.
pub(crate) use imports::params::ParamsSnapshot;

pub use backend::{
    FetchInterceptor, FetchObserver, RenderStatus, RuntimeConfig, RuntimeResourceLimits,
    WasmWidgetRuntime,
};
