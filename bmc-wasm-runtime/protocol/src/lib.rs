// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared protocol definitions for WASM widgets.
//! This crate contains constants and types shared between the SDK (WASM) and host.

#![no_std]

pub mod animation;
pub mod colors;
pub mod format;
pub mod icon;
pub mod ids;
pub mod led;
pub mod mesh;
pub mod nodes;
pub mod scope;
pub mod tags;
pub mod text;
pub mod version;

pub use animation::*;
pub use colors::*;
pub use format::*;
pub use icon::*;
pub use ids::*;
pub use led::*;
pub use mesh::*;
pub use nodes::*;
pub use scope::*;
pub use tags::*;
pub use text::*;
pub use version::*;
