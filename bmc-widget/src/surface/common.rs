// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    os::fd::AsFd,
};

use anyhow::{Context, Result};
use wayland_backend::client::ObjectId;
use wayland_client::{
    Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};

use crate::egl::DmaBufInfo;

/// Re-export so widgets can match on setting variants without depending on
/// `bmc-widget-protocol` directly.
pub use bmc_widget_protocol::{LifecycleState, SettingUpdate};

/// Re-export the deadline-aware dispatch helper from [`crate::poll`] so the
/// surface clients can keep importing it as `super::common::poll_dispatch`.
pub use crate::poll::PollOutcome;
pub use crate::poll::poll_dispatch;

/// Mapping from `wl_buffer` object id to reusable buffer slot.
pub type BufferSlotMap = HashMap<ObjectId, usize>;

/// Set of tracked `wl_buffer` object ids released by the compositor.
pub type ReleasedBufferSet = HashSet<ObjectId>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasedBuffer {
    id: ObjectId,
    slot: usize,
}

impl ReleasedBuffer {
    #[must_use]
    pub(crate) fn new(id: ObjectId, slot: usize) -> Self {
        Self { id, slot }
    }

    #[must_use]
    pub fn slot(&self) -> usize {
        self.slot
    }

    #[must_use]
    pub fn matches(&self, buffer: &wl_buffer::WlBuffer) -> bool {
        self.id == buffer.id()
    }
}

/// Block until a Wayland event arrives, then dispatch all pending events.
///
/// Shared implementation used by both [`crate::surface::XdgSurfaceClient`] and
/// [`crate::surface::DeckWidgetSurfaceClient`].
pub(crate) fn blocking_dispatch_impl<S: 'static>(
    queue: &mut EventQueue<S>,
    state: &mut S,
) -> Result<()> {
    queue
        .blocking_dispatch(state)
        .context("Wayland dispatch failed")?;
    Ok(())
}

/// Destroy all cached `wl_buffer`s and return the number destroyed.
///
/// Shared implementation for
/// [`crate::surface::XdgSurfaceClient::invalidate_cached_buffers`] and
/// [`crate::surface::DeckWidgetSurfaceClient::invalidate_cached_buffers`].
///
/// This also removes pending release notifications for cached buffers that
/// were actually destroyed. Release notifications for unrelated tracked
/// buffers, such as host-owned minted buffers, remain pending.
#[expect(
    clippy::mutable_key_type,
    reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
)]
pub(crate) fn invalidate_cached_wl_buffers(
    cached_buffers: &mut [Option<wl_buffer::WlBuffer>],
    buffer_slots: &mut BufferSlotMap,
    released_buffers: &mut ReleasedBufferSet,
) -> u32 {
    let mut destroyed = 0_u32;
    for cached in cached_buffers {
        if let Some(buf) = cached.take() {
            unregister_wl_buffer_slot(buffer_slots, released_buffers, &buf.id());
            buf.destroy();
            destroyed += 1;
        }
    }
    if destroyed > 0 {
        tracing::debug!("Destroyed {destroyed} cached wl_buffer(s)");
    }
    destroyed
}

#[expect(
    clippy::mutable_key_type,
    reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
)]
pub(crate) fn invalidate_cached_wl_buffer_slots(
    cached_buffers: &mut [Option<wl_buffer::WlBuffer>],
    buffer_slots: &mut BufferSlotMap,
    released_buffers: &mut ReleasedBufferSet,
    slots: &[usize],
) -> u32 {
    let mut destroyed = 0_u32;
    for slot in slots {
        let Some(cached) = cached_buffers.get_mut(*slot) else {
            continue;
        };
        if let Some(buf) = cached.take() {
            unregister_wl_buffer_slot(buffer_slots, released_buffers, &buf.id());
            buf.destroy();
            destroyed += 1;
        }
    }
    if destroyed > 0 {
        tracing::debug!("Destroyed {destroyed} cached wl_buffer slot(s)");
    }
    destroyed
}

#[expect(
    clippy::mutable_key_type,
    reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
)]
pub fn unregister_wl_buffer_slot(
    buffer_slots: &mut BufferSlotMap,
    released_buffers: &mut ReleasedBufferSet,
    buffer_id: &ObjectId,
) -> Option<usize> {
    released_buffers.remove(buffer_id);
    buffer_slots.remove(buffer_id)
}

#[expect(
    clippy::mutable_key_type,
    reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
)]
pub fn record_released_buffer(
    buffer_slots: &BufferSlotMap,
    released_buffers: &mut ReleasedBufferSet,
    buffer_id: ObjectId,
) -> bool {
    if !buffer_slots.contains_key(&buffer_id) {
        return false;
    }
    released_buffers.insert(buffer_id);
    true
}

#[expect(
    clippy::mutable_key_type,
    reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
)]
pub fn drain_released_buffer_slots(
    buffer_slots: &BufferSlotMap,
    released_buffers: &mut ReleasedBufferSet,
) -> Vec<usize> {
    let mut released_slots = BTreeSet::new();
    for released in drain_released_buffers(buffer_slots, released_buffers) {
        released_slots.insert(released.slot());
    }
    released_slots.into_iter().collect()
}

#[expect(
    clippy::mutable_key_type,
    reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
)]
pub fn drain_released_buffers(
    buffer_slots: &BufferSlotMap,
    released_buffers: &mut ReleasedBufferSet,
) -> Vec<ReleasedBuffer> {
    let mut released = Vec::new();
    for buffer_id in std::mem::take(released_buffers) {
        if let Some(&slot) = buffer_slots.get(&buffer_id) {
            released.push(ReleasedBuffer::new(buffer_id, slot));
        }
    }
    released
}

/// Attach buffer, damage, optionally request frame callback, and commit.
///
/// Shared implementation for buffer submission on both surface client types.
#[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
pub fn submit_buffer_to_surface<S>(
    surface: &wl_surface::WlSurface,
    qh: &QueueHandle<S>,
    buffer: &wl_buffer::WlBuffer,
    info: &DmaBufInfo,
    request_frame: bool,
) where
    S: Dispatch<wl_callback::WlCallback, ()> + 'static,
{
    surface.attach(Some(buffer), 0, 0);
    surface.damage_buffer(0, 0, info.width as i32, info.height as i32);

    if request_frame {
        surface.frame(qh, ());
    }

    surface.commit();
}

/// Typed events from the compositor to a widget.
#[derive(Debug, Clone)]
pub enum WidgetEvent {
    /// A setting was updated at runtime.
    Setting(bmc_widget_protocol::SettingUpdate),
    ParamUpdate(serde_json::Map<String, serde_json::Value>),
    /// Bound credential slots; names accounts, carries no secret.
    CredentialsUpdate(serde_json::Map<String, serde_json::Value>),
    /// Secret values held by this process on the widget's behalf,
    /// never handed to the widget itself.
    SecretsUpdate(bmc_widget_protocol::CredentialSecrets),
    /// The compositor requests graceful shutdown.
    Shutdown,
    /// Touch point down (standard Wayland `wl_touch`).
    TouchDown {
        id: i32,
        x: f64,
        y: f64,
    },
    /// Touch point moved (standard Wayland `wl_touch`).
    TouchMotion {
        id: i32,
        x: f64,
        y: f64,
    },
    /// Touch point lifted (standard Wayland `wl_touch`).
    TouchUp {
        id: i32,
    },
    /// Touch sequence cancelled (standard Wayland `wl_touch`).
    TouchCancel,
    /// Compositor published a new lifecycle state for this widget.
    Lifecycle(bmc_widget_protocol::LifecycleState),
    /// Automatic scene cycling will transition this widget on-screen soon.
    TransitionIncoming,
}

/// Common interface for widget surface clients.
///
/// Abstracts over XDG toplevel (standalone) and `deck_widget` (production)
/// backends so widget render loops can work with either.
pub trait WidgetSurface {
    /// Whether the event loop should keep running.
    fn running(&self) -> bool;
    /// Request shutdown (sets running to false).
    fn request_shutdown(&mut self);
    /// Current surface width in pixels.
    fn width(&self) -> u32;
    /// Current surface height in pixels.
    fn height(&self) -> u32;
    /// Whether a resize occurred since last acknowledged.
    fn take_size_changed(&mut self) -> bool;
    /// Whether a frame callback or timeout has fired -- widget should render.
    /// Unlike [`take_render_requested`](Self::take_render_requested), this
    /// does not clear the flag.
    fn needs_render(&self) -> bool;
    /// Whether a frame callback or timeout has fired -- widget should render.
    /// Clears the flag so subsequent calls return `false` until the next event.
    fn take_render_requested(&mut self) -> bool;
    /// Signal that a render is needed (e.g. after poll timeout).
    fn mark_needs_render(&mut self);
    /// Frame counter (wrapping).
    fn frame_count(&self) -> u32;
    /// Block until a Wayland event arrives, then dispatch.
    fn blocking_dispatch(&mut self) -> anyhow::Result<()>;
    /// Poll for events with timeout, then dispatch. -1 blocks, 0 non-blocking.
    fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<PollOutcome>;
    /// Request the first frame callback (call once before the event loop).
    fn request_frame(&self);
    /// Submit a DMA-BUF frame for a reusable buffer slot.
    ///
    /// Surface clients cache one `wl_buffer` per slot and reuse it across
    /// frames. Call [`invalidate_cached_buffers`](Self::invalidate_cached_buffers)
    /// when the underlying DMA-BUF objects are recreated, e.g. on resize.
    fn submit_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> anyhow::Result<()>;
    /// Invalidate cached `wl_buffer`s (call on resize).
    fn invalidate_cached_buffers(&mut self);
    /// Invalidate selected cached `wl_buffer` slots.
    fn invalidate_cached_buffer_slots(&mut self, slots: &[usize]);
    /// Drain slot ids whose submitted `wl_buffer`s were released.
    ///
    /// Returns deduplicated ids since the previous drain call.
    fn drain_released_slots(&mut self) -> Vec<usize>;
    /// Drain pending compositor events.
    fn drain_events(&mut self) -> Vec<WidgetEvent>;
}

/// Create a `wl_buffer` from DMA-BUF info using the `linux-dmabuf` protocol.
#[must_use]
#[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
pub fn create_buffer_from_dmabuf<S>(
    linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    info: &DmaBufInfo,
    qh: &QueueHandle<S>,
) -> wl_buffer::WlBuffer
where
    S: Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()>
        + Dispatch<wl_buffer::WlBuffer, ()>
        + 'static,
{
    let params = linux_dmabuf.create_params(qh, ());

    let modifier: u64 = info.modifier.into();
    let modifier_hi = (modifier >> 32) as u32;
    let modifier_lo = (modifier & 0xFFFF_FFFF) as u32;

    params.add(
        info.fd.as_fd(),
        0, // plane index
        0, // offset
        info.stride,
        modifier_hi,
        modifier_lo,
    );

    let buffer = params.create_immed(
        info.width as i32,
        info.height as i32,
        info.format as u32,
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    );
    params.destroy();

    buffer
}

/// Generate common `Dispatch` impls for a surface state type.
///
/// These impls are identical for both XDG and deck_widget surface clients.
/// The state type must have `frame_count: u32` and `needs_render: bool` fields.
macro_rules! impl_common_dispatch {
    ($state:ty) => {
        impl wayland_client::Dispatch<
            wayland_client::protocol::wl_compositor::WlCompositor,
            (),
        > for $state
        {
            fn event(
                _: &mut Self,
                _: &wayland_client::protocol::wl_compositor::WlCompositor,
                _: wayland_client::protocol::wl_compositor::Event,
                (): &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
            }
        }

        impl wayland_client::Dispatch<
            wayland_client::protocol::wl_surface::WlSurface,
            (),
        > for $state
        {
            fn event(
                _: &mut Self,
                _: &wayland_client::protocol::wl_surface::WlSurface,
                _: wayland_client::protocol::wl_surface::Event,
                (): &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
            }
        }

        impl wayland_client::Dispatch<
            wayland_client::protocol::wl_callback::WlCallback,
            (),
        > for $state
        {
            fn event(
                state: &mut Self,
                _: &wayland_client::protocol::wl_callback::WlCallback,
                event: wayland_client::protocol::wl_callback::Event,
                (): &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
                if let wayland_client::protocol::wl_callback::Event::Done { .. } = event {
                    state.frame_count = state.frame_count.wrapping_add(1);
                    state.needs_render = true;
                }
            }
        }

        impl wayland_client::Dispatch<
            wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
            (),
        > for $state
        {
            fn event(
                _: &mut Self,
                _: &wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
                event: wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::Event,
                (): &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
                match event {
                    wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::Event::Format { format } => {
                        tracing::trace!("DMA-BUF format: 0x{format:08x}");
                    }
                    wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::Event::Modifier {
                        format,
                        modifier_hi,
                        modifier_lo,
                    } => {
                        let modifier =
                            (u64::from(modifier_hi) << 32) | u64::from(modifier_lo);
                        tracing::trace!(
                            "DMA-BUF format 0x{format:08x} modifier 0x{modifier:016x}"
                        );
                    }
                    _ => {}
                }
            }
        }

        impl wayland_client::Dispatch<
            wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
            (),
        > for $state
        {
            fn event(
                _: &mut Self,
                _: &wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
                event: wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::Event,
                (): &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
                if let wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::Event::Failed = event {
                    tracing::error!("DMA-BUF buffer creation failed");
                }
            }
        }
    };
}

pub(crate) use impl_common_dispatch;

#[cfg(test)]
mod tests {
    use wayland_client::protocol::wl_buffer;

    use wayland_backend::client::ObjectId;

    use super::{
        BufferSlotMap, ReleasedBufferSet, drain_released_buffer_slots,
        invalidate_cached_wl_buffers, record_released_buffer, unregister_wl_buffer_slot,
    };

    #[test]
    fn invalidate_cached_wl_buffers_keeps_released_buffers_when_no_cached_buffers_destroyed() {
        let mut cached_buffers: Vec<Option<wl_buffer::WlBuffer>> = Vec::new();
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
        )]
        let mut buffer_slots = BufferSlotMap::new();
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashSet key"
        )]
        let mut released_buffers = ReleasedBufferSet::from([ObjectId::null()]);

        let destroyed = invalidate_cached_wl_buffers(
            &mut cached_buffers,
            &mut buffer_slots,
            &mut released_buffers,
        );

        assert_eq!(destroyed, 0);
        assert!(released_buffers.contains(&ObjectId::null()));
    }

    #[test]
    fn unregister_wl_buffer_slot_removes_only_matching_pending_buffer() {
        let buffer_id = ObjectId::null();
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
        )]
        let mut buffer_slots = BufferSlotMap::from([(buffer_id.clone(), 7)]);
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashSet key"
        )]
        let mut released_buffers = ReleasedBufferSet::from([buffer_id.clone()]);

        let removed =
            unregister_wl_buffer_slot(&mut buffer_slots, &mut released_buffers, &buffer_id);

        assert_eq!(removed, Some(7));
        assert!(!buffer_slots.contains_key(&buffer_id));
        assert!(released_buffers.is_empty());
    }

    #[test]
    fn drain_released_buffer_slots_resolves_pending_buffers_to_slots() {
        let buffer_id = ObjectId::null();
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
        )]
        let buffer_slots = BufferSlotMap::from([(buffer_id.clone(), 7)]);
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashSet key"
        )]
        let mut released_buffers = ReleasedBufferSet::from([buffer_id]);

        let released_slots = drain_released_buffer_slots(&buffer_slots, &mut released_buffers);

        assert_eq!(released_slots, vec![7]);
        assert!(released_buffers.is_empty());
    }

    #[test]
    fn record_released_buffer_tracks_known_buffer() {
        let tracked_id = ObjectId::null();
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashMap key"
        )]
        let buffer_slots = BufferSlotMap::from([(tracked_id.clone(), 7)]);
        #[expect(
            clippy::mutable_key_type,
            reason = "ObjectId has interior mutability but is safe to use as a HashSet key"
        )]
        let mut released_buffers = ReleasedBufferSet::new();

        assert!(record_released_buffer(
            &buffer_slots,
            &mut released_buffers,
            tracked_id.clone(),
        ));

        assert_eq!(released_buffers, ReleasedBufferSet::from([tracked_id]));
    }
}
