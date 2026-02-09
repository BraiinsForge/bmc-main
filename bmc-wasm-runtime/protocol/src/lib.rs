// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared protocol definitions for WASM widgets.
//! This crate contains constants and types shared between the SDK (WASM) and host.

#![no_std]

pub mod animation;
pub mod colors;
pub mod icon;
pub mod nodes;
pub mod text;

pub use animation::*;
pub use colors::*;
pub use icon::*;
pub use nodes::*;
pub use text::*;
