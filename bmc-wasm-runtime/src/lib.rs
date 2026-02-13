// Copyright (C) 2026  Braiins Systems s.r.o.

//! WebAssembly runtime for remote widget overlays.
//!
//! This crate provides a sandboxed WASM execution environment for remote widgets
//! to render interactive overlays on top of server-rendered images.
//!
//! See `docs/plan.md` for the full design document.
//!
//! # Safety
//!
//! All remaining `unsafe` in the runtime and SDK is forced by external APIs:
//!
//! - **OpenGL** — glow/glutin require unsafe for every GL call, context creation,
//!   and function-pointer loading. FemtoVG's `OpenGl::new_from_function` inherits this.
//! - **WASM host FFI** — `unsafe extern "C"` blocks declaring host imports and
//!   `#[unsafe(no_mangle)]` on WASM exports are the only way to cross the
//!   host↔guest boundary.
//! - **WASM allocator protocol** — `__alloc`/`__dealloc` and `__on_fetch_response`
//!   use `Vec::from_raw_parts` to transfer ownership of host-allocated buffers.

mod animation;
pub mod gpu;
pub mod renderer;

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

pub use runtime::{RenderStatus, WasmWidgetRuntime};
