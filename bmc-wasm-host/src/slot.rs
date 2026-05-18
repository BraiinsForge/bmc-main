// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};
use bmc_widget::surface::{DeckWidgetSurfaceClient, WidgetEvent, WidgetSurface};
use glow::HasContext;
use serde_json::{Map, Value};

use crate::host::SharedHost;
use crate::lifecycle::{
    LifecycleState, LifecycleStateMachine, SlotApplyCtx, frame_callback_enabled, should_render,
};
use crate::render_target::{RenderTarget, RenderTargetFactory};

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
    pub factory: Rc<dyn RenderTargetFactory>,
    pub last_render_at: Option<Instant>,
    pub monotonic_origin: Instant,
    pub frame_count: u64,
    pub initial_params: Map<String, Value>,
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
        let mut runtime = WasmWidgetRuntime::new(
            &wasm_bytes,
            initial.width,
            initial.height,
            RuntimeConfig {
                params: bmc_wasm_runtime::parse_params_json(&initial.params).unwrap_or_else(
                    |err| {
                        tracing::warn!(?err, "invalid initial params — empty map");
                        BTreeMap::default()
                    },
                ),
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
            factory,
            last_render_at: None,
            monotonic_origin: Instant::now(),
            frame_count: 0,
            initial_params: initial.params,
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
        for event in <DeckWidgetSurfaceClient as WidgetSurface>::drain_events(&mut self.surface) {
            self.on_wayland_event(event);
        }
        Ok(())
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
    /// dirty surface (lifecycle / params / touch), no animation, no frame callbacks;
    /// Visible / Leaving run the full animation loop. Gate animation-driven renders
    /// on `frame_callback_enabled` so an Entering slot whose runtime returns
    /// `wants_next_frame() == true` does NOT spin a continuous render loop.
    #[must_use]
    pub fn needs_render(&self, now: Instant) -> bool {
        let gate = if !self.is_renderable() {
            RenderGate::NotRenderable
        } else if self.is_blocked() {
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
            is_blocked: self.is_blocked(),
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

    pub fn schedule_next_runtime_frame(&mut self, now: Instant) {
        self.next_frame_due_at = if self.runtime.wants_next_frame() {
            Some(match self.runtime.next_frame_delay() {
                None => now,
                Some(delay_ms) => now + Duration::from_millis(delay_ms.into()),
            })
        } else {
            None
        };
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

    pub fn apply_lifecycle(&mut self, now: Instant, shared: &SharedHost) {
        let previous = self.lifecycle.current();
        let w = self.surface.width();
        let h = self.surface.height();
        let mut ctx = SlotApplyCtx {
            factory: &self.factory,
            egl: &shared.egl,
            surface: &self.surface,
            render_target: &mut self.render_target,
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
        let (dmabuf, slot_idx, status) = {
            let target = self.render_target.as_mut().expect(
                "BUG: render() called on a slot without a render target — \
                 needs_render() should have gated this off when lifecycle ∉ {Entering, Visible, Leaving}",
            );
            let target_width = target.width;
            let target_height = target.height;
            let egl_target = target.as_egl_mut().expect(
                "BUG: EglRenderTargetFactory allocated all WidgetSlot render targets in Task 8",
            );

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
            let status = self.runtime.with_renderer(ptr, |rt| rt.render(delta_ms))?;
            unsafe { ptr.as_ptr().as_mut() }
                .expect(
                    "BUG: renderer pointer was NonNull when stored, \
                     raw-pointer reborrow must produce a non-null reference",
                )
                .flush();

            let current_export = egl_target.buffers.current_ref().expect(
                "BUG: ensure_current succeeded above, so DoubleBufferState::current_ref \
                 must return Some; an internal invariant of DoubleBufferState was violated",
            );
            shared
                .scratch
                .blit_to(&shared.egl, current_export.fbo, target_width, target_height);

            unsafe {
                shared.egl.gl().finish();
            }

            let (dmabuf, slot_idx) = egl_target.buffers.export_and_swap()?;
            (dmabuf, slot_idx, status)
        };

        let wants_immediate = self.frame_callback_enabled()
            && self.runtime.wants_next_frame()
            && self.runtime.next_frame_delay().is_none();
        let target = self
            .render_target
            .as_mut()
            .expect("BUG: render target must still be present after export_and_swap");
        let egl_target = target.as_egl_mut().expect(
            "BUG: EglRenderTargetFactory allocated all WidgetSlot render targets in Task 8",
        );
        self.surface.submit_buffer_with_wl_buffer(
            &dmabuf,
            &egl_target.wl_buffers[slot_idx],
            wants_immediate,
        )?;
        self.surface.flush()?;
        self.schedule_next_runtime_frame(Instant::now());
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

    pub fn shutdown(mut self, shared: &mut SharedHost) {
        let asset_namespace = self.runtime.asset_namespace();
        drop(self.runtime);

        let evicted_renderer_assets = evict_renderer_assets(&mut shared.renderer, &asset_namespace);
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
            self.factory.destroy(target, &shared.egl);
        }
        drop(self.surface);
        drop(self.control_socket);
    }

    fn on_wayland_event(&mut self, event: WidgetEvent) {
        match event {
            WidgetEvent::Lifecycle(state) => {
                let target = map_protocol_lifecycle(state);
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
            WidgetEvent::Setting(setting) => {
                tracing::info!(
                    peer_pid = self.peer_pid,
                    wasm = %self.wasm_basename,
                    ?setting,
                    "setting event received"
                );
            }
            WidgetEvent::Shutdown => {
                tracing::info!(
                    peer_pid = self.peer_pid,
                    wasm = %self.wasm_basename,
                    "shutdown event received"
                );
            }
            WidgetEvent::ParamUpdate(params) => {
                if let Ok(table) = bmc_wasm_runtime::parse_params_json(&params) {
                    tracing::info!(
                        peer_pid = self.peer_pid,
                        wasm = %self.wasm_basename,
                        params = table.len(),
                        "param update received"
                    );
                    self.runtime.deliver_params_update(table);
                    self.surface.mark_needs_render();
                }
            }
            WidgetEvent::TouchDown { x, y, .. } => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "touch coordinates from Wayland are pixel positions; f64→f32 precision loss is acceptable"
                )]
                self.push_touch(bmc_render::interaction::TouchEvent::Down {
                    x: x as f32,
                    y: y as f32,
                });
            }
            WidgetEvent::TouchMotion { x, y, .. } => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "touch coordinates from Wayland are pixel positions; f64→f32 precision loss is acceptable"
                )]
                self.push_touch(bmc_render::interaction::TouchEvent::Move {
                    x: x as f32,
                    y: y as f32,
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
    use super::{RendererAssetEvictor, evict_renderer_assets};

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
}
