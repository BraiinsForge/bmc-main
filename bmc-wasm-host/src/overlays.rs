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

use std::ptr::NonNull;
use std::time::Instant;

use bmc_render::renderer::{FrameClear, Renderer};
use bmc_system_overlay::{HostedOverlay, SystemOverlay};
use bmc_widget::egl::EglContext;

use crate::host::SharedHost;

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

struct OverlaySpec {
    name: &'static str,
    make: fn() -> Box<dyn SystemOverlay>,
}

/// Add one compiled-in overlay entry to the ordered registry.
///
/// Each invocation is the single source of truth for a compiled-in overlay:
/// the Cargo feature gate, the runtime name (used for env-var lookup and
/// logging), and the constructor expression all live here and nowhere else.
/// Adding a new overlay = one new `register_overlay!` line.
macro_rules! register_overlay {
    ($specs:expr, $feature:literal, $name:literal, $make:expr) => {
        #[cfg(feature = $feature)]
        {
            $specs.push(OverlaySpec {
                name: $name,
                make: || $make,
            });
        }
    };
}

fn overlay_specs() -> Vec<OverlaySpec> {
    let mut specs = Vec::new();
    register_overlay!(
        specs,
        "overlay-upgrade",
        "upgrade-firmware",
        Box::new(bmc_overlay_upgrade::UpgradeOverlay::firmware())
    );
    register_overlay!(
        specs,
        "overlay-alarm",
        "alarm",
        Box::new(bmc_overlay_alarm::AlarmOverlay::default())
    );
    register_overlay!(
        specs,
        "overlay-upgrade",
        "upgrade-packages",
        Box::new(bmc_overlay_upgrade::UpgradeOverlay::packages())
    );
    register_overlay!(
        specs,
        "overlay-offline",
        "offline",
        Box::new(bmc_overlay_offline::OfflineOverlay::default())
    );
    register_overlay!(
        specs,
        "overlay-device-info",
        "device-info",
        Box::new(bmc_overlay_device_info::DeviceInfoOverlay::default())
    );
    register_overlay!(
        specs,
        "overlay-settings-tray",
        "settings-tray",
        Box::new(bmc_overlay_settings_tray::SettingsTrayOverlay::default())
    );
    specs
}

/// Build the compiled-in system overlays. Each opens its own Wayland
/// connection and allocates buffers from `egl`. A failure to start one overlay
/// is logged and skipped, never fatal to the host.
pub fn build_overlays(egl: &EglContext) -> Vec<HostedOverlay> {
    // Layer rank controls cross-layer stacking. Within Top, firmware registers
    // before alarm so a firing alarm is painted above the upgrade blocker; the
    // package card uses Bottom and therefore remains above the Background
    // offline indicator.
    let mut overlays = Vec::new();
    for spec in overlay_specs() {
        if overlay_enabled(spec.name) {
            match HostedOverlay::connect((spec.make)(), egl) {
                Ok(overlay) => overlays.push(overlay),
                Err(error) => tracing::error!("failed to start {} overlay: {}", spec.name, error),
            }
        } else {
            tracing::info!("overlay {} disabled via BMC_OVERLAY_* env var", spec.name);
        }
    }
    overlays
}

/// Pay each overlay's one-time renderer setup (SVG icon decode/upload, glyph
/// atlas) at startup so the first screen-edge reveal does not stall mid-swipe.
/// Mirrors `render_hosted_overlay`'s GL setup but draws into the host scratch
/// and exports nothing — no overlay buffer is allocated or mapped.
pub fn prewarm_hosted_overlay(
    overlay: &mut HostedOverlay,
    ptr: NonNull<dyn Renderer>,
    shared: &mut SharedHost,
) -> anyhow::Result<()> {
    let (w, h) = overlay.size();
    anyhow::ensure!(
        shared.scratch.supports_size(w, h),
        "host scratch FBO {:?} cannot prewarm overlay {:?}",
        shared.scratch.max_size(),
        (w, h),
    );
    crate::slot::stage_frame_under_gpu_lock(
        shared,
        crate::slot::FrameStaging::Shared,
        "host_overlay_prewarm",
        w,
        h,
        |_| {
            // SAFETY: same invariants as `render_hosted_overlay` — `ptr` is non-null
            // by construction and the renderer outlives this call; the prewarm pass
            // runs before the event loop, one overlay at a time, so no other `&mut
            // dyn Renderer` to this renderer is live.
            let renderer = unsafe { ptr.as_ptr().as_mut() }
                .expect("BUG: NonNull renderer is non-null by construction");
            renderer.begin_frame_with_clear(w, h, 1.0, FrameClear::TransparentBlack);
            overlay.overlay_mut().prewarm(renderer);
            renderer.flush();
            Ok(())
        },
        || {},
    )?;
    Ok(())
}

/// Move a clean mapped slide frame through layer-shell placement;
/// otherwise paint and submit a fresh buffer through the shared renderer.
pub fn render_hosted_overlay(
    overlay: &mut HostedOverlay,
    ptr: NonNull<dyn Renderer>,
    shared: &mut SharedHost,
    now: Instant,
) -> anyhow::Result<()> {
    #[cfg(feature = "profiling")]
    let frame_start = Instant::now();
    overlay.prepare_for_render_with_target_resize(|target, client, width, height| {
        shared.with_gpu_render_lock("host_overlay_resize", |shared| {
            target.resize(&shared.egl, client, width, height)
        })
    })?;
    let size = overlay.size();
    anyhow::ensure!(
        shared.scratch.supports_size(size.0, size.1),
        "host scratch FBO {:?} cannot render resized overlay {:?}",
        shared.scratch.max_size(),
        size,
    );
    #[expect(
        clippy::cast_possible_truncation,
        reason = "layer-shell margins are integer pixels"
    )]
    let layer_offset = overlay
        .overlay_mut()
        .layer_shell_offset(now)
        .map(|y| y.round() as i32);
    if let Some(offset) = layer_offset
        && overlay.overlay_mut().can_reuse_content(now)
        && overlay.commit_position_if_mapped(offset)?
    {
        overlay.mark_rendered(Instant::now());
        return Ok(());
    }
    crate::slot::stage_frame_under_gpu_lock(
        shared,
        crate::slot::FrameStaging::Shared,
        "host_system_overlay",
        size.0,
        size.1,
        |shared| {
            overlay.target_mut().ensure_current(&shared.egl)?;
            // SAFETY: the renderer outlives this call and the single-threaded host
            // does not hold another mutable renderer reference during this pass.
            let renderer = unsafe { ptr.as_ptr().as_mut() }
                .expect("BUG: NonNull renderer is non-null by construction");
            renderer.begin_frame_with_clear(size.0, size.1, 1.0, FrameClear::TransparentBlack);
            overlay.overlay_mut().render(renderer, size);
            renderer.flush();
            let _ = overlay.overlay_mut().take_content_dirty();
            let fbo = overlay.target_mut().current_fbo();
            shared.blit_staging_to(fbo, size.0, size.1);
            Ok(())
        },
        || {},
    )?;
    if let Some(offset) = layer_offset {
        overlay.stage_vertical_offset(offset)?;
    }
    let (dmabuf, slot) = overlay.target_mut().export_and_swap()?;
    overlay.submit_exported(&dmabuf, slot)?;
    // Fresh timestamp: the pass-level `now` predates this pass's widget
    // renders, and anchoring a ramp that far back would replay the jitter
    // this frame just paid.
    overlay.mark_rendered(Instant::now());
    #[cfg(feature = "profiling")]
    if let Some(offset) = layer_offset {
        tracing::info!(target: bmc_render::profile::TARGET, offset_y = offset,
            frame_us = frame_start.elapsed().as_micros(),
            "system overlay buffer frame submitted");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{overlay_enabled, overlay_specs};

    #[cfg(all(
        feature = "overlay-alarm",
        feature = "overlay-device-info",
        feature = "overlay-offline",
        feature = "overlay-settings-tray",
        feature = "overlay-upgrade"
    ))]
    #[test]
    fn default_registry_keeps_alarm_above_firmware() {
        let names: Vec<_> = overlay_specs().into_iter().map(|spec| spec.name).collect();

        assert_eq!(
            names,
            [
                "upgrade-firmware",
                "alarm",
                "upgrade-packages",
                "offline",
                "device-info",
                "settings-tray",
            ]
        );
    }

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
