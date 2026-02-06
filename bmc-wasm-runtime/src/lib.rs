// Copyright (C) 2026  Braiins Systems s.r.o.

//! WebAssembly runtime for remote widget overlays.
//!
//! This crate provides a sandboxed WASM execution environment for remote widgets
//! to render interactive overlays on top of server-rendered images.
//!
//! See `docs/plan.md` for the full design document.

mod animation;
mod drawing;

// Re-export colors and color macro from protocol crate
pub mod colors {
    pub use bmc_wasm_protocol::colors::*;
}
pub use bmc_wasm_protocol::color;
mod host_api;
mod runtime;
pub mod tree;

pub mod components;
pub mod interaction;

pub use runtime::WasmWidgetRuntime;
