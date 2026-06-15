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

/// Whether the overlay named `name` is enabled. Each overlay maps to an env var
/// `BMC_OVERLAY_<NAME>` (uppercased, `-`→`_`). Overlays are on by default; only
/// `0`/`false`/`off` (case-insensitive) disable one. Unknown values keep it
/// on. This is a development convenience, not product configuration.
#[must_use]
pub fn overlay_enabled(name: &str) -> bool {
    let var = format!("BMC_OVERLAY_{}", name.to_uppercase().replace('-', "_"));
    match std::env::var(var) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        ),
        Err(_) => true,
    }
}

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
        ("settings-tray", || {
            Box::new(bmc_overlay_settings_tray::SettingsTrayOverlay::default())
        }),
    ];
    let mut overlays = Vec::new();
    for (name, make) in factories {
        if !overlay_enabled(name) {
            tracing::info!("overlay {name} disabled via BMC_OVERLAY_* env var");
            continue;
        }
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
    // Blit-only slide branch: while a slide is animating with unchanged content,
    // skip Taffy layout + femtovg repaint entirely and present the once-painted
    // GPU cache at the current offset. `None` means full-paint this frame
    // (content changed, or no slide).
    let cached_blit = overlay.overlay_mut().wants_cached_blit(now);
    let (dmabuf, slot) = {
        // Lock lifetime matches WidgetSlot::render: held across render+blit+
        // fence-wait, then dropped BEFORE export_and_swap. Bind it and `drop` it
        // explicitly (a bare `let _lock` would drop at block end, after export).
        let gpu_render_lock = shared.acquire_gpu_render_lock("host_system_overlay")?;
        overlay.target_mut().ensure_current(&shared.egl)?;

        if let Some(offset_y) = cached_blit {
            // Animation frame: clear-transparent + shader-copy the cached panel
            // into the export buffer at the slide offset. No layout, no paint.
            #[expect(
                clippy::cast_precision_loss,
                reason = "overlay band height in pixels converts to NDC without meaningful loss"
            )]
            let panel_h = size.1 as f32;
            let fbo = overlay.target_mut().current_fbo();
            overlay.target_mut().blit_cached_panel(
                &shared.egl,
                &shared.scratch,
                fbo,
                size,
                panel_h,
                offset_y,
            )?;
            shared.flush_and_wait_gl();
            drop(gpu_render_lock);
            overlay.target_mut().export_and_swap()?
        } else {
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

            // Refresh the cache from this paint if the content changed, so a
            // later animation frame can present it without repainting.
            if overlay.overlay_mut().take_content_dirty() {
                overlay
                    .target_mut()
                    .capture_panel(&shared.egl, &shared.scratch, size.0, size.1)?;
            }

            // A dirty frame mid-slide must present at the offset, not the
            // settled full-frame blit (which would snap the panel into place).
            // After clearing the dirty flag above, `wants_cached_blit` reports
            // the offset iff a slide is still animating.
            let fbo = overlay.target_mut().current_fbo();
            match overlay.overlay_mut().wants_cached_blit(now) {
                Some(offset_y) if overlay.target_mut().is_cached() => {
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "overlay band height in pixels converts to NDC without meaningful loss"
                    )]
                    let panel_h = size.1 as f32;
                    overlay.target_mut().blit_cached_panel(
                        &shared.egl,
                        &shared.scratch,
                        fbo,
                        size,
                        panel_h,
                        offset_y,
                    )?;
                }
                Some(_) | None => shared.blit_staging_to(fbo, size.0, size.1),
            }
            shared.flush_and_wait_gl();
            drop(gpu_render_lock); // release before the buffer handoff, like slot.rs
            overlay.target_mut().export_and_swap()?
        }
    };
    // Mint+attach the wl_buffer and mark the slot in-flight. Done inside one
    // HostedOverlay method so target and client are borrowed together legally.
    overlay.submit_exported(&dmabuf, slot)?;
    overlay.mark_rendered(now);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::overlay_enabled;

    fn with_var<R>(key: &str, val: Option<&str>, f: impl FnOnce() -> R) -> R {
        let prev = std::env::var_os(key);
        match val {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
        let out = f();
        match prev {
            Some(p) => unsafe { std::env::set_var(key, p) },
            None => unsafe { std::env::remove_var(key) },
        }
        out
    }

    // Each test uses a DISTINCT overlay name: `cargo test`/nextest run tests in
    // parallel and the env is process-global, so two tests touching the same
    // BMC_OVERLAY_* var would race the set/restore.

    #[test]
    fn default_on_when_unset() {
        with_var("BMC_OVERLAY_ALARMS", None, || {
            assert!(overlay_enabled("alarms"));
        });
    }

    #[test]
    fn disabled_by_falsey_values() {
        for v in ["0", "false", "False", "OFF", "off"] {
            with_var("BMC_OVERLAY_DEVICE_INFO", Some(v), || {
                assert!(!overlay_enabled("device-info"), "{v} should disable");
            });
        }
    }

    #[test]
    fn enabled_by_truthy_or_unknown_values() {
        for v in ["1", "true", "on", "yes", "anything"] {
            with_var("BMC_OVERLAY_OFFLINE", Some(v), || {
                assert!(overlay_enabled("offline"), "{v} should keep enabled");
            });
        }
    }

    #[test]
    fn name_maps_to_uppercased_underscored_var() {
        with_var("BMC_OVERLAY_SETTINGS_TRAY", Some("0"), || {
            assert!(!overlay_enabled("settings-tray"));
        });
    }
}
