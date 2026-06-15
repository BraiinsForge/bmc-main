// Copyright (C) 2026  Braiins Systems s.r.o.

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use bmc_widget::egl::{DmaBufInfo, EglContext};

use crate::gpu::OverlayRenderTarget;
use crate::overlay::{SystemOverlay, resolved_configured_size};
use crate::surface::LayerSurfaceClient;

const MIN_INTER_FRAME: Duration = Duration::from_millis(8);

#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct render-gate predicate; a flags enum would be less readable at the single call site"
)]
#[derive(Debug, Clone, Copy)]
struct RenderGate {
    failed: bool,
    visible: bool,
    mapped: bool,
    wants_render: bool,
    inter_frame_ok: bool,
    client_running: bool,
    target_available: bool,
}

#[must_use]
fn overlay_needs_render(gate: RenderGate) -> bool {
    let wants = gate.wants_render || (gate.visible && !gate.mapped);
    !gate.failed
        && gate.visible
        && wants
        && gate.inter_frame_ok
        && gate.client_running
        && gate.target_available
}

#[must_use]
fn overlay_needs_hide(mapped: bool, visible: bool) -> bool {
    mapped && !visible
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResizeTransition {
    unmap_before_resize: bool,
    mapped_after_resize: bool,
}

#[must_use]
fn resize_transition(mapped: bool) -> ResizeTransition {
    ResizeTransition {
        unmap_before_resize: mapped,
        mapped_after_resize: false,
    }
}

#[must_use]
fn screen_edge_visible(revealed: bool, overlay_visible: bool) -> bool {
    revealed && overlay_visible
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct wake-gate predicate; a flags enum would be less readable at the single call site"
)]
#[derive(Debug, Clone, Copy)]
struct PollGate {
    failed: bool,
    visible: bool,
    wants_render: bool,
    client_running: bool,
    target_available: bool,
}

/// Max time the host may sleep for this overlay. `next_wake` is the tick-based
/// wake (already converted to a remaining `Duration`); `inter_frame_remaining`
/// is `Some` while the 8 ms frame floor has time left.
///
/// The wake decision must agree with `overlay_needs_render`: while invisible
/// (or otherwise non-rendering) a latched `wants_render` must not request an
/// immediate wake, or the host busy-spins on a frame that never renders.
#[must_use]
fn overlay_poll_timeout(
    gate: PollGate,
    next_wake: Option<Duration>,
    inter_frame_remaining: Option<Duration>,
) -> Option<Duration> {
    if gate.failed || !gate.wants_render || !gate.visible || !gate.client_running {
        return next_wake;
    }
    match inter_frame_remaining {
        Some(d) => Some(next_wake.map_or(d, |t| d.min(t))),
        None if gate.target_available => Some(Duration::ZERO),
        None => next_wake,
    }
}

/// A system overlay hosted inside another process (e.g. bmc-wasm-host). It owns
/// its Wayland connection and export buffers but borrows the host's renderer
/// and GPU stack for the actual frame, which the host orchestrates.
#[expect(missing_debug_implementations)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct overlay-state flag; a flags enum would obscure the field accesses"
)]
pub struct HostedOverlay {
    overlay: Box<dyn SystemOverlay>,
    client: LayerSurfaceClient,
    target: OverlayRenderTarget,
    config_size: (u32, u32),
    size: (u32, u32),
    last_render: Option<Instant>,
    next_wake: Option<Instant>,
    wants_render: bool,
    /// Whether the overlay currently wants to be on-screen (from its `tick`).
    visible: bool,
    /// Whether the surface is currently mapped (has a live buffer attached).
    mapped: bool,
    /// Set after a non-fatal render/export/attach error. A failed overlay is
    /// dropped from the host's list (terminal) — it must NOT keep `wants_render`
    /// latched, or it would busy-retry-and-log every pass.
    failed: bool,
    /// `Some(edge)` for a screen-edge overlay; its map/unmap is driven by
    /// reveal/hide events, not directly by `tick`'s `visible`.
    screen_edge: Option<crate::overlay::ScreenEdge>,
    /// True between a `revealed` event and the next hide+re-arm.
    revealed: bool,
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
        let screen_edge = overlay.screen_edge();
        if let Some(edge) = screen_edge {
            client.create_screen_edge(edge)?;
        }
        Ok(Self {
            overlay,
            client,
            target,
            config_size,
            size,
            last_render: None,
            next_wake: None,
            wants_render,
            visible: false,
            mapped: false,
            failed: false,
            screen_edge,
            revealed: false,
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
        if self.screen_edge.is_some() {
            if self.client.take_reveal() {
                self.revealed = true;
                self.overlay.on_reveal();
                self.wants_render = true;
            }
            if self.client.take_hidden() {
                self.revealed = false;
            }
        }
        for released in self.client.drain_released_buffers() {
            self.target.mark_released_buffer(&released);
        }
        if let Some(configured_size) = self.client.take_configured_size_change() {
            let size = resolved_configured_size(self.config_size, configured_size);
            if self.size != size {
                let transition = resize_transition(self.mapped);
                if transition.unmap_before_resize {
                    self.client.attach_null_buffer()?;
                    self.client.roundtrip_after_resize_unmap(configured_size)?;
                    self.target.free_for_hide(egl, &mut self.client)?;
                    self.last_render = None;
                }
                self.mapped = transition.mapped_after_resize;
                self.target.resize(egl, &mut self.client, size.0, size.1)?;
                self.size = size;
            }
            self.wants_render = true;
        }
        if self.client.take_needs_render() {
            self.wants_render = true;
        }
        Ok(())
    }

    /// Run background work; updates visibility, render-want and next-wake.
    pub fn tick(&mut self, now: Instant) {
        let outcome = self.overlay.tick(now);
        self.visible = match self.screen_edge {
            Some(_) => screen_edge_visible(self.revealed, outcome.visible),
            None => outcome.visible,
        };
        if self.visible {
            self.wants_render |= outcome.wants_render;
        }
        self.next_wake = outcome.next_wake;
    }

    /// Whether a frame should be rendered+submitted this pass. A first show
    /// (visible but not yet mapped) always renders, even without `wants_render`.
    #[must_use]
    pub fn needs_render(&self, now: Instant) -> bool {
        let inter_frame_ok = self
            .last_render
            .is_none_or(|t| now.duration_since(t) >= MIN_INTER_FRAME);
        overlay_needs_render(RenderGate {
            failed: self.failed,
            visible: self.visible,
            mapped: self.mapped,
            wants_render: self.wants_render,
            inter_frame_ok,
            client_running: self.client.running(),
            target_available: self.target.available(),
        })
    }

    /// Whether the overlay is mapped but no longer wants to be — the host must
    /// unmap and free its buffers this pass.
    #[must_use]
    pub fn needs_hide(&self) -> bool {
        overlay_needs_hide(self.mapped, self.visible)
    }

    /// Unmap the surface and free export buffers. Called by the host when
    /// `needs_hide` is true.
    pub fn hide(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        // Ordering is load-bearing: flush the NULL attach before destroying
        // exported buffers so the compositor observes the unmap first.
        self.client.attach_null_buffer()?;
        self.client.roundtrip_after_hide_unmap()?;
        self.target.free_for_hide(egl, &mut self.client)?;
        self.mapped = false;
        self.wants_render = false;
        // Clear the frame-floor timestamp so a later re-show renders promptly
        // and the hosted/standalone loops stay symmetric.
        self.last_render = None;
        if self.screen_edge.is_some() {
            self.revealed = false;
            self.client.rearm_screen_edge()?;
        }
        Ok(())
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
        let inter_frame_remaining = self
            .last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(now.duration_since(t)))
            .filter(|d| !d.is_zero());
        overlay_poll_timeout(
            PollGate {
                failed: self.failed,
                visible: self.visible,
                wants_render: self.wants_render,
                client_running: self.client.running(),
                target_available: self.target.available(),
            },
            tick,
            inter_frame_remaining,
        )
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

    /// Restore layer-shell pending state after a NULL-buffer unmap before
    /// rendering into a new frame.
    pub fn prepare_for_render(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        if !self.client.ensure_ready_for_buffer_attach()? {
            return Ok(());
        }

        let size = resolved_configured_size(self.config_size, self.client.size());
        if self.size != size {
            self.target.resize(egl, &mut self.client, size.0, size.1)?;
            self.size = size;
        }
        Ok(())
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

    /// Mark a render as completed at `now`; the surface is now mapped.
    pub fn mark_rendered(&mut self, now: Instant) {
        self.last_render = Some(now);
        self.wants_render = false;
        self.mapped = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runnable_gate(visible: bool, mapped: bool, wants_render: bool) -> RenderGate {
        RenderGate {
            failed: false,
            visible,
            mapped,
            wants_render,
            inter_frame_ok: true,
            client_running: true,
            target_available: true,
        }
    }

    #[test]
    fn first_show_renders_without_dirty_flag() {
        assert!(overlay_needs_render(runnable_gate(true, false, false)));
    }

    #[test]
    fn hidden_ignores_latched_render_request() {
        assert!(!overlay_needs_render(runnable_gate(false, false, true)));
    }

    #[test]
    fn mapped_but_invisible_needs_hide() {
        assert!(overlay_needs_hide(true, false));
    }

    #[test]
    fn mapped_resize_unmaps_before_destroying_buffers() {
        assert_eq!(
            resize_transition(true),
            ResizeTransition {
                unmap_before_resize: true,
                mapped_after_resize: false,
            }
        );
        assert_eq!(
            resize_transition(false),
            ResizeTransition {
                unmap_before_resize: false,
                mapped_after_resize: false,
            }
        );
    }

    #[test]
    fn screen_edge_overlay_visible_only_while_revealed() {
        assert!(
            !screen_edge_visible(false, true),
            "armed-but-hidden stays unmapped"
        );
        assert!(screen_edge_visible(true, true), "revealed and wanted maps");
        assert!(
            !screen_edge_visible(true, false),
            "dismissed while revealed unmaps"
        );
    }

    #[test]
    fn throttled_first_show_waits_for_frame_floor() {
        let mut gate = runnable_gate(true, false, false);
        gate.inter_frame_ok = false;
        assert!(!overlay_needs_render(gate));
    }

    fn runnable_poll_gate(visible: bool, wants_render: bool) -> PollGate {
        PollGate {
            failed: false,
            visible,
            wants_render,
            client_running: true,
            target_available: true,
        }
    }

    #[test]
    fn invisible_overlay_with_latched_render_does_not_busy_spin() {
        let gate = runnable_poll_gate(false, true);
        assert_eq!(
            overlay_poll_timeout(gate, Some(Duration::from_secs(2)), None),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn renderable_overlay_polls_immediately() {
        let gate = runnable_poll_gate(true, true);
        assert_eq!(overlay_poll_timeout(gate, None, None), Some(Duration::ZERO));
    }

    #[test]
    fn renderable_overlay_waits_for_inter_frame_floor() {
        let gate = runnable_poll_gate(true, true);
        assert_eq!(
            overlay_poll_timeout(gate, None, Some(Duration::from_millis(5))),
            Some(Duration::from_millis(5))
        );
    }
}
