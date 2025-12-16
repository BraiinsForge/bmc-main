// Copyright (C) 2025  Braiins Systems s.r.o.

//! BMC Wayland Compositor
//!
//! A minimal Wayland compositor for displaying widget surfaces on the Braiins Deck.
//! Uses smithay with DRM/KMS backend for direct framebuffer access.

pub mod drm_backend;
pub mod render;
pub mod state;
