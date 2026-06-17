// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Wayland surface client with DMA-BUF buffer management.
//!
//! Provides [`XdgSurfaceClient`] -- a turnkey Wayland client that connects to
//! the compositor, creates an XDG toplevel surface, and manages DMA-BUF buffer
//! submission via `zwp_linux_dmabuf_v1`. Widgets only need to implement their
//! render loop on top.

pub(crate) mod common;
mod deck_widget;
mod xdg;

pub use common::{
    BufferSlotMap, LifecycleState, PollOutcome, ReleasedBuffer, ReleasedBufferSet, SettingUpdate,
    WidgetEvent, WidgetSurface, create_buffer_from_dmabuf, drain_released_buffer_slots,
    drain_released_buffers, poll_dispatch, record_released_buffer, submit_buffer_to_surface,
    unregister_wl_buffer_slot,
};
pub use deck_widget::{
    DeckWidgetEvent, DeckWidgetSurfaceClient, DeckWidgetSurfaceState, InitialState,
};
pub use xdg::{XdgSurfaceClient, XdgSurfaceState};
