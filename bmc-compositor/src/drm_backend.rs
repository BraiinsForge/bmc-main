// Copyright (C) 2025  Braiins Systems s.r.o.

//! DRM/KMS backend for direct framebuffer access
//!
//! This module provides direct DRM access without requiring a seat daemon.
//! Suitable for embedded devices where we run as root with exclusive display access.

use anyhow::{Context, Result};
use smithay::{
    backend::drm::{DrmDevice, DrmDeviceFd, DrmNode},
    reexports::{
        calloop::EventLoop,
        drm::control::{Device as ControlDevice, Mode, connector, crtc},
        gbm::Device as GbmDevice,
    },
    utils::{Physical, Size},
};
use std::{
    fs::OpenOptions,
    os::unix::io::OwnedFd,
    path::{Path, PathBuf},
};

/// Display dimensions for the Braiins Deck (after 90-degree rotation)
/// The physical panel is 480x1280 (portrait), rotated to 1280x480 for landscape UI
/// Note: The panel may report 600x1280 but the actual visible area is 480x1280
pub const DISPLAY_WIDTH: u32 = 1280;
pub const DISPLAY_HEIGHT: u32 = 480;

/// DRM backend state using direct device access
#[expect(
    missing_debug_implementations,
    reason = "contains non-Debug smithay types"
)]
pub struct DrmBackendState {
    /// Primary GPU node
    pub primary_gpu: Option<DrmNode>,

    /// Device state (once initialized)
    pub device: Option<DeviceState>,
}

/// Per-device state
#[expect(
    missing_debug_implementations,
    reason = "contains non-Debug smithay types"
)]
pub struct DeviceState {
    /// DRM device
    pub drm: DrmDevice,

    /// GBM allocator for buffers
    pub gbm: GbmDevice<DrmDeviceFd>,

    /// Path to the device
    pub path: PathBuf,

    /// Active connector
    pub connector: Option<connector::Handle>,

    /// Active CRTC
    pub crtc: Option<crtc::Handle>,

    /// Display mode
    pub mode: Option<Mode>,
}

/// Information about the configured display
#[derive(Debug, Clone)]
pub struct DisplayInfo {
    /// Display width in pixels
    pub width: u32,
    /// Display height in pixels
    pub height: u32,
    /// Refresh rate in millihertz (e.g., 60000 = 60Hz)
    pub refresh_mhz: u32,
}

impl DrmBackendState {
    /// Initialize the DRM backend with direct device access
    pub fn new() -> Result<(EventLoop<'static, Self>, Self)> {
        // Create event loop
        let event_loop: EventLoop<'_, Self> =
            EventLoop::try_new().context("Failed to create event loop")?;

        let state = Self {
            primary_gpu: None,
            device: None,
        };

        Ok((event_loop, state))
    }

    /// Initialize the DRM backend without creating an event loop
    /// Use this when the event loop is managed externally
    #[must_use]
    pub fn new_without_event_loop() -> Self {
        Self {
            primary_gpu: None,
            device: None,
        }
    }

    /// Find the primary GPU using udev
    /// Tries all card devices and returns one that has connectors (actual display)
    pub fn find_primary_gpu(&mut self) -> Result<DrmNode> {
        let mut enumerator =
            ::udev::Enumerator::new().context("Failed to create udev enumerator")?;

        // Filter to DRM subsystem
        enumerator
            .match_subsystem("drm")
            .context("Failed to set udev subsystem filter")?;

        // Collect all card devices
        let mut cards: Vec<_> = enumerator
            .scan_devices()?
            .filter_map(|device| {
                device.devnode().and_then(|devnode| {
                    let devnode_str = devnode.to_string_lossy();
                    if devnode_str.contains("/dev/dri/card") {
                        Some(devnode.to_path_buf())
                    } else {
                        None
                    }
                })
            })
            .collect();

        // Sort to get consistent ordering (card0, card1, etc.)
        cards.sort();

        tracing::info!("Found {} DRM card(s): {:?}", cards.len(), cards);

        // Try each card, prefer one with connectors
        for card_path in &cards {
            tracing::info!("Trying DRM card: {:?}", card_path);

            if let Ok(file) = OpenOptions::new().read(true).write(true).open(card_path) {
                let owned_fd: OwnedFd = file.into();
                let drm_fd = DrmDeviceFd::new(owned_fd.into());

                if let Ok((drm, _)) = DrmDevice::new(drm_fd, false) {
                    if let Ok(resources) = drm.resource_handles() {
                        let num_connectors = resources.connectors().len();
                        tracing::info!(
                            "  {:?}: {} connectors, {} CRTCs",
                            card_path,
                            num_connectors,
                            resources.crtcs().len()
                        );

                        // Use the first card that has connectors
                        if num_connectors > 0 {
                            if let Ok(node) = DrmNode::from_path(card_path) {
                                tracing::info!("Selected DRM card: {:?}", card_path);
                                self.primary_gpu = Some(node);
                                return Ok(node);
                            }
                        }
                    }
                }
            }
        }

        anyhow::bail!("No DRM device with connectors found")
    }

    /// Open and initialize the DRM device directly
    pub fn init_device(&mut self, path: &Path) -> Result<()> {
        tracing::info!("Opening DRM device: {:?}", path);

        // Open device directly (requires root or video group membership)
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .context("Failed to open DRM device - are you root?")?;

        let owned_fd: OwnedFd = file.into();
        let drm_fd = DrmDeviceFd::new(owned_fd.into());

        // Create DRM device
        let (drm, _drm_notifier) =
            DrmDevice::new(drm_fd.clone(), false).context("Failed to create DRM device")?;

        // Create GBM device for buffer allocation
        let gbm = GbmDevice::new(drm_fd).context("Failed to create GBM device")?;

        // Log available resources
        let resources = drm
            .resource_handles()
            .context("Failed to get DRM resources")?;

        tracing::info!(
            "DRM resources: {} connectors, {} CRTCs, {} encoders",
            resources.connectors().len(),
            resources.crtcs().len(),
            resources.encoders().len()
        );

        // Log connector info
        for conn_handle in resources.connectors() {
            if let Ok(conn) = drm.get_connector(*conn_handle, false) {
                tracing::info!(
                    "  Connector {:?}: {:?}, {:?}",
                    conn_handle,
                    conn.interface(),
                    conn.state()
                );
            }
        }

        self.device = Some(DeviceState {
            drm,
            gbm,
            path: path.to_path_buf(),
            connector: None,
            crtc: None,
            mode: None,
        });

        tracing::info!("DRM device initialized successfully");

        Ok(())
    }

    /// Configure the display by finding a connected output and setting a mode
    pub fn configure_display(&mut self) -> Result<DisplayInfo> {
        let device = self
            .device
            .as_mut()
            .context("Device not initialized - call init_device first")?;

        let resources = device
            .drm
            .resource_handles()
            .context("Failed to get DRM resources")?;

        // Find a connected connector
        let mut selected_connector = None;
        let mut selected_mode = None;

        for conn_handle in resources.connectors() {
            let conn = device
                .drm
                .get_connector(*conn_handle, true)
                .context("Failed to get connector info")?;

            if conn.state() == connector::State::Connected {
                tracing::info!(
                    "Found connected connector: {:?} ({:?})",
                    conn_handle,
                    conn.interface()
                );

                // Log available modes
                for mode in conn.modes() {
                    tracing::info!(
                        "  Mode: {}x{}@{}mHz {:?}",
                        mode.size().0,
                        mode.size().1,
                        mode.vrefresh(),
                        mode.mode_type()
                    );
                }

                // Prefer the preferred mode, or the first available
                let mode = conn
                    .modes()
                    .iter()
                    .find(|m| {
                        m.mode_type()
                            .contains(drm::control::ModeTypeFlags::PREFERRED)
                    })
                    .or_else(|| conn.modes().first())
                    .copied();

                if let Some(mode) = mode {
                    selected_connector = Some(*conn_handle);
                    selected_mode = Some(mode);
                    break;
                }
            }
        }

        let connector = selected_connector.context("No connected display found")?;
        let mode = selected_mode.context("No display mode available")?;

        tracing::info!(
            "Selected mode: {}x{}@{}mHz",
            mode.size().0,
            mode.size().1,
            mode.vrefresh()
        );

        // Find a CRTC for this connector
        let conn_info = device.drm.get_connector(connector, false)?;
        let encoder = conn_info
            .current_encoder()
            .or_else(|| conn_info.encoders().first().copied())
            .context("Connector has no encoder")?;

        let encoder_info = device.drm.get_encoder(encoder)?;
        let crtc = encoder_info
            .crtc()
            .or_else(|| resources.crtcs().first().copied())
            .context("No CRTC available")?;

        tracing::info!("Using CRTC: {:?}, Encoder: {:?}", crtc, encoder);

        device.connector = Some(connector);
        device.crtc = Some(crtc);
        device.mode = Some(mode);

        Ok(DisplayInfo {
            width: u32::from(mode.size().0),
            height: u32::from(mode.size().1),
            refresh_mhz: mode.vrefresh(),
        })
    }

    /// Get the device state
    #[must_use]
    pub fn device(&self) -> Option<&DeviceState> {
        self.device.as_ref()
    }

    /// Get mutable device state
    pub fn device_mut(&mut self) -> Option<&mut DeviceState> {
        self.device.as_mut()
    }
}

impl DeviceState {
    /// Get display size
    #[must_use]
    pub fn display_size(&self) -> Option<Size<i32, Physical>> {
        self.mode
            .map(|m| Size::from((i32::from(m.size().0), i32::from(m.size().1))))
    }
}
