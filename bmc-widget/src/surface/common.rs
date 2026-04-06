use std::os::fd::{AsFd, AsRawFd};

use anyhow::{Context, Result};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};

use crate::egl::DmaBufInfo;

/// Re-export so widgets can match on setting variants without depending on
/// `bmc-widget-protocol` directly.
pub use bmc_widget_protocol::SettingUpdate;

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

/// Poll for Wayland events with a timeout, then dispatch pending events.
///
/// Returns `Ok(true)` on all normal paths (events read, timeout, or
/// already-queued events dispatched). Returns `Ok(false)` only if a
/// non-fatal `EAGAIN`/`EWOULDBLOCK` occurs in `poll(2)` or
/// `ReadEventsGuard::read()`; pending events are still dispatched. A
/// timeout of `-1` blocks indefinitely; `0` is non-blocking.
///
/// This follows the `prepare_read -> poll -> read/cancel -> dispatch_pending`
/// pattern required by `wayland-client`.
pub(crate) fn poll_dispatch<S: 'static>(
    conn: &Connection,
    queue: &mut EventQueue<S>,
    state: &mut S,
    timeout_ms: i32,
) -> Result<bool> {
    conn.flush()?;

    let read_guard = queue.prepare_read();
    let mut read_would_block = false;

    match read_guard {
        None => {
            // Events already queued -- just dispatch them
            queue
                .dispatch_pending(state)
                .context("Wayland dispatch failed")?;
            return Ok(true);
        }
        Some(guard) => {
            let fd = conn.as_fd();
            let mut pollfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };

            let poll_ret = unsafe { libc::poll(&raw mut pollfd, 1, timeout_ms) };

            match poll_ret.cmp(&0) {
                std::cmp::Ordering::Greater => match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(err))
                        if err.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // Non-fatal race: poll reported readability but no
                        // event was available by the time we read.
                        read_would_block = true;
                    }
                    Err(err) => return Err(err).context("Wayland socket read failed"),
                },
                std::cmp::Ordering::Equal => {
                    // Timeout -- cancel read
                    drop(guard);
                }
                std::cmp::Ordering::Less => {
                    // Error
                    let err = std::io::Error::last_os_error();
                    drop(guard);
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "all other io::ErrorKind variants are fatal"
                    )]
                    match err.kind() {
                        std::io::ErrorKind::Interrupted => {
                            // EINTR -- not fatal, just dispatch pending
                        }
                        std::io::ErrorKind::WouldBlock => {
                            // EAGAIN -- non-fatal, dispatch pending and
                            // signal caller via Ok(false)
                            queue
                                .dispatch_pending(state)
                                .context("Wayland dispatch failed")?;
                            return Ok(false);
                        }
                        _ => {
                            return Err(err).context("poll(2) on Wayland fd failed");
                        }
                    }
                }
            }
        }
    }

    queue
        .dispatch_pending(state)
        .context("Wayland dispatch failed")?;

    if read_would_block {
        return Ok(false);
    }

    Ok(true)
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
    /// The compositor requests graceful shutdown.
    Shutdown,
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
    /// Number of per-frame buffers currently held by the compositor.
    ///
    /// This is meaningful for backends that create a fresh `wl_buffer` per
    /// submit and destroy it on `Release`.
    ///
    /// Cached-buffer backends reuse the same small set of `wl_buffer`s across
    /// frames and rely on frame callbacks for backpressure instead, so this may
    /// remain `0` even while frames continue to be presented.
    fn pending_buffer_count(&self) -> u32;
    /// Whether more frames can be submitted without exceeding the limit.
    ///
    /// For per-frame buffer backends this typically compares
    /// [`pending_buffer_count`](Self::pending_buffer_count) with `max_pending`.
    ///
    /// Cached-buffer backends may return `true` unconditionally because the
    /// compositor backpressures them through frame callbacks rather than
    /// per-frame in-flight buffer accumulation.
    fn can_submit_frame(&self, max_pending: u32) -> bool;
    /// Frame counter (wrapping).
    fn frame_count(&self) -> u32;
    /// Block until a Wayland event arrives, then dispatch.
    fn blocking_dispatch(&mut self) -> anyhow::Result<()>;
    /// Poll for events with timeout, then dispatch. -1 blocks, 0 non-blocking.
    fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<bool>;
    /// Request the first frame callback (call once before the event loop).
    fn request_frame(&self);
    /// Submit a DMA-BUF frame. Optionally request a frame callback.
    fn commit_buffer(&mut self, info: &DmaBufInfo, request_frame: bool) -> anyhow::Result<()>;
    /// Submit a DMA-BUF frame using cached `wl_buffer` for a double-buffer slot.
    fn commit_cached_buffer(
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
