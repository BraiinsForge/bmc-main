// Copyright (C) 2026  Braiins Systems s.r.o.

//! GPU-accelerated rendering backend (FemtoVG + cosmic-text).

pub mod bitmap;
pub mod icons;
mod renderer;
mod sphere;
pub mod text;

pub mod builtin_icons {
    include!(concat!(env!("OUT_DIR"), "/builtin_icons.rs"));
}

pub use renderer::FemtoVgRenderer;
