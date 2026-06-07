// Copyright (C) 2026  Braiins Systems s.r.o.

//! Standalone overlay process: own EGL/renderer/connection and a poll loop.

use std::time::{Duration, Instant};

use bmc_gpu_render_lock::GpuRenderLock;
use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer as _;
use bmc_widget::egl::{EglContext, SharedRenderScratch};

use crate::gpu::{OverlayRenderTarget, wait_for_gpu};
use crate::overlay::{LayerConfig, SystemOverlay, resolved_configured_size};
use crate::surface::LayerSurfaceClient;

const MIN_INTER_FRAME: Duration = Duration::from_millis(8);

/// Run an overlay as its own process: own connection, renderer, and loop.
pub fn run_standalone(mut overlay: Box<dyn SystemOverlay>) -> anyhow::Result<()> {
    let config: LayerConfig = overlay.layer_config();

    // Connect and configure the layer surface; learn the real size.
    let mut client = LayerSurfaceClient::connect(&config)?;
    let mut size = resolved_configured_size(config.size, client.size());

    // GPU stack (owned in standalone mode).
    let egl = EglContext::new()?;
    let mut scratch = SharedRenderScratch::new(&egl, size.0, size.1)?;
    let gpu_lock = GpuRenderLock::from_env()?;
    // SAFETY: EglContext::new makes the GL context current on this thread; the standalone process renders single-threaded.
    let mut renderer = unsafe {
        FemtoVgRenderer::new(
            EglContext::get_proc_address,
            size.0,
            size.1,
            scratch.staging_fbo_id(),
            0,
        )?
    };
    let mut target = OverlayRenderTarget::new(&egl, size.0, size.1)?;

    overlay.init();
    let mut last_render: Option<Instant> = None;
    // A wanted render that can't run yet (no free buffer slot, or inter-frame
    // throttle) must NOT be lost — it stays pending until a frame actually
    // renders. take_needs_render()/tick are consumed every pass, so we latch
    // the request here rather than re-reading it.
    let mut pending_render = false;

    while client.running() {
        // Drain what the previous poll_dispatch delivered.
        for ev in client.drain_touch() {
            overlay.on_touch(ev);
        }
        for released in client.drain_released_buffers() {
            target.mark_released_buffer(&released);
        }
        if let Some(configured_size) = client.take_configured_size_change() {
            let new_size = resolved_configured_size(config.size, configured_size);
            if new_size != size {
                resize_standalone_rendering(
                    &egl,
                    &mut scratch,
                    &mut renderer,
                    &mut target,
                    &mut client,
                    new_size,
                )?;
                size = new_size;
            }
            pending_render = true;
        }

        let now = Instant::now();
        let tick = overlay.tick(now);
        if tick.wants_render || client.take_needs_render() {
            pending_render = true;
        }

        // Remaining time on the inter-frame floor, if a render was throttled.
        let inter_frame_remaining = last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(now.duration_since(t)))
            .filter(|d| !d.is_zero());

        // `target.available()` gates on a free (released) export slot, so we
        // never draw into a buffer the compositor is still displaying.
        if pending_render && target.available() && inter_frame_remaining.is_none() {
            render_frame(
                &egl,
                &scratch,
                &gpu_lock,
                &mut target,
                &mut renderer,
                &mut *overlay,
                &mut client,
                size,
            )?;
            pending_render = false;
            last_render = Some(now);
        }

        let timeout = if pending_render && inter_frame_remaining.is_some() {
            inter_frame_remaining
        } else if pending_render {
            None
        } else {
            tick.next_wake.map(|t| t.saturating_duration_since(now))
        };
        let timeout_ms = timeout.map_or(-1, |d| {
            i32::try_from(d.as_millis().max(1)).unwrap_or(i32::MAX)
        });
        client.poll_dispatch(timeout_ms)?;
    }
    // Not called on the ? error paths above: the process exits shortly after and the Wayland connection close releases the wl_buffers.
    // DoubleBufferState does not free on Drop; release GL/EGL/GBM explicitly.
    drop(renderer);
    target.destroy(&egl);
    scratch.destroy(&egl);
    Ok(())
}

fn resize_standalone_rendering(
    egl: &EglContext,
    scratch: &mut SharedRenderScratch,
    renderer: &mut FemtoVgRenderer,
    target: &mut OverlayRenderTarget,
    client: &mut LayerSurfaceClient,
    size: (u32, u32),
) -> anyhow::Result<()> {
    let new_scratch = SharedRenderScratch::new(egl, size.0, size.1)?;
    // SAFETY: `egl` is current on this single-threaded standalone render loop,
    // and `new_scratch` owns the FBO id passed as the renderer's screen target.
    let new_renderer = unsafe {
        FemtoVgRenderer::new(
            EglContext::get_proc_address,
            size.0,
            size.1,
            new_scratch.staging_fbo_id(),
            0,
        )?
    };
    let old_scratch = std::mem::replace(scratch, new_scratch);
    *renderer = new_renderer;
    old_scratch.destroy(egl);
    target.resize(egl, client, size.0, size.1);
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "render owns the full GPU stack plus connection; no meaningful grouping"
)]
fn render_frame(
    egl: &EglContext,
    scratch: &SharedRenderScratch,
    gpu_lock: &GpuRenderLock,
    target: &mut OverlayRenderTarget,
    renderer: &mut FemtoVgRenderer,
    overlay: &mut dyn SystemOverlay,
    client: &mut LayerSurfaceClient,
    size: (u32, u32),
) -> anyhow::Result<()> {
    let lock = gpu_lock.lock("system_overlay_standalone")?;
    target.ensure_current(egl)?;
    // begin_frame binds the staging FBO and sets the viewport, returning its id. The renderer was constructed against that id, so the returned value is unused.
    let _staging = scratch.begin_frame(egl, size.0, size.1);
    renderer.begin_frame(size.0, size.1, 1.0);
    // BOTH scratch.begin_frame and FemtoVgRenderer::begin_frame clear opaque
    // black. A see-through overlay must start transparent, so re-clear the bound
    // staging FBO to alpha 0 AFTER femtovg's clear and before drawing — the
    // recorded draws then flush over a transparent base.
    // SAFETY: the EGL context is current on this (single render) thread; only GL state calls are made.
    unsafe {
        use glow::HasContext as _;
        let gl = egl.gl();
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
    overlay.render(renderer, size);
    renderer.flush();
    scratch.blit_to(egl, target.current_fbo(), size.0, size.1);
    wait_for_gpu(egl);
    drop(lock);
    let (dmabuf, slot) = target.export_and_swap()?;
    // Mint+cache the wl_buffer for this slot, then attach. mark_presented marks
    // the slot in-flight until the compositor sends wl_buffer.release.
    let wl_buffer = target.wl_buffer_for_slot(client, &dmabuf, slot)?;
    client.submit_buffer_with_wl_buffer(&dmabuf, &wl_buffer)?;
    client.flush()?;
    target.mark_presented(slot);
    Ok(())
}
