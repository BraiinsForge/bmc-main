// Copyright (C) 2026  Braiins Systems s.r.o.

use std::ptr::NonNull;
use std::time::Instant;

use bmc_overlay_device_info::DeviceInfoOverlay;
use bmc_overlay_offline::OfflineOverlay;
use bmc_render::renderer::{FrameClear, Renderer};
use bmc_system_overlay::{HostedOverlay, SystemOverlay};
use bmc_widget::egl::EglContext;

use crate::host::SharedHost;

type OverlayFactory = (&'static str, fn() -> Box<dyn SystemOverlay>);

/// Build the compiled-in system overlays. Each opens its own Wayland
/// connection and allocates buffers from `egl`. A failure to start one overlay
/// is logged and skipped, never fatal to the host.
pub fn build_overlays(egl: &EglContext) -> Vec<HostedOverlay> {
    // Stacking is by layer rank, not build order: the offline indicator is on
    // the Bottom layer and the startup overlay on Top, so the fullscreen startup
    // overlay occludes the offline chip regardless of the order built here.
    let factories: Vec<OverlayFactory> = vec![
        ("offline", || Box::new(OfflineOverlay::default())),
        ("device-info", || Box::new(DeviceInfoOverlay::default())),
    ];
    let mut overlays = Vec::new();
    for (name, make) in factories {
        match HostedOverlay::connect(make(), egl) {
            Ok(o) => overlays.push(o),
            Err(e) => tracing::error!("failed to start {name} overlay: {e}"),
        }
    }
    overlays
}

/// Render one hosted overlay through the shared renderer, mirroring
/// `WidgetSlot::render`: lock GPU, stage, draw, blit, fence-wait, export, attach.
pub fn render_hosted_overlay(
    overlay: &mut HostedOverlay,
    ptr: NonNull<dyn Renderer>,
    shared: &mut SharedHost,
    now: Instant,
) -> anyhow::Result<()> {
    overlay.prepare_for_render(&shared.egl)?;
    let size = overlay.size();
    anyhow::ensure!(
        shared.scratch.supports_size(size.0, size.1),
        "host scratch FBO {:?} cannot render resized overlay {:?}",
        shared.scratch.max_size(),
        size,
    );
    let (dmabuf, slot) = {
        // Lock lifetime matches WidgetSlot::render: held across render+blit+
        // fence-wait, then dropped BEFORE export_and_swap. Bind it and `drop` it
        // explicitly (a bare `let _lock` would drop at block end, after export).
        let gpu_render_lock = shared.acquire_gpu_render_lock("host_system_overlay")?;
        overlay.target_mut().ensure_current(&shared.egl)?;
        let _staging = shared.scratch.begin_frame(&shared.egl, size.0, size.1);
        crate::slot::normalize_gl_state(&shared.egl, size.0, size.1);

        // SAFETY: two invariants hold here.
        // (1) Non-null + outlives-call: `ptr` is `NonNull<dyn Renderer>`, so
        //     the address is guaranteed non-null, and the renderer is owned by
        //     `main_loop::run` for the entire program lifetime — it outlives
        //     this call.
        // (2) Aliasing: `SharedHost` does not own the renderer (see its
        //     aliasing invariant in host.rs); the host renders components
        //     strictly one at a time in its single render loop, so no other
        //     `&mut dyn Renderer` to this renderer is live during the call.
        let renderer = unsafe { ptr.as_ptr().as_mut() }
            .expect("BUG: NonNull renderer is non-null by construction");
        renderer.begin_frame_with_clear(size.0, size.1, 1.0, FrameClear::TransparentBlack);
        overlay.overlay_mut().render(renderer, size);
        renderer.flush();

        let fbo = overlay.target_mut().current_fbo();
        shared.blit_staging_to(fbo, size.0, size.1);
        shared.flush_and_wait_gl();
        drop(gpu_render_lock); // release before the buffer handoff, like slot.rs
        overlay.target_mut().export_and_swap()?
    };
    // Mint+attach the wl_buffer and mark the slot in-flight. Done inside one
    // HostedOverlay method so target and client are borrowed together legally.
    overlay.submit_exported(&dmabuf, slot)?;
    overlay.mark_rendered(now);
    Ok(())
}
