// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use bmc_widget::egl::{DmaBufInfo, EglContext};

use crate::gpu::OverlayRenderTarget;
use crate::overlay::{SystemOverlay, resolved_configured_size};
use crate::surface::LayerSurfaceClient;

const MIN_INTER_FRAME: Duration = Duration::from_millis(8);

/// A system overlay hosted inside another process (e.g. bmc-wasm-host). It owns
/// its Wayland connection and export buffers but borrows the host's renderer
/// and GPU stack for the actual frame, which the host orchestrates.
#[expect(missing_debug_implementations)]
pub struct HostedOverlay {
    overlay: Box<dyn SystemOverlay>,
    client: LayerSurfaceClient,
    target: OverlayRenderTarget,
    config_size: (u32, u32),
    size: (u32, u32),
    last_render: Option<Instant>,
    next_wake: Option<Instant>,
    wants_render: bool,
    /// Set after a non-fatal render/export/attach error. A failed overlay is
    /// dropped from the host's list (terminal) — it must NOT keep `wants_render`
    /// latched, or it would busy-retry-and-log every pass.
    failed: bool,
}

impl HostedOverlay {
    /// Connect the overlay's own Wayland client and allocate its export buffers
    /// from the host's EGL context.
    pub fn connect(mut overlay: Box<dyn SystemOverlay>, egl: &EglContext) -> anyhow::Result<Self> {
        let config = overlay.layer_config();
        let config_size = config.size;
        let mut client = LayerSurfaceClient::connect(&config)?;
        let size = resolved_configured_size(config_size, client.size());
        let target = OverlayRenderTarget::new(egl, size.0, size.1)?;
        let wants_render = client.take_needs_render();
        overlay.init();
        Ok(Self {
            overlay,
            client,
            target,
            config_size,
            size,
            last_render: None,
            next_wake: None,
            wants_render,
            failed: false,
        })
    }

    #[must_use]
    pub fn connection_fd(&self) -> RawFd {
        self.client.connection_fd()
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Drain Wayland events, deliver touch, and pick up surface-dirty.
    pub fn dispatch(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        // Non-blocking: the host already polled the fd. poll_dispatch(0) runs the
        // correct prepare_read -> poll(0) -> read -> dispatch sequence.
        self.client.poll_dispatch(0)?;
        for ev in self.client.drain_touch() {
            self.overlay.on_touch(ev);
        }
        for released in self.client.drain_released_buffers() {
            self.target.mark_released_buffer(&released);
        }
        if let Some(configured_size) = self.client.take_configured_size_change() {
            let size = resolved_configured_size(self.config_size, configured_size);
            if self.size != size {
                self.target.resize(egl, &mut self.client, size.0, size.1);
                self.size = size;
            }
            self.wants_render = true;
        }
        if self.client.take_needs_render() {
            self.wants_render = true;
        }
        Ok(())
    }

    /// Run background work; updates the next-wake hint.
    pub fn tick(&mut self, now: Instant) {
        let outcome = self.overlay.tick(now);
        if outcome.wants_render {
            self.wants_render = true;
        }
        self.next_wake = outcome.next_wake;
    }

    /// Whether the overlay should be rendered this pass (respecting the
    /// inter-frame floor).
    #[must_use]
    pub fn needs_render(&self, now: Instant) -> bool {
        let inter_frame_ok = self
            .last_render
            .is_none_or(|t| now.duration_since(t) >= MIN_INTER_FRAME);
        !self.failed
            && self.wants_render
            && inter_frame_ok
            && self.client.running()
            && self.target.available()
    }

    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Mark a terminal non-fatal failure. Clears the latched render so it cannot retry.
    pub fn mark_failed(&mut self) {
        self.failed = true;
        self.wants_render = false;
    }

    /// Release GL/EGL/GBM resources before this overlay is dropped. Must be
    /// called for failed overlays, closed overlays, and at host shutdown —
    /// OverlayRenderTarget/DoubleBufferState do not free on Drop.
    pub fn shutdown(&mut self, egl: &EglContext) {
        self.target.destroy(egl);
    }

    #[must_use]
    pub fn next_wake(&self) -> Option<Instant> {
        self.next_wake
    }

    /// Max time the host may sleep on this overlay's behalf. `Some(ZERO)` means
    /// poll immediately. Covers the throttled case so the host wakes at the 8 ms
    /// boundary to run a latched-but-throttled render.
    #[must_use]
    pub fn poll_timeout(&self, now: Instant) -> Option<Duration> {
        let tick = self.next_wake.map(|t| t.saturating_duration_since(now));
        if self.failed || !self.wants_render || !self.client.running() {
            return tick;
        }
        let inter_frame_remaining = self
            .last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(now.duration_since(t)))
            .filter(|d| !d.is_zero());
        match inter_frame_remaining {
            Some(d) => Some(tick.map_or(d, |t| d.min(t))),
            None if self.target.available() => Some(Duration::ZERO),
            None => tick,
        }
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.client.running()
    }

    pub fn overlay_mut(&mut self) -> &mut dyn SystemOverlay {
        &mut *self.overlay
    }

    pub fn target_mut(&mut self) -> &mut OverlayRenderTarget {
        &mut self.target
    }

    /// Attach an exported buffer: mint+cache its `wl_buffer`, commit it to the
    /// layer surface, and mark the slot in-flight until the compositor releases
    /// it. Borrows target and client together (legal as one `&mut self`).
    pub fn submit_exported(&mut self, dmabuf: &DmaBufInfo, slot: usize) -> anyhow::Result<()> {
        let wl_buffer = self
            .target
            .wl_buffer_for_slot(&mut self.client, dmabuf, slot)?;
        self.client
            .submit_buffer_with_wl_buffer(dmabuf, &wl_buffer)?;
        self.client.flush()?;
        self.target.mark_presented(slot);
        Ok(())
    }

    /// Mark a render as completed at `now` and clear the dirty flag.
    pub fn mark_rendered(&mut self, now: Instant) {
        self.last_render = Some(now);
        self.wants_render = false;
    }
}
