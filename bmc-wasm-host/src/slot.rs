// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{
    NextAlarm as RuntimeNextAlarm, RenderStatus, RuntimeConfig, SystemSnapshot, WasmWidgetRuntime,
};
use bmc_widget::surface::{DeckWidgetSurfaceClient, WidgetEvent, WidgetSurface};
use bmc_widget_protocol::{NextAlarm as WireNextAlarm, SettingUpdate};
use serde_json::{Map, Value};

use crate::host::SharedHost;
use crate::lifecycle::{
    LifecycleState, LifecycleStateMachine, SlotApplyCtx, frame_callback_enabled, should_render,
};
use crate::render_target::{EglRenderTarget, RenderTarget, RenderTargetFactory};

/// Per-slot inter-frame floor — caps a misbehaving widget that returns
/// `wants_next_frame() == true` every iteration at ~120 fps.
pub const MIN_INTER_FRAME: std::time::Duration = std::time::Duration::from_millis(8);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RenderGate {
    #[default]
    NotRenderable,
    Blocked,
    Renderable,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SlotRenderInputs {
    pub gate: RenderGate,
    pub surface_needs_render: bool,
    pub runtime_frame_due: bool,
    pub min_inter_frame_remaining: Option<Duration>,
}

#[must_use]
pub fn slot_needs_render_from_inputs(inputs: SlotRenderInputs) -> bool {
    if inputs.gate != RenderGate::Renderable {
        return false;
    }
    if !inputs.surface_needs_render && !inputs.runtime_frame_due {
        return false;
    }
    inputs
        .min_inter_frame_remaining
        .is_none_or(|remaining| remaining.is_zero())
}

#[must_use]
pub fn duration_to_timeout_millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

#[must_use]
pub fn refresh_runtime_frame_due_at(
    existing_due_at: Option<Instant>,
    wants_next_frame: bool,
    next_frame_delay: Option<u32>,
    anchor: Instant,
) -> Option<Instant> {
    if !wants_next_frame {
        return None;
    }

    let candidate = match next_frame_delay {
        None => anchor,
        Some(delay_ms) => anchor + Duration::from_millis(delay_ms.into()),
    };
    Some(existing_due_at.map_or(candidate, |existing| existing.min(candidate)))
}

trait RendererAssetEvictor {
    fn evict_renderer_prefix(&mut self, prefix: &str) -> usize;
}

impl<T: Renderer + ?Sized> RendererAssetEvictor for T {
    fn evict_renderer_prefix(&mut self, prefix: &str) -> usize {
        Renderer::evict_prefix(self, prefix)
    }
}

fn evict_renderer_assets(renderer: &mut impl RendererAssetEvictor, asset_namespace: &str) -> usize {
    renderer.evict_renderer_prefix(asset_namespace)
}

#[derive(Debug)]
pub enum ControlSocketStatus {
    WouldBlock,
    PeerClosed,
    UnsolicitedByte(u8),
    Error(std::io::Error),
}

#[must_use]
pub fn classify_control_socket_read(
    result: std::io::Result<usize>,
    byte: u8,
) -> ControlSocketStatus {
    match result {
        Ok(0) => ControlSocketStatus::PeerClosed,
        Ok(_) => ControlSocketStatus::UnsolicitedByte(byte),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => ControlSocketStatus::WouldBlock,
        Err(e) => ControlSocketStatus::Error(e),
    }
}

#[expect(missing_debug_implementations)]
pub struct WidgetSlot {
    pub surface: DeckWidgetSurfaceClient,
    pub runtime: WasmWidgetRuntime,
    pub lifecycle: LifecycleStateMachine,
    pub render_target: Option<RenderTarget>,
    pub retired_render_targets: Vec<RenderTarget>,
    pub factory: Rc<dyn RenderTargetFactory>,
    pub last_render_at: Option<Instant>,
    pub monotonic_origin: Instant,
    pub frame_count: u64,
    pub initial_params: Map<String, Value>,
    /// Deck-wide system snapshot accumulated from `SettingUpdate` wayland
    /// events. Seeded from the compositor's initial configure batch,
    /// then mutated per-field by each subsequent setting delivery.
    ///
    /// After a drain that touched any setting, the latest snapshot is pushed
    /// to the runtime via `deliver_system_update`.
    pub pending_system: SystemSnapshot,
    pub peer_pid: libc::pid_t,
    pub wasm_basename: String,
    pub control_socket: UnixStream,
    pub next_frame_due_at: Option<Instant>,
}

impl WidgetSlot {
    pub fn from_handshake(
        wasm_path: &Path,
        wayland_fd: std::os::fd::OwnedFd,
        control_socket: UnixStream,
        peer_pid: libc::pid_t,
        factory: Rc<dyn RenderTargetFactory>,
    ) -> Result<Self> {
        tracing::info!(
            peer_pid,
            wasm = %wasm_path.display(),
            "connecting widget Wayland fd"
        );
        let (surface, initial) = DeckWidgetSurfaceClient::connect_with_fd(wayland_fd)
            .context("DeckWidgetSurfaceClient::connect_with_fd")?;
        tracing::info!(
            peer_pid,
            wasm = %wasm_path.display(),
            w = initial.width,
            h = initial.height,
            params = initial.params.len(),
            settings = initial.settings.len(),
            "widget Wayland configure received"
        );
        let wasm_bytes =
            std::fs::read(wasm_path).with_context(|| format!("read {}", wasm_path.display()))?;
        tracing::info!(
            peer_pid,
            wasm = %wasm_path.display(),
            bytes = wasm_bytes.len(),
            "wasm module read"
        );
        // Seed the system snapshot from any per-field setting events
        // the compositor  included in the initial configure batch.
        //
        // The runtime sees the same bytes via `RuntimeConfig::system`
        // before `init` runs, so the widget's first frame observes
        // operator values.
        let mut pending_system = SystemSnapshot::default();
        for setting in &initial.settings {
            apply_setting_update(&mut pending_system, setting);
        }
        let viewport_shape = bmc_wasm_protocol::ViewportShape::from(initial.viewport_shape);
        let display = bmc_wasm_runtime::RuntimeDisplayInfo::from(initial.display);
        let mut runtime = WasmWidgetRuntime::new(
            &wasm_bytes,
            initial.width,
            initial.height,
            viewport_shape,
            display,
            RuntimeConfig {
                params: bmc_wasm_runtime::parse_params_json(&initial.params).unwrap_or_else(
                    |err| {
                        tracing::warn!(?err, "invalid initial params — empty map");
                        BTreeMap::default()
                    },
                ),
                system: pending_system.clone(),
                ..RuntimeConfig::default()
            },
        )?;
        runtime.set_time(chrono::Local::now().fixed_offset(), 0);
        tracing::info!(
            peer_pid,
            wasm = %wasm_path.display(),
            w = initial.width,
            h = initial.height,
            "wasm runtime initialized; waiting for lifecycle event"
        );

        Ok(Self {
            surface,
            runtime,
            lifecycle: LifecycleStateMachine::new(),
            render_target: None,
            retired_render_targets: Vec::new(),
            factory,
            last_render_at: None,
            monotonic_origin: Instant::now(),
            frame_count: 0,
            initial_params: initial.params,
            pending_system,
            peer_pid,
            wasm_basename: wasm_path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            control_socket,
            next_frame_due_at: None,
        })
    }

    pub fn dispatch_wayland_events(&mut self) -> Result<()> {
        self.surface.poll_dispatch(0)?;
        // The compositor fans a single localization save into one per-field setting
        // event per setting; per-event delivery would call `on_system_update`
        // (and re-render) N times for one operator action.
        //
        // Collect Setting events into `pending_system` during the drain
        // and push a single `deliver_system_update` after the loop.
        // Similarly, every `ParamUpdate` carries the full params set per the
        // compositor↔widget contract; multiple updates in one drain collapse
        // to the latest event — never merged.
        let mut system_dirty = false;
        let mut latest_params: Option<serde_json::Map<String, Value>> = None;
        for event in <DeckWidgetSurfaceClient as WidgetSurface>::drain_events(&mut self.surface) {
            match event {
                WidgetEvent::Setting(setting) => {
                    apply_setting_update(&mut self.pending_system, &setting);
                    system_dirty = true;
                }
                WidgetEvent::ParamUpdate(params) => latest_params = Some(params),
                event @ (WidgetEvent::Lifecycle(_)
                | WidgetEvent::Shutdown
                | WidgetEvent::TouchDown { .. }
                | WidgetEvent::TouchMotion { .. }
                | WidgetEvent::TouchUp { .. }
                | WidgetEvent::TouchCancel) => self.on_wayland_event(&event),
            }
        }
        if system_dirty {
            tracing::info!(
                peer_pid = self.peer_pid,
                wasm = %self.wasm_basename,
                "settings drained from wayland — delivering system snapshot to runtime"
            );
            self.runtime
                .deliver_system_update(self.pending_system.clone());
            self.surface.mark_needs_render();
        }
        if let Some(params) = latest_params
            && let Ok(table) = bmc_wasm_runtime::parse_params_json(&params)
        {
            tracing::info!(
                peer_pid = self.peer_pid,
                wasm = %self.wasm_basename,
                params = table.len(),
                "param update received"
            );
            self.runtime.deliver_params_update(table);
            self.surface.mark_needs_render();
        }
        let released_buffers = self.surface.drain_released_buffers();
        if let Some(egl_target) = self
            .render_target
            .as_mut()
            .and_then(RenderTarget::as_egl_mut)
        {
            for released in &released_buffers {
                egl_target.mark_released_buffer(released);
            }
        }
        for target in &mut self.retired_render_targets {
            let Some(egl_target) = target.as_egl_mut() else {
                continue;
            };
            for released in &released_buffers {
                egl_target.mark_released_buffer(released);
            }
        }
        Ok(())
    }

    pub fn reclaim_retired_render_targets(&mut self, shared: &SharedHost) {
        let mut pending = Vec::new();
        for mut target in self.retired_render_targets.drain(..) {
            match self
                .factory
                .destroy_released_slots(&mut target, &shared.egl, &mut self.surface)
            {
                crate::render_target::RenderTargetCleanup::Complete => {}
                crate::render_target::RenderTargetCleanup::PendingRelease => {
                    pending.push(target);
                }
            }
        }
        self.retired_render_targets = pending;
    }

    pub fn dispatch_control_socket(&mut self) -> Result<()> {
        use std::io::Read;
        let mut buf = [0_u8; 1];
        let result = (&self.control_socket).read(&mut buf);
        match classify_control_socket_read(result, buf[0]) {
            ControlSocketStatus::WouldBlock => Ok(()),
            ControlSocketStatus::PeerClosed => Err(anyhow::anyhow!("control socket EOF")),
            ControlSocketStatus::UnsolicitedByte(b) => Err(anyhow::anyhow!(
                "unsolicited byte on control socket (protocol violation): {b:#04x}"
            )),
            ControlSocketStatus::Error(e) => Err(anyhow::Error::from(e)),
        }
    }

    #[must_use]
    pub fn is_renderable(&self) -> bool {
        should_render(self.lifecycle.current())
    }

    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.lifecycle.blocked()
    }

    #[must_use]
    pub fn render_buffer_available(&self) -> bool {
        self.render_target
            .as_ref()
            .and_then(RenderTarget::as_egl)
            .is_none_or(EglRenderTarget::current_slot_available)
    }

    #[must_use]
    pub fn surface_needs_render(&self) -> bool {
        self.surface.needs_render()
    }

    #[must_use]
    pub fn runtime_frame_due(&self, now: Instant) -> bool {
        self.next_frame_due_at.is_some_and(|due| now >= due)
    }

    #[must_use]
    pub fn runtime_frame_due_in(&self, now: Instant) -> Option<Duration> {
        self.next_frame_due_at
            .map(|due| due.saturating_duration_since(now))
    }

    #[must_use]
    pub fn frame_callback_enabled(&self) -> bool {
        frame_callback_enabled(self.lifecycle.current())
    }

    /// Spec § 7 / BDK-437 § States table: Entering renders **once** in response to a
    /// dirty surface (lifecycle / params / touch), no animation, no frame callbacks.
    /// Visible widgets run the full animation loop; Leaving widgets keep their render
    /// target for scene transitions but no longer drive runtime animation frames. Gate
    /// animation-driven renders on `frame_callback_enabled` so inactive slots whose
    /// runtime returns `wants_next_frame() == true` do NOT spin a continuous render loop.
    #[must_use]
    pub fn needs_render(&self, now: Instant) -> bool {
        let gate = if !self.is_renderable() {
            RenderGate::NotRenderable
        } else if self.is_blocked() || !self.render_buffer_available() {
            RenderGate::Blocked
        } else {
            RenderGate::Renderable
        };
        let runtime_frame_due = self.frame_callback_enabled() && self.runtime_frame_due(now);
        slot_needs_render_from_inputs(SlotRenderInputs {
            gate,
            surface_needs_render: self.surface_needs_render(),
            runtime_frame_due,
            min_inter_frame_remaining: self.min_inter_frame_remaining(now),
        })
    }

    #[must_use]
    pub fn has_min_inter_frame_elapsed(&self, now: Instant) -> bool {
        match self.last_render_at {
            None => true,
            Some(t) => now.duration_since(t) >= MIN_INTER_FRAME,
        }
    }

    #[must_use]
    pub fn min_inter_frame_remaining(&self, now: Instant) -> Option<std::time::Duration> {
        let t = self.last_render_at?;
        let elapsed = now.duration_since(t);
        MIN_INTER_FRAME.checked_sub(elapsed)
    }

    #[must_use]
    pub fn retry_in(&self, now: Instant) -> Option<std::time::Duration> {
        let t = self.lifecycle.retry_at()?;
        Some(t.saturating_duration_since(now))
    }

    #[must_use]
    pub fn poll_inputs(&self, now: Instant) -> crate::main_loop::SlotPollInputs {
        crate::main_loop::SlotPollInputs {
            retry_in: self.retry_in(now),
            is_renderable: self.is_renderable(),
            is_blocked: self.is_blocked() || !self.render_buffer_available(),
            frame_callback_enabled: self.frame_callback_enabled(),
            animation_wants_immediate: self.runtime_frame_due(now),
            surface_needs_render: self.surface_needs_render(),
            min_inter_frame_remaining: self.min_inter_frame_remaining(now),
            next_frame_delay: self
                .runtime_frame_due_in(now)
                .map(duration_to_timeout_millis),
            has_pending_io: self.runtime.has_pending_io(),
        }
    }

    /// Schedule the next runtime frame relative to `anchor` — the instant
    /// the just-rendered frame *started*, not the instant it finished.
    ///
    /// Anchoring to the start makes the widget's requested delay
    /// the true frame period; render time is absorbed into
    /// the interval instead of stacking on top.
    pub fn schedule_next_runtime_frame(&mut self, anchor: Instant) {
        self.next_frame_due_at = if self.runtime.wants_next_frame() {
            Some(match self.runtime.next_frame_delay() {
                None => anchor,
                Some(delay_ms) => anchor + Duration::from_millis(delay_ms.into()),
            })
        } else {
            None
        };
    }

    pub fn refresh_next_runtime_frame_after_delivery(&mut self, anchor: Instant) {
        self.next_frame_due_at = refresh_runtime_frame_due_at(
            self.next_frame_due_at,
            self.runtime.wants_next_frame(),
            self.runtime.next_frame_delay(),
            anchor,
        );
    }

    pub fn tick_delta(&mut self, now: Instant) -> u32 {
        let delta = self.last_render_at.map_or(0, |t| {
            u32::try_from(now.duration_since(t).as_millis()).unwrap_or(u32::MAX)
        });
        self.last_render_at = Some(now);
        delta
    }

    #[must_use]
    pub fn monotonic_ms(&self, now: Instant) -> u64 {
        u64::try_from(now.duration_since(self.monotonic_origin).as_millis()).unwrap_or(u64::MAX)
    }

    pub fn advance_runtime_time(
        &mut self,
        system_time: chrono::DateTime<chrono::FixedOffset>,
        now: Instant,
    ) {
        self.runtime.set_time(system_time, self.monotonic_ms(now));
    }

    pub fn apply_lifecycle(&mut self, now: Instant, shared: &SharedHost) {
        let previous = self.lifecycle.current();
        let w = self.surface.width();
        let h = self.surface.height();
        let mut ctx = SlotApplyCtx {
            factory: &self.factory,
            egl: &shared.egl,
            surface: &mut self.surface,
            render_target: &mut self.render_target,
            retired_render_targets: &mut self.retired_render_targets,
            width: w,
            height: h,
        };
        self.lifecycle.apply(&mut ctx, now);
        let current = self.lifecycle.current();
        if previous != current {
            tracing::info!(
                peer_pid = self.peer_pid,
                wasm = %self.wasm_basename,
                ?previous,
                ?current,
                blocked = self.lifecycle.blocked(),
                render_target = self.render_target.is_some(),
                "slot lifecycle applied"
            );
        }
    }

    pub fn render(
        &mut self,
        ptr: NonNull<dyn Renderer>,
        delta_ms: u32,
        shared: &mut SharedHost,
    ) -> Result<RenderStatus> {
        let _ = self.surface.take_render_requested();
        let frame_start_instant = Instant::now();
        let frame_start = HostRenderProfiling::start_phase();
        let (dmabuf, slot_idx, status, target_width, target_height) = {
            let phase_start = HostRenderProfiling::start_phase();
            let target = self.render_target.as_mut().expect(
                "BUG: render() called on a slot without a render target — \
                 needs_render() should have gated this off when lifecycle ∉ {Prepared, Entering, Visible, Leaving}",
            );
            let target_width = target.width;
            let target_height = target.height;
            let egl_target = target.as_egl_mut().expect(
                "BUG: EglRenderTargetFactory allocated all WidgetSlot render targets in Task 8",
            );

            let gpu_render_lock = shared.acquire_gpu_render_lock("host_widget_render")?;
            egl_target.buffers.ensure_current(&shared.egl)?;
            let _staging_fbo = shared
                .scratch
                .begin_frame(&shared.egl, target_width, target_height);
            normalize_gl_state(&shared.egl, target_width, target_height);

            unsafe { ptr.as_ptr().as_mut() }
                .expect(
                    "BUG: renderer pointer was NonNull when stored, \
                     raw-pointer reborrow must produce a non-null reference",
                )
                .begin_frame(target_width, target_height, 1.0);
            HostRenderProfiling::log_phase(&self.wasm_basename, "frame_setup", phase_start);

            let phase_start = HostRenderProfiling::start_phase();
            let status = self.runtime.with_renderer(ptr, |rt| rt.render(delta_ms))?;
            HostRenderProfiling::log_phase(&self.wasm_basename, "runtime_render", phase_start);

            let phase_start = HostRenderProfiling::start_phase();
            unsafe { ptr.as_ptr().as_mut() }
                .expect(
                    "BUG: renderer pointer was NonNull when stored, \
                     raw-pointer reborrow must produce a non-null reference",
                )
                .flush();
            HostRenderProfiling::log_phase(&self.wasm_basename, "femtovg_flush", phase_start);

            let current_export = egl_target.buffers.current_ref().expect(
                "BUG: ensure_current succeeded above, so DoubleBufferState::current_ref \
                 must return Some; an internal invariant of DoubleBufferState was violated",
            );
            let phase_start = HostRenderProfiling::start_phase();
            shared.blit_staging_to(current_export.fbo, target_width, target_height);
            HostRenderProfiling::log_phase(&self.wasm_basename, "staging_blit", phase_start);

            let phase_start = HostRenderProfiling::start_phase();
            // Hold the cross-process GPU render lock until the host GL work is
            // complete. This keeps host and compositor handoffs on the same
            // no-overlapping-in-flight-jobs invariant.
            shared.flush_and_wait_gl();
            HostRenderProfiling::log_phase(&self.wasm_basename, "gl_wait", phase_start);
            drop(gpu_render_lock);

            let phase_start = HostRenderProfiling::start_phase();
            let (dmabuf, slot_idx) = egl_target.buffers.export_and_swap()?;
            HostRenderProfiling::log_phase(&self.wasm_basename, "export_and_swap", phase_start);
            (dmabuf, slot_idx, status, target_width, target_height)
        };

        let wants_immediate = self.frame_callback_enabled()
            && self.runtime.wants_next_frame()
            && self.runtime.next_frame_delay().is_none();
        self.submit_exported_buffer(&dmabuf, slot_idx, wants_immediate)?;
        HostRenderProfiling::log_frame(
            &self.wasm_basename,
            self.frame_count + 1,
            delta_ms,
            frame_start,
            HostRenderFrameContext::new(target_width, target_height, wants_immediate, status),
        );
        self.schedule_next_runtime_frame(frame_start_instant);
        self.frame_count += 1;
        if self.frame_count <= 3 || self.frame_count.is_multiple_of(120) {
            tracing::info!(
                peer_pid = self.peer_pid,
                wasm = %self.wasm_basename,
                frame = self.frame_count,
                delta_ms,
                ?status,
                wants_immediate,
                "widget frame submitted"
            );
        } else {
            tracing::debug!(
                peer_pid = self.peer_pid,
                wasm = %self.wasm_basename,
                frame = self.frame_count,
                delta_ms,
                ?status,
                wants_immediate,
                "widget frame submitted"
            );
        }
        Ok(status)
    }

    fn submit_exported_buffer(
        &mut self,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
        slot_idx: usize,
        wants_immediate: bool,
    ) -> Result<()> {
        let target = self
            .render_target
            .as_mut()
            .expect("BUG: render target must still be present after export_and_swap");
        let egl_target = target.as_egl_mut().expect(
            "BUG: EglRenderTargetFactory allocated all WidgetSlot render targets in Task 8",
        );
        let phase_start = HostRenderProfiling::start_phase();
        let wl_buffer = egl_target
            .wl_buffer_for_slot(&mut self.surface, dmabuf, slot_idx)
            .map_err(anyhow::Error::msg)?;
        self.surface
            .submit_buffer_with_wl_buffer(dmabuf, &wl_buffer, wants_immediate)?;
        HostRenderProfiling::log_phase(&self.wasm_basename, "wayland_attach_commit", phase_start);

        let phase_start = HostRenderProfiling::start_phase();
        self.surface.flush()?;
        HostRenderProfiling::log_phase(&self.wasm_basename, "wayland_flush", phase_start);
        egl_target.mark_presented(slot_idx);
        Ok(())
    }

    pub fn shutdown(mut self, shared: &mut SharedHost, renderer: &mut FemtoVgRenderer) {
        let asset_namespace = self.runtime.asset_namespace();
        drop(self.runtime);

        let evicted_renderer_assets = evict_renderer_assets(renderer, &asset_namespace);
        if evicted_renderer_assets > 0 {
            tracing::debug!(
                peer_pid = self.peer_pid,
                wasm = %self.wasm_basename,
                asset_namespace = %asset_namespace,
                evicted_renderer_assets,
                "widget renderer assets evicted"
            );
        }

        if let Some(target) = self.render_target.take() {
            self.factory.destroy(target, &shared.egl, &mut self.surface);
        }
        for target in self.retired_render_targets.drain(..) {
            self.factory.destroy(target, &shared.egl, &mut self.surface);
        }
        drop(self.surface);
        drop(self.control_socket);
    }

    fn on_wayland_event(&mut self, event: &WidgetEvent) {
        match event {
            WidgetEvent::Lifecycle(state) => {
                let target = map_protocol_lifecycle(*state);
                let previous_target = self.lifecycle.target();
                let effect = self.lifecycle.on_event(target);
                if effect.request_render {
                    self.surface.mark_needs_render();
                }
                tracing::info!(
                    peer_pid = self.peer_pid,
                    wasm = %self.wasm_basename,
                    ?previous_target,
                    ?target,
                    request_render = effect.request_render,
                    "lifecycle event received"
                );
            }
            WidgetEvent::Setting(_) | WidgetEvent::ParamUpdate(_) => {
                // The drain loop in `dispatch_wayland_events` filters
                // Setting and ParamUpdate events into their coalescing
                // paths and never dispatches them here.
                unreachable!("Setting/ParamUpdate handled in dispatch_wayland_events drain");
            }
            WidgetEvent::Shutdown => {
                tracing::info!(
                    peer_pid = self.peer_pid,
                    wasm = %self.wasm_basename,
                    "shutdown event received"
                );
            }
            WidgetEvent::TouchDown { x, y, .. } => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "touch coordinates from Wayland are pixel positions; f64→f32 precision loss is acceptable"
                )]
                self.push_touch(bmc_render::interaction::TouchEvent::Down {
                    x: *x as f32,
                    y: *y as f32,
                });
            }
            WidgetEvent::TouchMotion { x, y, .. } => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "touch coordinates from Wayland are pixel positions; f64→f32 precision loss is acceptable"
                )]
                self.push_touch(bmc_render::interaction::TouchEvent::Move {
                    x: *x as f32,
                    y: *y as f32,
                });
            }
            WidgetEvent::TouchUp { .. } => {
                self.push_touch(bmc_render::interaction::TouchEvent::Up);
            }
            WidgetEvent::TouchCancel => {
                self.push_touch(bmc_render::interaction::TouchEvent::Cancel);
            }
        }
    }

    fn push_touch(&mut self, event: bmc_render::interaction::TouchEvent) {
        self.runtime.push_touch_event(event);
        self.surface.mark_needs_render();
    }
}

/// Fold a per-field [`SettingUpdate`] (wayland-wire) into
/// the [`SystemSnapshot`] the host maintains per widget slot.
///
/// The host bridges the wayland-wire enums (`bmc_shared_time::*`,
/// `bmc_shared_utils::*`, `bmc_widget_protocol::*`) and the wasmi-wire
/// enums (`bmc_wasm_protocol::system::*`).
///
/// The wasm runtime intentionally does not depend on the bmc-shared crates,
/// so the translation lives here.
fn apply_setting_update(snap: &mut SystemSnapshot, update: &SettingUpdate) {
    match update {
        SettingUpdate::Timezone(tz) => snap.settings.timezone.clone_from(tz),
        SettingUpdate::TimeFormat(t) => snap.settings.time_format = time_format_to_wasmi(*t),
        SettingUpdate::DateFormat(d) => snap.settings.date_format = date_format_to_wasmi(*d),
        SettingUpdate::NumberFormat(n) => snap.settings.number_format = number_format_to_wasmi(*n),
        SettingUpdate::TemperatureUnit(u) => {
            snap.settings.temperature_unit = temperature_unit_to_wasmi(*u);
        }
        SettingUpdate::FirstDayOfWeek(w) => snap.settings.first_day_of_week = weekday_to_wasmi(*w),
        SettingUpdate::UnitSystem(u) => snap.settings.unit_system = unit_system_to_wasmi(*u),
        SettingUpdate::NextAlarm(na) => {
            snap.next_alarm = na.as_ref().map(next_alarm_to_runtime);
        }
        SettingUpdate::NightMode(active) => snap.night_mode = *active,
    }
}

fn time_format_to_wasmi(
    t: bmc_widget_protocol::TimeSystem,
) -> bmc_wasm_protocol::system::TimeFormat {
    use bmc_wasm_protocol::system::TimeFormat as W;
    use bmc_widget_protocol::TimeSystem as H;
    match t {
        H::Hour12 => W::Hour12,
        H::Hour24 => W::Hour24,
    }
}

fn date_format_to_wasmi(
    d: bmc_widget_protocol::DateFormat,
) -> bmc_wasm_protocol::system::DateFormat {
    use bmc_wasm_protocol::system::DateFormat as W;
    use bmc_widget_protocol::DateFormat as H;
    match d {
        H::DdMmYyyyDot => W::DdMmYyyyDot,
        H::DdMmYyyySlash => W::DdMmYyyySlash,
        H::DMYyyySlash => W::DMYyyySlash,
        H::MDYyyySlash => W::MDYyyySlash,
        H::DdMmYyyyDash => W::DdMmYyyyDash,
        H::YyyyMDSlash => W::YyyyMDSlash,
        H::YyyyMmDdDot => W::YyyyMmDdDot,
        H::YyyyMmDdDash => W::YyyyMmDdDash,
    }
}

fn number_format_to_wasmi(
    n: bmc_widget_protocol::NumberFormat,
) -> bmc_wasm_protocol::system::NumberFormat {
    use bmc_wasm_protocol::system::NumberFormat as W;
    use bmc_widget_protocol::NumberFormat as H;
    match n {
        H::SpaceGroupCommaDecimal => W::SpaceGroupCommaDecimal,
        H::CommaGroupDotDecimal => W::CommaGroupDotDecimal,
        H::DotGroupCommaDecimal => W::DotGroupCommaDecimal,
        H::SpaceGroupDotDecimal => W::SpaceGroupDotDecimal,
    }
}

fn temperature_unit_to_wasmi(
    u: bmc_widget_protocol::TemperatureUnit,
) -> bmc_wasm_protocol::system::TemperatureUnit {
    use bmc_wasm_protocol::system::TemperatureUnit as W;
    use bmc_widget_protocol::TemperatureUnit as H;
    match u {
        H::Celsius => W::Celsius,
        H::Fahrenheit => W::Fahrenheit,
    }
}

fn weekday_to_wasmi(w: bmc_widget_protocol::WeekDay) -> bmc_wasm_protocol::system::Weekday {
    use bmc_wasm_protocol::system::Weekday as W;
    use bmc_widget_protocol::WeekDay as H;
    match w {
        H::Monday => W::Monday,
        H::Tuesday => W::Tuesday,
        H::Wednesday => W::Wednesday,
        H::Thursday => W::Thursday,
        H::Friday => W::Friday,
        H::Saturday => W::Saturday,
        H::Sunday => W::Sunday,
    }
}

fn unit_system_to_wasmi(
    u: bmc_widget_protocol::UnitSystem,
) -> bmc_wasm_protocol::system::UnitSystem {
    use bmc_wasm_protocol::system::UnitSystem as W;
    use bmc_widget_protocol::UnitSystem as H;
    match u {
        H::Metric => W::Metric,
        H::Imperial => W::Imperial,
    }
}

fn next_alarm_to_runtime(na: &WireNextAlarm) -> RuntimeNextAlarm {
    RuntimeNextAlarm {
        fire_at_utc_ms: na.fire_at_utc_ms,
        name: na.name.clone(),
    }
}

fn map_protocol_lifecycle(s: bmc_widget_protocol::LifecycleState) -> LifecycleState {
    use bmc_widget_protocol::LifecycleState as P;
    match s {
        P::Dormant => LifecycleState::Dormant,
        P::Prepared => LifecycleState::Prepared,
        P::Entering => LifecycleState::Entering,
        P::Visible => LifecycleState::Visible,
        P::Leaving => LifecycleState::Leaving,
        _ => {
            tracing::warn!("unknown protocol lifecycle state, defaulting to Dormant");
            LifecycleState::Dormant
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostRenderProfiling;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostRenderFrameContext {
    target_width: u32,
    target_height: u32,
    wants_immediate: bool,
    status: RenderStatus,
}

impl HostRenderFrameContext {
    fn new(
        target_width: u32,
        target_height: u32,
        wants_immediate: bool,
        status: RenderStatus,
    ) -> Self {
        Self {
            target_width,
            target_height,
            wants_immediate,
            status,
        }
    }
}

#[cfg(feature = "profiling")]
type HostRenderPhaseStart = Instant;

#[cfg(not(feature = "profiling"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostRenderPhaseStart;

impl HostRenderProfiling {
    #[cfg(feature = "profiling")]
    const DEFAULT_FRAME_SUMMARY_INTERVAL: u64 = 120;

    #[cfg(feature = "profiling")]
    fn start_phase() -> HostRenderPhaseStart {
        Instant::now()
    }

    #[cfg(not(feature = "profiling"))]
    fn start_phase() -> HostRenderPhaseStart {
        HostRenderPhaseStart
    }

    #[cfg(feature = "profiling")]
    fn log_phase(wasm: &str, phase: &str, started: HostRenderPhaseStart) {
        tracing::debug!(
            wasm,
            phase,
            elapsed_us = started.elapsed().as_micros(),
            "wasm host render phase"
        );
    }

    #[cfg(not(feature = "profiling"))]
    fn log_phase(_wasm: &str, _phase: &str, _started: HostRenderPhaseStart) {}

    #[cfg(feature = "profiling")]
    fn log_frame(
        wasm: &str,
        frame: u64,
        delta_ms: u32,
        started: HostRenderPhaseStart,
        context: HostRenderFrameContext,
    ) {
        let elapsed_us = started.elapsed().as_micros();
        #[expect(
            clippy::cast_precision_loss,
            reason = "profiling output only needs approximate FPS"
        )]
        let render_fps = if elapsed_us == 0 {
            f64::INFINITY
        } else {
            1_000_000.0 / elapsed_us as f64
        };

        tracing::debug!(
            wasm,
            frame,
            delta_ms,
            total_us = elapsed_us,
            render_fps,
            target_width = context.target_width,
            target_height = context.target_height,
            wants_immediate = context.wants_immediate,
            status = ?context.status,
            "wasm host render frame"
        );

        if frame.is_multiple_of(Self::DEFAULT_FRAME_SUMMARY_INTERVAL) {
            tracing::info!(
                wasm,
                frame,
                delta_ms,
                total_us = elapsed_us,
                render_fps,
                target_width = context.target_width,
                target_height = context.target_height,
                wants_immediate = context.wants_immediate,
                status = ?context.status,
                "wasm host render frame summary"
            );
        }
    }

    #[cfg(not(feature = "profiling"))]
    fn log_frame(
        _wasm: &str,
        _frame: u64,
        _delta_ms: u32,
        _started: HostRenderPhaseStart,
        _context: HostRenderFrameContext,
    ) {
    }
}

#[expect(
    clippy::cast_possible_wrap,
    reason = "GL viewport dimensions fit in i32"
)]
fn normalize_gl_state(egl: &bmc_widget::egl::EglContext, w: u32, h: u32) {
    use glow::HasContext;
    let gl = egl.gl();
    unsafe {
        gl.viewport(0, 0, w as i32, h as i32);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::STENCIL_TEST);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.disable(glow::BLEND);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        gl.stencil_mask(0xFF);
        gl.active_texture(glow::TEXTURE0);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HostRenderFrameContext, RendererAssetEvictor, SystemSnapshot, apply_setting_update,
        evict_renderer_assets,
    };
    use bmc_widget_protocol::{NextAlarm as WireNextAlarm, SettingUpdate};

    #[test]
    fn apply_setting_update_timezone_folds_into_snapshot() {
        let mut snap = SystemSnapshot::default();
        apply_setting_update(
            &mut snap,
            &SettingUpdate::Timezone("Europe/Prague".to_owned()),
        );
        assert_eq!(snap.settings.timezone, "Europe/Prague");
    }

    #[test]
    fn apply_setting_update_next_alarm_some_translates_payload() {
        let mut snap = SystemSnapshot::default();
        apply_setting_update(
            &mut snap,
            &SettingUpdate::NextAlarm(Some(WireNextAlarm {
                fire_at_utc_ms: 1_700_000_000_000,
                name: "Wake up".to_owned(),
            })),
        );
        let na = snap
            .next_alarm
            .expect("BUG: NextAlarm(Some) must populate the snapshot");
        assert_eq!(na.fire_at_utc_ms, 1_700_000_000_000);
        assert_eq!(na.name, "Wake up");
    }

    #[test]
    fn apply_setting_update_next_alarm_none_clears_snapshot_field() {
        let mut snap = SystemSnapshot {
            next_alarm: Some(bmc_wasm_runtime::NextAlarm {
                fire_at_utc_ms: 1,
                name: "stale".to_owned(),
            }),
            ..SystemSnapshot::default()
        };
        apply_setting_update(&mut snap, &SettingUpdate::NextAlarm(None));
        assert_eq!(snap.next_alarm, None);
    }

    #[test]
    fn apply_setting_update_night_mode_active_sets_snapshot_field() {
        let mut snap = SystemSnapshot::default();
        apply_setting_update(&mut snap, &SettingUpdate::NightMode(true));
        assert!(snap.night_mode);
    }

    #[test]
    fn apply_setting_update_night_mode_inactive_clears_snapshot_field() {
        let mut snap = SystemSnapshot {
            night_mode: true,
            ..SystemSnapshot::default()
        };
        apply_setting_update(&mut snap, &SettingUpdate::NightMode(false));
        assert!(!snap.night_mode);
    }

    #[derive(Debug)]
    struct RecordingEvictor {
        result: usize,
        prefixes: Vec<String>,
    }

    impl RendererAssetEvictor for RecordingEvictor {
        fn evict_renderer_prefix(&mut self, prefix: &str) -> usize {
            self.prefixes.push(prefix.to_owned());
            self.result
        }
    }

    #[test]
    fn evict_renderer_assets_uses_exact_runtime_namespace() {
        let mut evictor = RecordingEvictor {
            result: 3,
            prefixes: Vec::new(),
        };

        assert_eq!(evict_renderer_assets(&mut evictor, "42"), 3);
        assert_eq!(evictor.prefixes, vec!["42".to_owned()]);
    }

    #[test]
    fn host_render_frame_context_carries_summary_dimensions_and_status() {
        let context =
            HostRenderFrameContext::new(638, 480, true, bmc_wasm_runtime::RenderStatus::Ok);

        assert_eq!(context.target_width, 638);
        assert_eq!(context.target_height, 480);
        assert!(context.wants_immediate);
        assert_eq!(context.status, bmc_wasm_runtime::RenderStatus::Ok);
    }
}
