// Copyright (C) 2025  Braiins Systems s.r.o.

//! WebAssembly runtime for remote widget overlays.
//!
//! This crate provides a sandboxed WASM execution environment for remote widgets
//! to render interactive overlays on top of server-rendered images.
//!
//! See `docs/plan.md` for the full design document.

pub mod colors;
mod drawing;
mod host_api;
mod runtime;
pub mod tree;

pub mod components;
pub mod interaction;

pub use runtime::WasmWidgetRuntime;
