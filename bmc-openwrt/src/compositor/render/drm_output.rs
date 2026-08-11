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

//! DRM output device management.

use anyhow::{Context, Result};
use smithay::{
    backend::drm::{DrmDevice, DrmDeviceFd, DrmSurface, PlaneConfig, PlaneState},
    reexports::drm::control::{Device as ControlDevice, Mode, connector, crtc, framebuffer, plane},
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
};
use std::{fs::OpenOptions, os::unix::io::OwnedFd, path::Path, time::Duration};

pub struct DrmOutput {
    drm: DrmDevice,
    surface: DrmSurface,
    primary_plane: plane::Handle,
    width: u32,
    height: u32,
    logical_width: u32,
    logical_height: u32,
    /// Refresh rate of the selected mode, in mHz (matches `wl_output::mode`
    /// units so it can be forwarded directly to Wayland clients).
    refresh_mhz: i32,
    frame_count: u32,
    flip_pending: bool,
    /// Monotonic timestamp of the last DRM vblank/page-flip event, in ms.
    /// Wayland frame callbacks use this as their `time` argument — the spec
    /// only requires monotonicity from an unspecified epoch, and using the
    /// kernel-delivered vblank timestamp is both cheaper than sampling our
    /// own clock at fire time and more accurate (it's the real presentation
    /// time, not an approximation). `None` until the first vblank.
    last_vblank_ms: Option<u32>,
}

impl DrmOutput {
    /// Whether [`Self::page_flip`] hands damage clips to KMS, `false` while
    /// the Etnaviv stall documented there is uncharacterised. A const rather
    /// than a bare `None` at the call site so the preconditions for
    /// re-enabling can assert on it at compile time — see the assert beside
    /// the overlay draws in `scene_renderer`, which repaint without reporting
    /// damage.
    pub const DAMAGE_CLIPS_ENABLED: bool = false;

    pub fn new(display_path: &Path, display_profile: bmc_platform::DisplayProfile) -> Result<Self> {
        tracing::info!("Opening display device: {:?}", display_path);

        let display_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(display_path)
            .context("Failed to open display device")?;

        let display_fd = DrmDeviceFd::new(OwnedFd::from(display_file).into());
        let (mut drm, _notifier) =
            DrmDevice::new(display_fd, false).context("Failed to create DRM device")?;

        let (connector, crtc, mode) = Self::find_display_config(&drm)?;

        tracing::info!(
            "Display: {}x{} @ {}Hz",
            mode.size().0,
            mode.size().1,
            mode.vrefresh()
        );

        let surface = drm
            .create_surface(crtc, mode, &[connector])
            .context("Failed to create DRM surface")?;

        let planes = surface.planes();
        let primary_plane = planes.primary.first().context("No primary plane")?.handle;

        let mode_width = u32::from(mode.size().0);
        let mode_height = u32::from(mode.size().1);
        let visible_area = display_profile.visible_area;
        let width = visible_area.width;
        let height = visible_area.height;
        let (logical_width, logical_height) = logical_size_from_profile(&display_profile);
        if (mode_width, mode_height) != (visible_area.width, visible_area.height) {
            tracing::warn!(
                mode_width,
                mode_height,
                visible_width = visible_area.width,
                visible_height = visible_area.height,
                "panel-reported mode does not match profile visible area; using profile",
            );
        }

        // `mode.vrefresh()` reports Hz; `wl_output::mode` expects mHz.
        #[expect(
            clippy::cast_possible_wrap,
            reason = "vrefresh is small (tens of Hz); * 1000 fits in i32"
        )]
        let refresh_mhz = (mode.vrefresh() * 1_000) as i32;

        tracing::info!(
            "Buffer dimensions: {}x{} physical, {}x{} logical (mode {}x{})",
            width,
            height,
            logical_width,
            logical_height,
            mode_width,
            mode_height
        );

        Ok(Self {
            drm,
            surface,
            primary_plane,
            width,
            height,
            logical_width,
            logical_height,
            refresh_mhz,
            frame_count: 0,
            flip_pending: false,
            last_vblank_ms: None,
        })
    }

    fn find_display_config(drm: &DrmDevice) -> Result<(connector::Handle, crtc::Handle, Mode)> {
        let resources = drm
            .resource_handles()
            .context("Failed to get DRM resources")?;

        for conn_handle in resources.connectors() {
            let conn = drm.get_connector(*conn_handle, true)?;

            if conn.state() != connector::State::Connected {
                continue;
            }

            tracing::info!("Found display: {:?} ({:?})", conn_handle, conn.interface());

            let mode = conn
                .modes()
                .iter()
                .find(|m| {
                    m.mode_type()
                        .contains(smithay::reexports::drm::control::ModeTypeFlags::PREFERRED)
                })
                .or_else(|| conn.modes().first())
                .copied()
                .context("No display mode")?;

            let encoder = conn
                .current_encoder()
                .or_else(|| conn.encoders().first().copied())
                .context("No encoder")?;

            let encoder_info = drm.get_encoder(encoder)?;
            let crtc = encoder_info
                .crtc()
                .or_else(|| resources.crtcs().first().copied())
                .context("No CRTC")?;

            return Ok((*conn_handle, crtc, mode));
        }

        anyhow::bail!("No connected display found")
    }

    pub fn drm(&self) -> &DrmDevice {
        &self.drm
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Refresh rate of the selected DRM mode, in mHz.
    pub fn refresh_mhz(&self) -> i32 {
        self.refresh_mhz
    }

    pub fn logical_size(&self) -> (u32, u32) {
        (self.logical_width, self.logical_height)
    }

    pub fn is_flip_pending(&self) -> bool {
        self.flip_pending
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "wrapping at ~49.7 days is acceptable for frame-callback time"
    )]
    pub fn on_vblank(&mut self, timestamp: Duration) {
        self.flip_pending = false;
        self.last_vblank_ms = Some(timestamp.as_millis() as u32);
    }

    /// Monotonic timestamp (ms) of the last DRM vblank/page-flip, for use
    /// as the `wl_callback.done` `time` argument. `None` until the first
    /// vblank has been observed.
    pub fn last_vblank_ms(&self) -> Option<u32> {
        self.last_vblank_ms
    }

    pub fn page_flip(
        &mut self,
        fb: framebuffer::Handle,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<()> {
        let src_size: Size<f64, BufferCoord> =
            Size::from((f64::from(self.width), f64::from(self.height)));
        let src_rect = Rectangle::from_size(src_size);

        #[expect(clippy::cast_possible_wrap)]
        let dst_size: Size<i32, Physical> = Size::from((self.width as i32, self.height as i32));
        let dst_rect = Rectangle::from_size(dst_size);

        // Passing a `PlaneDamageClips` blob on every atomic commit — the
        // originally-intended consumer of the `damage` argument — caused
        // the Etnaviv KMS path to periodically stall for ~300 ms on the
        // Deck's Vivante GC400, manifesting as sub-20 Hz choppiness
        // (see docs/devlogs/BDK-389-combined-scene/glyph-damage-bisect).
        // Until that's characterised upstream or behind a GPU probe, keep
        // `Self::DAMAGE_CLIPS_ENABLED` false and the argument in the
        // signature so the damage-tracker plumbing in scene_renderer stays in
        // place. Re-enabling means flipping that const and building the clips
        // blob here — and the compile-time assert keyed off it in
        // scene_renderer then fails until the scene placeholder and separator
        // draws, which repaint without reporting damage, are damage-tracked
        // too. Otherwise their
        // appearance/disappearance (e.g. a logo scene receiving its first
        // widget commit) leaves stale overlay pixels on the panel.
        let _ = damage;

        let plane_config = PlaneConfig {
            src: src_rect,
            dst: dst_rect,
            transform: Transform::Normal,
            alpha: 1.0,
            // Gated by `Self::DAMAGE_CLIPS_ENABLED`; see above.
            damage_clips: None,
            fb,
            fence: None,
        };

        let plane_state = PlaneState {
            handle: self.primary_plane,
            config: Some(plane_config),
        };

        if self.frame_count == 0 {
            self.surface
                .commit([plane_state], true)
                .context("Initial commit failed")?;
            tracing::info!("Initial frame committed");
        } else {
            self.surface
                .page_flip([plane_state], true)
                .context("Page flip failed")?;
        }

        self.flip_pending = true;
        self.frame_count = self.frame_count.wrapping_add(1);

        Ok(())
    }
}

#[must_use]
fn logical_size_from_profile(display_profile: &bmc_platform::DisplayProfile) -> (u32, u32) {
    (
        display_profile.logical_width,
        display_profile.logical_height,
    )
}

#[cfg(test)]
mod tests {
    use super::logical_size_from_profile;
    use bmc_platform::{HardwareProfile, Product};

    #[test]
    fn logical_size_uses_profile_values_without_forcing_axis_swap() {
        let bmm100 = HardwareProfile::for_product(Product::Bmm100);
        assert_eq!(
            logical_size_from_profile(&bmm100.display),
            (320, 240),
            "BMM100 has a Deg0 scanout transform, so logical size must not be swapped",
        );

        let bmm101 = HardwareProfile::for_product(Product::Bmm101);
        assert_eq!(
            logical_size_from_profile(&bmm101.display),
            (480, 320),
            "BMM101 has a Deg0 scanout transform, so logical size must not be swapped",
        );
    }
}
