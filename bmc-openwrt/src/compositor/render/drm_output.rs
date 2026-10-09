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
#[cfg(feature = "profiling")]
use smithay::utils::{Clock, Monotonic};
use smithay::{
    backend::drm::{DrmDevice, DrmDeviceFd, DrmSurface, PlaneConfig, PlaneDamageClips, PlaneState},
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
    damage_clips_supported: bool,
    /// When the pending flip was committed, on the clock of the kernel's event timestamps.
    #[cfg(feature = "profiling")]
    committed_at: Option<Duration>,
    /// Commit to the kernel's flip-complete timestamp.
    /// MIPI DBI panels send that event once the SPI write ends,
    /// so there it times the panel transfer, without the event loop's latency.
    #[cfg(feature = "profiling")]
    transfer: ii_stopwatch::Jitter,
    #[cfg(feature = "profiling")]
    sent_bytes: usize,
    #[cfg(feature = "profiling")]
    pixel_format: bmc_platform::DisplayPixelFormat,
    /// Start of the window the next [`Self::take_transfer_report`] covers.
    /// Reports come with renders, so an idle window runs long.
    #[cfg(feature = "profiling")]
    report_since: std::time::Instant,
}

impl DrmOutput {
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
        let damage_clips_supported = plane_has_property(&drm, primary_plane, "FB_DAMAGE_CLIPS")
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to read the primary plane's properties: {e:#}");
                false
            });
        tracing::info!("Primary plane takes damage clips: {damage_clips_supported}");

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
            damage_clips_supported,
            #[cfg(feature = "profiling")]
            committed_at: None,
            #[cfg(feature = "profiling")]
            transfer: ii_stopwatch::Jitter::new(),
            #[cfg(feature = "profiling")]
            sent_bytes: 0,
            #[cfg(feature = "profiling")]
            pixel_format: display_profile.pixel_format,
            #[cfg(feature = "profiling")]
            report_since: std::time::Instant::now(),
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

    /// Whether the primary plane has `FB_DAMAGE_CLIPS`,
    /// without which the driver ignores the clips [`Self::page_flip`] takes and sends whole frames.
    pub fn takes_damage_clips(&self) -> bool {
        self.damage_clips_supported
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "wrapping at ~49.7 days is acceptable for frame-callback time"
    )]
    pub fn on_vblank(&mut self, timestamp: Duration) {
        #[cfg(feature = "profiling")]
        if let Some(transfer) = self
            .committed_at
            .take()
            .and_then(|committed_at| timestamp.checked_sub(committed_at))
        {
            self.transfer.add(transfer);
        }
        self.flip_pending = false;
        self.last_vblank_ms = Some(timestamp.as_millis() as u32);
    }

    /// Monotonic timestamp (ms) of the last DRM vblank/page-flip, for use
    /// as the `wl_callback.done` `time` argument. `None` until the first
    /// vblank has been observed.
    pub fn last_vblank_ms(&self) -> Option<u32> {
        self.last_vblank_ms
    }

    /// Put `fb` on the panel.
    /// `clips` limits the transfer to those regions,
    /// which must cover every pixel that differs from the frame the panel holds;
    /// `None` sends the whole frame.
    /// MIPI DBI drivers send the bounding box of all clips as one window.
    pub fn page_flip(
        &mut self,
        fb: framebuffer::Handle,
        clips: Option<&[Rectangle<i32, Physical>]>,
    ) -> Result<()> {
        let src_size: Size<f64, BufferCoord> =
            Size::from((f64::from(self.width), f64::from(self.height)));
        let src_rect = Rectangle::from_size(src_size);

        #[expect(clippy::cast_possible_wrap)]
        let dst_size: Size<i32, Physical> = Size::from((self.width as i32, self.height as i32));
        let dst_rect = Rectangle::from_size(dst_size);
        let damage_clips = clips
            .map(|rects| {
                PlaneDamageClips::from_damage(
                    self.surface.device_fd(),
                    src_rect,
                    dst_rect,
                    rects.iter().copied(),
                )
            })
            .transpose()
            .context("Failed to build plane damage clips")?
            .flatten();

        let plane_config = PlaneConfig {
            src: src_rect,
            dst: dst_rect,
            transform: Transform::Normal,
            alpha: 1.0,
            damage_clips: damage_clips.as_ref().map(PlaneDamageClips::blob),
            fb,
            fence: None,
        };

        let plane_state = PlaneState {
            handle: self.primary_plane,
            config: Some(plane_config),
        };

        #[cfg(feature = "profiling")]
        let committed_at = Duration::from(Clock::<Monotonic>::new().now());
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
        #[cfg(feature = "profiling")]
        {
            self.committed_at = Some(committed_at);
            self.sent_bytes += self.transferred_pixels(clips) * self.pixel_format.bytes_per_pixel();
        }

        Ok(())
    }

    /// Pixels a MIPI DBI driver sends for `clips`: their bounding box.
    #[cfg(feature = "profiling")]
    fn transferred_pixels(&self, clips: Option<&[Rectangle<i32, Physical>]>) -> usize {
        let full = self.width as usize * self.height as usize;
        let Some(bounds) = clips.and_then(|rects| rects.iter().copied().reduce(Rectangle::merge))
        else {
            return full;
        };
        usize::try_from(bounds.size.w).unwrap_or(0) * usize::try_from(bounds.size.h).unwrap_or(0)
    }

    /// Panel transfers since the last call, then reset.
    /// Format: `transfer=<flips>:<avg>/<max> sent=<bytes>B window=<elapsed>`.
    #[cfg(feature = "profiling")]
    pub fn take_transfer_report(&mut self) -> String {
        let report = format!(
            "transfer={}:{:?}/{:?} sent={}B window={:.1?}",
            self.transfer.n,
            self.transfer.avg(),
            self.transfer.max,
            self.sent_bytes,
            self.report_since.elapsed()
        );
        self.transfer.reset();
        self.sent_bytes = 0;
        self.report_since = std::time::Instant::now();
        report
    }
}

fn plane_has_property(drm: &DrmDevice, plane: plane::Handle, name: &str) -> Result<bool> {
    Ok(drm
        .get_properties(plane)?
        .as_hashmap(drm)?
        .contains_key(name))
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
