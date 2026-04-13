// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime — delegates to the wasmi backend.

mod backend;
mod background;
mod delivery;
mod imports;
mod memory;
mod time;

pub use backend::{
    FetchInterceptor, FetchObserver, RenderStatus, RuntimeConfig, RuntimeResourceLimits,
    WasmWidgetRuntime,
};
