// Copyright (C) 2026  Braiins Systems s.r.o.

//! `bmc-wasm-host` library surface, re-exported for integration tests.

pub mod cache_gc;
pub(crate) mod control;
pub mod host;
pub mod lifecycle;
pub mod logging;
pub mod main_loop;
mod overlays;
pub mod render_target;
pub mod slot;
pub mod startup;
