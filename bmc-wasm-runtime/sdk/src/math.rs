// Copyright (C) 2026  Braiins Systems s.r.o.

//! 3D math utilities — re-exports `glam` types for widget use.
//!
//! Enabled by the `math-3d` feature. Provides:
//! - `glam::Quat`, `glam::Vec3`, `glam::Mat3`, etc.
//! - `From<glam::Quat>` / `Into<glam::Quat>` on `Orientation`

pub use glam::*;
