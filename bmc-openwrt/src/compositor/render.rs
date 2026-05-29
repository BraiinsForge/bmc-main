// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL/OpenGL ES rendering for split GPU/display architecture.

mod buffer_pool;
mod drm_output;
mod egl_context;
mod scanout_swizzle;

pub use buffer_pool::{BufferPool, ScanoutFormat};
pub use drm_output::DrmOutput;
pub use egl_context::EglContext;
pub use scanout_swizzle::ScanoutSwizzler;
