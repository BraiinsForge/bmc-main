// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::fd::AsFd;

use anyhow::{Context, Result};
use wayland_client::{
    Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};

use crate::egl::DmaBufInfo;

/// Re-export so widgets can match on setting variants without depending on
/// `bmc-widget-protocol` directly.
pub use bmc_widget_protocol::SettingUpdate;

/// Re-export the deadline-aware dispatch helper from [`crate::poll`] so the
/// surface clients can keep importing it as `super::common::poll_dispatch`.
/// The non-gated [`crate::poll`] module is the single owner of this logic
/// (also used by `crate::wayland::WidgetProtocolClient::wait_for_configure`).
pub use crate::poll::PollOutcome;
pub(crate) use crate::poll::poll_dispatch;

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
pub(crate) fn invalidate_cached_wl_buffers(
    cached_buffers: &mut [Option<wl_buffer::WlBuffer>],
) -> u32 {
    let mut destroyed = 0_u32;
    for cached in cached_buffers {
        if let Some(buf) = cached.take() {
            buf.destroy();
            destroyed += 1;
        }
    }
    if destroyed > 0 {
        tracing::debug!("Destroyed {destroyed} cached wl_buffer(s)");
    }
    destroyed
}

/// Attach buffer, damage, optionally request frame callback, and commit.
///
/// Shared implementation for buffer submission on both surface client types.
#[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
pub(crate) fn submit_buffer_to_surface<S>(
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
}

/// Common interface for widget surface clients.
///
/// Abstracts over XDG toplevel (standalone) and `deck_widget_v1` (production)
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
    /// Drain pending compositor events.
    fn drain_events(&mut self) -> Vec<WidgetEvent>;
}

/// Create a `wl_buffer` from DMA-BUF info using the `linux-dmabuf` protocol.
#[must_use]
#[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
pub(crate) fn create_buffer_from_dmabuf<S>(
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
