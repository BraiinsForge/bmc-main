// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL/OpenGL ES rendering for split GPU/display architecture.

mod buffer_pool;
mod drm_output;
mod egl_context;

pub use buffer_pool::BufferPool;
pub use drm_output::DrmOutput;
pub use egl_context::EglContext;
