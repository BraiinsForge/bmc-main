// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared protocol definitions for WASM widgets.
//! This crate contains constants and types shared between the SDK (WASM) and host.

pub mod animation;
pub mod arc;
pub mod colors;
pub mod display;
pub mod fill;
pub mod ids;
pub mod mesh;
pub mod nodes;
pub mod params;
pub mod svg;
pub mod system;
pub mod tags;
pub mod text;
pub mod version;
pub mod versioned_snapshot;

pub use animation::*;
pub use arc::*;
pub use colors::*;
pub use display::{DisplayShape, ViewportShape};
pub use fill::*;
pub use ids::*;
pub use mesh::*;
pub use nodes::*;
pub use svg::*;
pub use tags::*;
pub use text::*;
pub use version::*;
