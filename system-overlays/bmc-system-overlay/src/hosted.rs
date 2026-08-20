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

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use bmc_widget::egl::{DmaBufInfo, EglContext};

use crate::gpu::OverlayRenderTarget;
use crate::overlay::{
    AlarmEvent, FenceState, HideFenceAction, HideFenceGate, MIN_INTER_FRAME, PollGate, RenderGate,
    SystemOverlay, deliver_upgrade_snapshot_and_tick, hide_fence_action, hide_fence_after_tick,
    overlay_needs_hide, overlay_needs_render, overlay_poll_timeout, resize_transition,
    resolved_configured_size, screen_edge_visible,
};
use crate::surface::LayerSurfaceClient;

/// Bound on how long `HostedOverlay::hide` waits for its presentation fence
/// before unmapping anyway. Covers a repaint under cross-process GPU-lock
/// contention (device traces showed ~112 ms worst-case frame gaps) while
/// bounding how long a wedged compositor can defer the unmap.
const UNMAP_FENCE_TIMEOUT: Duration = Duration::from_millis(150);

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
    /// Deadline for the in-flight presentation fence armed by `hide`; `None`
    /// when no fence is pending.
    hide_fence: Option<Instant>,
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
        let mut client = LayerSurfaceClient::connect(
            &config,
            crate::surface::ProtocolOptIns::from_overlay(overlay.as_ref()),
        )?;
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
            hide_fence: None,
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
        self.dispatch_with_target_resize(|target, client, free_before_resize, width, height| {
            if free_before_resize {
                target.free_for_hide(egl, client)?;
            }
            target.resize(egl, client, width, height)
        })
    }

    /// Drain Wayland events before delegating a compositor-driven target resize.
    pub fn dispatch_with_target_resize(
        &mut self,
        resize_target: impl FnOnce(
            &mut OverlayRenderTarget,
            &mut LayerSurfaceClient,
            bool,
            u32,
            u32,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
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
        if self.overlay.uses_settings() {
            // Capabilities go first: the wire sends them first on bind, and
            // the trait promises them before any other settings event.
            if let Some(caps) = self.client.take_capabilities() {
                self.overlay.on_capabilities(caps);
            }
            if let Some(v) = self.client.take_brightness() {
                self.overlay.on_brightness(v);
            }
            if let Some(ap) = self.client.take_wifi_ap() {
                self.overlay.on_wifi_ap(ap.as_deref());
            }
            if let Some(v) = self.client.take_volume() {
                self.overlay.on_volume(v);
            }
            if let Some((active, until)) = self.client.take_night_mode() {
                self.overlay.on_night_mode(active, until.as_deref());
            }
            if let Some(reason) = self.client.take_restart_declined() {
                self.overlay.on_restart_declined(&reason);
            }
            if let Some(active) = self.client.take_preempted() {
                self.overlay.on_preempted(active);
            }
        }
        if self.overlay.uses_alarm() {
            // One latest-wins slot, so a stop-then-ring within a single dispatch
            // round applies the ring (not the trailing stop) and vice versa.
            match self.client.take_alarm_event() {
                Some(AlarmEvent::Ring {
                    time,
                    period,
                    label,
                    snooze_allowed,
                }) => self
                    .overlay
                    .on_alarm_ring(&time, &period, &label, snooze_allowed),
                Some(AlarmEvent::Stop) => self.overlay.on_alarm_stop(),
                None => {}
            }
        }
        if self.overlay.uses_device_info() {
            if let Some((state, boot_flow_delivered)) = self.client.take_device_state() {
                self.overlay.on_device_state(state, boot_flow_delivered);
            }
            if let Some((step, ssid)) = self.client.take_setup_progress() {
                self.overlay.on_setup_progress(step, &ssid);
            }
            if let Some(ap) = self.client.take_access_point() {
                self.overlay.on_access_point(ap.as_ref());
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
                    // This NULL-attaches directly, bypassing `hide`'s fence
                    // machinery, so a fence armed just before the resize
                    // must not dangle across the remap.
                    self.hide_fence = None;
                    self.client.cancel_presentation_fence();
                    self.client.attach_null_buffer()?;
                    self.client.roundtrip_after_resize_unmap(configured_size)?;
                    self.overlay.mark_content_dirty();
                    self.last_render = None;
                }
                self.mapped = transition.mapped_after_resize;
                resize_target(
                    &mut self.target,
                    &mut self.client,
                    transition.unmap_before_resize,
                    size.0,
                    size.1,
                )?;
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
        let outcome = deliver_upgrade_snapshot_and_tick(
            &mut *self.overlay,
            self.client.take_upgrade_snapshot(),
            now,
        );
        self.visible = match self.screen_edge {
            Some(_) => screen_edge_visible(self.revealed, outcome.visible),
            None => outcome.visible,
        };
        if self.visible {
            self.wants_render |= outcome.wants_render;
        }
        // During a normal dismiss this never fires: the screen edge only
        // re-arms after the unmap, so no re-reveal can arrive mid-fence.
        // Kept so the state machine stays self-consistent if a visibility
        // source ever behaves differently.
        if self.visible && self.hide_fence.is_some() {
            // Dropping the local deadline must abandon the client fence too,
            // or its late callback would satisfy an unrelated future hide.
            self.client.cancel_presentation_fence();
        }
        self.hide_fence = hide_fence_after_tick(self.visible, self.hide_fence);
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

    /// Whether this hidden overlay wants its panel cache repainted: unmapped,
    /// not on its way to being shown, healthy, and holding a content change.
    /// Mapped overlays refresh through the normal render path instead.
    #[must_use]
    pub fn needs_cache_refresh(&self) -> bool {
        !self.failed
            && !self.visible
            && !self.mapped
            && self.client.running()
            && self.overlay.content_dirty()
            && self.overlay.uses_panel_cache()
    }

    /// Unmap the surface and free export buffers. Called by the host every
    /// pass while `needs_hide` is true. Non-blocking: the first call arms a
    /// presentation fence and returns without unmapping; later calls unmap
    /// once the fence resolves (compositor callback or deadline) or the
    /// client is gone, so the compositor's last repaint is confirmed shown
    /// before the NULL attach.
    pub fn hide(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        self.hide_with_target_cleanup(|target, client| target.free_for_hide(egl, client))
    }

    /// Unmap the surface before delegating render-target cleanup.
    ///
    /// The callback lets a host synchronize GPU cleanup without holding that
    /// synchronization across the compositor roundtrip.
    pub fn hide_with_target_cleanup(
        &mut self,
        free_target: impl FnOnce(
            &mut OverlayRenderTarget,
            &mut LayerSurfaceClient,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let frame_presented = self.client.take_frame_presented();
        let now = Instant::now();
        let action = hide_fence_action(HideFenceGate {
            fence: match self.hide_fence {
                None => FenceState::Unarmed,
                Some(deadline) => FenceState::Armed {
                    deadline_passed: now >= deadline,
                },
            },
            frame_presented,
            client_running: self.client.running(),
        });

        match action {
            HideFenceAction::Arm => {
                self.client.request_presentation_fence()?;
                self.hide_fence = Some(now + UNMAP_FENCE_TIMEOUT);
                Ok(())
            }
            HideFenceAction::Wait => Ok(()),
            HideFenceAction::Unmap { timed_out } => {
                if timed_out {
                    tracing::warn!(
                        "hide-fence deadline reached before a presented frame; unmapping anyway"
                    );
                }
                // Ordering is load-bearing: flush the NULL attach before destroying
                // exported buffers so the compositor observes the unmap first.
                self.client.attach_null_buffer()?;
                self.client.roundtrip_after_hide_unmap()?;
                free_target(&mut self.target, &mut self.client)?;
                self.mapped = false;
                self.wants_render = false;
                // Clear the frame-floor timestamp so a later re-show renders promptly
                // and the hosted/standalone loops stay symmetric.
                self.last_render = None;
                self.hide_fence = None;
                // No-op after a normal fence resolve; drops the still-pending
                // client fence when the deadline forced this unmap.
                self.client.cancel_presentation_fence();
                if self.screen_edge.is_some() {
                    self.revealed = false;
                    self.client.rearm_screen_edge()?;
                }
                Ok(())
            }
        }
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
        let gated = overlay_poll_timeout(
            PollGate {
                failed: self.failed,
                visible: self.visible,
                wants_render: self.wants_render,
                client_running: self.client.running(),
                target_available: self.target.available(),
            },
            tick,
            inter_frame_remaining,
        );
        // A pending hide fence must still wake the loop at its deadline even
        // if nothing else would — otherwise a silent compositor strands the
        // unmap on the next unrelated event.
        let fence_remaining = self
            .hide_fence
            .map(|deadline| deadline.saturating_duration_since(now));
        match (gated, fence_remaining) {
            (Some(g), Some(f)) => Some(g.min(f)),
            (None, Some(f)) => Some(f),
            (Some(g), None) => Some(g),
            (None, None) => None,
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

    /// Restore layer-shell pending state after a NULL-buffer unmap before
    /// rendering into a new frame.
    pub fn prepare_for_render(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        self.prepare_for_render_with_target_resize(|target, client, width, height| {
            target.resize(egl, client, width, height)
        })
    }

    /// Restore pending surface state before delegating a required target resize.
    pub fn prepare_for_render_with_target_resize(
        &mut self,
        resize_target: impl FnOnce(
            &mut OverlayRenderTarget,
            &mut LayerSurfaceClient,
            u32,
            u32,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if !self.client.ensure_ready_for_buffer_attach()? {
            return Ok(());
        }

        let size = resolved_configured_size(self.config_size, self.client.size());
        if self.size != size {
            resize_target(&mut self.target, &mut self.client, size.0, size.1)?;
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

    /// Mark a render as completed at `now`; the surface is now mapped. Also
    /// notifies the overlay, so time-anchored animations can anchor at the
    /// hand-off to the compositor.
    pub fn mark_rendered(&mut self, now: Instant) {
        self.overlay.on_frame_submitted(now);
        self.last_render = Some(now);
        self.wants_render = false;
        self.mapped = true;
    }

    /// Forward the overlay's accumulated control requests to the compositor.
    /// Called once per loop iteration after the render step so render-produced
    /// requests (a brightness slider drag reads `TreeResult.drags` in `render`)
    /// go out the same pass they were created. No-op unless the overlay opted
    /// into `deck_settings_v1`.
    pub fn forward_settings_requests(&mut self) {
        if !self.overlay.uses_settings() {
            return;
        }
        for req in self.overlay.drain_settings_requests() {
            if let Err(e) = self.client.send_settings_request(req) {
                tracing::warn!("settings request failed: {e}");
            }
        }
    }

    /// Forward the overlay's accumulated alarm requests (dismiss/snooze) to the
    /// compositor. Runs after render, same pass as they were produced. No-op
    /// unless the overlay opted into `deck_alarm_v1`.
    pub fn forward_alarm_requests(&mut self) {
        if !self.overlay.uses_alarm() {
            return;
        }
        for req in self.overlay.drain_alarm_requests() {
            if let Err(e) = self.client.send_alarm_request(req) {
                tracing::warn!("alarm request failed: {e}");
            }
        }
    }
}
