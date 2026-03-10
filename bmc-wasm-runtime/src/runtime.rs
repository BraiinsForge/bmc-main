// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime — delegates to the wasmi backend.

#[path = "runtime_wasmi.rs"]
mod backend;

pub use backend::{RenderStatus, WasmWidgetRuntime};
