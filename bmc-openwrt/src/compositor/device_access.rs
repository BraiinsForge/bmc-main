// Copyright (C) 2026  Braiins Systems s.r.o.

//! Root-only device access for Smithay/libinput without `libseat`.
//!
//! The compositor process runs as `root` and owns `/dev/dri/*` and
//! `/dev/input/*` for the entire product lifetime. Instead of pulling in
//! `seatd`/`elogind` or Smithay's `backend_session_libseat` path, we give
//! libinput a direct open/close implementation that behaves like `open(2)`.
//!
//! Non-goals of this module:
//! - No session daemon, no VT broker, no pause/resume notifier.
//! - No multi-seat support beyond the configured seat name.
//! - No device permission handling — the caller is expected to be `root`.

use std::fs::OpenOptions;
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use input::LibinputInterface;

/// Default seat name used for udev/libinput classification.
pub const DEFAULT_SEAT_NAME: &str = "seat0";
/// Default DRM scanout/card node.
pub const DEFAULT_SCANOUT_NODE: &str = "/dev/dri/card1";
/// Default DRM render node.
pub const DEFAULT_RENDER_NODE: &str = "/dev/dri/renderD128";

/// Static configuration describing how the compositor should access devices.
///
/// All paths are optional — when `None`, the higher layer (udev discovery or
/// cold scan) is expected to pick sane defaults. Explicit overrides exist so
/// appliance images can pin known-good nodes without relying on discovery.
#[derive(Debug, Clone)]
pub struct DeviceAccessConfig {
    seat_name: String,
    scanout_node: Option<PathBuf>,
    render_node: Option<PathBuf>,
}

impl Default for DeviceAccessConfig {
    fn default() -> Self {
        Self {
            seat_name: DEFAULT_SEAT_NAME.to_owned(),
            scanout_node: None,
            render_node: None,
        }
    }
}

impl DeviceAccessConfig {
    /// Build a configuration with the given seat name.
    #[must_use]
    pub fn with_seat(mut self, seat: impl Into<String>) -> Self {
        self.seat_name = seat.into();
        self
    }

    /// Override the DRM scanout/card node path.
    #[must_use]
    pub fn with_scanout_node(mut self, path: impl Into<PathBuf>) -> Self {
        self.scanout_node = Some(path.into());
        self
    }

    /// Override the DRM render node path.
    #[must_use]
    pub fn with_render_node(mut self, path: impl Into<PathBuf>) -> Self {
        self.render_node = Some(path.into());
        self
    }

    #[must_use]
    pub fn seat_name(&self) -> &str {
        &self.seat_name
    }

    #[must_use]
    pub fn scanout_node(&self) -> Option<&Path> {
        self.scanout_node.as_deref()
    }

    #[must_use]
    pub fn render_node(&self) -> Option<&Path> {
        self.render_node.as_deref()
    }

    #[must_use]
    pub fn resolved_scanout_node(&self) -> &Path {
        self.scanout_node
            .as_deref()
            .unwrap_or_else(|| Path::new(DEFAULT_SCANOUT_NODE))
    }

    #[must_use]
    pub fn resolved_render_node(&self) -> &Path {
        self.render_node
            .as_deref()
            .unwrap_or_else(|| Path::new(DEFAULT_RENDER_NODE))
    }
}

/// Root-privileged implementation of [`LibinputInterface`].
///
/// `open_restricted` performs a plain `open(2)` with the flags libinput
/// requests; `close_restricted` drops the owned fd. There is no session
/// daemon between the compositor and the kernel.
#[derive(Debug, Default)]
pub struct RootLibinputInterface;

impl LibinputInterface for RootLibinputInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let read = (flags & libc::O_ACCMODE) != libc::O_WRONLY;
        let write = (flags & libc::O_ACCMODE) != libc::O_RDONLY;
        OpenOptions::new()
            .custom_flags(flags)
            .read(read)
            .write(write)
            .open(path)
            .map(OwnedFd::from)
            .map_err(|err| {
                let errno = err.raw_os_error().unwrap_or(libc::EIO);
                tracing::warn!("open_restricted({}) failed: {err}", path.display());
                -errno
            })
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        // Dropping the `OwnedFd` closes it. libinput never reuses the fd
        // after calling `close_restricted`, so no further cleanup is needed.
        drop(fd);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use input::LibinputInterface;

    use super::{
        DEFAULT_RENDER_NODE, DEFAULT_SCANOUT_NODE, DEFAULT_SEAT_NAME, DeviceAccessConfig,
        RootLibinputInterface,
    };

    #[test]
    fn default_config_uses_seat0() {
        let cfg = DeviceAccessConfig::default();
        assert_eq!(cfg.seat_name(), DEFAULT_SEAT_NAME);
        assert!(cfg.scanout_node().is_none());
        assert!(cfg.render_node().is_none());
        assert_eq!(cfg.resolved_scanout_node(), Path::new(DEFAULT_SCANOUT_NODE));
        assert_eq!(cfg.resolved_render_node(), Path::new(DEFAULT_RENDER_NODE));
    }

    #[test]
    fn overrides_are_preserved() {
        let cfg = DeviceAccessConfig::default()
            .with_seat("seat1")
            .with_scanout_node(PathBuf::from("/dev/dri/card0"))
            .with_render_node(PathBuf::from("/dev/dri/renderD128"));
        assert_eq!(cfg.seat_name(), "seat1");
        assert_eq!(
            cfg.scanout_node().map(std::path::Path::to_path_buf),
            Some(PathBuf::from("/dev/dri/card0"))
        );
        assert_eq!(
            cfg.render_node().map(std::path::Path::to_path_buf),
            Some(PathBuf::from("/dev/dri/renderD128"))
        );
    }

    #[test]
    fn open_restricted_reports_errno_for_missing_path() {
        let mut interface = RootLibinputInterface;
        let err = interface
            .open_restricted(
                std::path::Path::new("/nonexistent/bmc/device"),
                libc::O_RDONLY,
            )
            .expect_err("BUG: opening a nonexistent path must fail");
        assert!(err < 0, "errno must be reported as negative, got {err}");
        assert_eq!(-err, libc::ENOENT);
    }
}
