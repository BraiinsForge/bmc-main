// Copyright (C) 2026  Braiins Systems s.r.o.

//! GPU-accelerated rendering backend (FemtoVG + cosmic-text).

pub mod bitmap;
pub mod mesh;
mod renderer;
mod sphere;
pub mod svg;
pub mod text;

pub mod builtin_icons {
    include!(concat!(env!("OUT_DIR"), "/builtin_icons.rs"));
}

pub use renderer::{FemtoVgRenderer, FemtovgImageId};
