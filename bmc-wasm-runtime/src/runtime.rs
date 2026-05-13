// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime — delegates to the wasmi backend.

mod backend;
mod background;
mod delivery;
mod imports;
mod memory;
mod time;

// Re-export `encode_params` so `HostState::encoded_params` (in `host_api`) can call it
// without `imports` itself becoming public.
pub(crate) use imports::params::encode_params;

pub use backend::{
    FetchInterceptor, FetchObserver, RenderStatus, RuntimeConfig, RuntimeResourceLimits,
    WasmWidgetRuntime,
};
