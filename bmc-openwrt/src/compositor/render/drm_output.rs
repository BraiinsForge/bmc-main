// Copyright (C) 2025  Braiins Systems s.r.o.

//! DRM output device management.

use anyhow::{Context, Result};
use smithay::{
    backend::drm::{DrmDevice, DrmDeviceFd, DrmSurface, PlaneConfig, PlaneState},
    reexports::drm::control::{Device as ControlDevice, Mode, connector, crtc, framebuffer, plane},
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
};
use std::{fs::OpenOptions, os::unix::io::OwnedFd, path::Path};

pub struct DrmOutput {
    drm: DrmDevice,
    surface: DrmSurface,
    primary_plane: plane::Handle,
    width: u32,
    height: u32,
    /// Refresh rate of the selected mode, in mHz (matches `wl_output::mode`
    /// units so it can be forwarded directly to Wayland clients).
    refresh_mhz: i32,
    frame_count: u32,
    flip_pending: bool,
}

impl DrmOutput {
    pub fn new(display_path: &Path) -> Result<Self> {
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
        // Panel reports 600x1280 but only 480x1280 is visible
        let width = if mode_width == 600 { 480 } else { mode_width };
        let height = mode_height;

        // `mode.vrefresh()` reports Hz; `wl_output::mode` expects mHz.
        #[expect(
            clippy::cast_possible_wrap,
            reason = "vrefresh is small (tens of Hz); * 1000 fits in i32"
        )]
        let refresh_mhz = (mode.vrefresh() * 1_000) as i32;

        tracing::info!(
            "Buffer dimensions: {}x{} (mode {}x{})",
            width,
            height,
            mode_width,
            mode_height
        );

        Ok(Self {
            drm,
            surface,
            primary_plane,
            width,
            height,
            refresh_mhz,
            frame_count: 0,
            flip_pending: false,
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

    /// Logical size (rotated for landscape orientation)
    pub fn logical_size(&self) -> (u32, u32) {
        (self.height, self.width)
    }

    pub fn is_flip_pending(&self) -> bool {
        self.flip_pending
    }

    pub fn on_vblank(&mut self) {
        self.flip_pending = false;
    }

    pub fn page_flip(&mut self, fb: framebuffer::Handle) -> Result<()> {
        let src_size: Size<f64, BufferCoord> =
            Size::from((f64::from(self.width), f64::from(self.height)));
        let src_rect = Rectangle::from_size(src_size);

        #[expect(clippy::cast_possible_wrap)]
        let dst_size: Size<i32, Physical> = Size::from((self.width as i32, self.height as i32));
        let dst_rect = Rectangle::from_size(dst_size);

        let plane_config = PlaneConfig {
            src: src_rect,
            dst: dst_rect,
            transform: Transform::Normal,
            alpha: 1.0,
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
                .commit([plane_state].into_iter(), true)
                .context("Initial commit failed")?;
            tracing::info!("Initial frame committed");
        } else {
            self.surface
                .page_flip([plane_state].into_iter(), true)
                .context("Page flip failed")?;
        }

        self.flip_pending = true;
        self.frame_count = self.frame_count.wrapping_add(1);

        Ok(())
    }
}
