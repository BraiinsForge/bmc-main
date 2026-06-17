// Copyright (C) 2025  Braiins Systems s.r.o.

#[cfg(feature = "gpu")]
pub mod egl;
mod poll;
#[cfg(feature = "gpu")]
pub mod surface;
pub mod wayland;

pub use wayland::{WaylandError, WidgetEventHandler, WidgetProtocolClient};

pub use poll::poll_dispatch;

#[cfg(feature = "gpu")]
pub use surface::common::{
    BufferSlotMap, ReleasedBuffer, ReleasedBufferSet, create_buffer_from_dmabuf,
    drain_released_buffer_slots, drain_released_buffers, record_released_buffer,
    submit_buffer_to_surface, unregister_wl_buffer_slot,
};
