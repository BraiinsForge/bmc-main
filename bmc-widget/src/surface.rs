// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Wayland surface client with DMA-BUF buffer management.
//!
//! Provides [`XdgSurfaceClient`] -- a turnkey Wayland client that connects to
//! the compositor, creates an XDG toplevel surface, and manages DMA-BUF buffer
//! submission via `zwp_linux_dmabuf_v1`. Widgets only need to implement their
//! render loop on top.

mod common;
mod deck_widget;
mod xdg;

pub use common::{SettingUpdate, WidgetEvent, WidgetSurface};
pub use deck_widget::{DeckWidgetEvent, DeckWidgetSurfaceClient, DeckWidgetSurfaceState};
pub use xdg::{XdgSurfaceClient, XdgSurfaceState};
