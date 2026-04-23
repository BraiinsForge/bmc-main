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

use bmc_platform::linux_input::discover_touch_node;
use input::{Libinput, LibinputInterface};

/// Default seat name used for libinput classification.
///
/// Only meaningful if the target image runs `udevd` and the compositor
/// ever moves off path-based libinput back to `Libinput::new_with_udev`.
/// The current default OpenWrt image uses `mdev` and does not tag devices
/// with `ID_SEAT`, so the seat name is informational on this appliance.
pub const DEFAULT_SEAT_NAME: &str = "seat0";
/// Default DRM scanout/card node.
pub const DEFAULT_SCANOUT_NODE: &str = "/dev/dri/card1";
/// Default DRM render node.
pub const DEFAULT_RENDER_NODE: &str = "/dev/dri/renderD128";

/// Static configuration describing how the compositor should access devices.
///
/// All paths are optional — when `None`, each `resolved_*_node` accessor
/// falls back to the appliance default. Explicit overrides exist so other
/// images can pin known-good nodes without relying on auto-discovery, which
/// in turn is not available on the default OpenWrt image because it runs
/// `mdev` instead of `udevd`.
#[derive(Debug, Clone)]
pub struct DeviceAccessConfig {
    seat_name: String,
    scanout_node: Option<PathBuf>,
    render_node: Option<PathBuf>,
    input_nodes: Vec<PathBuf>,
}

impl Default for DeviceAccessConfig {
    fn default() -> Self {
        Self {
            seat_name: DEFAULT_SEAT_NAME.to_owned(),
            scanout_node: None,
            render_node: None,
            input_nodes: Vec::new(),
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

    /// Append an evdev input device node that libinput should watch.
    ///
    /// Intended for path-based libinput contexts on images without udev;
    /// callers can invoke this multiple times to register more than one
    /// input device.
    #[must_use]
    pub fn with_input_node(mut self, path: impl Into<PathBuf>) -> Self {
        self.input_nodes.push(path.into());
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

    /// Evdev nodes libinput should watch.
    ///
    /// Explicit overrides from [`Self::with_input_node`] take precedence
    /// and are returned verbatim; otherwise the compositor consults
    /// [`bmc_platform::linux_input::discover_touch_node`] and registers
    /// the single canonical touchscreen (or nothing, when none is
    /// present). Returning `Vec<PathBuf>` instead of `Option<PathBuf>`
    /// keeps the explicit-override surface unchanged — callers can still
    /// pin multiple nodes for test rigs and unusual images.
    #[must_use]
    pub fn resolved_input_nodes(&self) -> Vec<PathBuf> {
        if self.input_nodes.is_empty() {
            discover_touch_node().into_iter().collect()
        } else {
            self.input_nodes.clone()
        }
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
        // Defense in depth: libinput's caller-supplied path is opened as
        // root, so resolve symlinks first and then refuse anything that
        // isn't an evdev node under `/dev/input/`. Every current caller
        // feeds `DeviceAccessConfig::with_input_node` a trusted value
        // (discovery result or a static override), but this validation
        // is the tripwire for future code paths that may expose the
        // input-node setting to user input.
        let canonical = path.canonicalize().map_err(|err| {
            let errno = err.raw_os_error().unwrap_or(libc::EIO);
            tracing::warn!(
                "open_restricted: canonicalize({}) failed: {err}",
                path.display(),
            );
            -errno
        })?;
        if !is_valid_input_node(&canonical) {
            tracing::warn!(
                "open_restricted: refusing {} (resolved to {}); only /dev/input/eventN is allowed",
                path.display(),
                canonical.display(),
            );
            return Err(-libc::EACCES);
        }

        let read = (flags & libc::O_ACCMODE) != libc::O_WRONLY;
        let write = (flags & libc::O_ACCMODE) != libc::O_RDONLY;
        OpenOptions::new()
            .custom_flags(flags)
            .read(read)
            .write(write)
            .open(&canonical)
            .map(OwnedFd::from)
            .map_err(|err| {
                let errno = err.raw_os_error().unwrap_or(libc::EIO);
                tracing::warn!("open_restricted({}) failed: {err}", canonical.display());
                -errno
            })
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        // Dropping the `OwnedFd` closes it. libinput never reuses the fd
        // after calling `close_restricted`, so no further cleanup is needed.
        drop(fd);
    }
}

/// Returns `true` only for absolute `/dev/input/eventN` paths where `N`
/// is a non-empty ASCII decimal integer. Callers are expected to resolve
/// symlinks before checking.
fn is_valid_input_node(path: &Path) -> bool {
    path.to_str()
        .and_then(|s| s.strip_prefix("/dev/input/event"))
        .is_some_and(|tail| !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()))
}

/// Raise libinput's internal log verbosity to DEBUG so hotplug and
/// device-tag diagnostics appear alongside our own `tracing` output.
///
/// The `input` crate does not wrap `libinput_log_set_priority`, so the
/// FFI declaration lives here rather than at the call site. Libinput's
/// default log handler writes to stderr, which our `bmc.log` wrapper
/// tees into the unified log, so no custom handler is needed.
///
/// `LIBINPUT_LOG_PRIORITY_DEBUG = 10` per libinput's public enum.
pub(crate) fn set_libinput_debug_priority(ctx: &Libinput) {
    use input::AsRaw;
    unsafe extern "C" {
        fn libinput_log_set_priority(libinput: *mut core::ffi::c_void, priority: core::ffi::c_uint);
    }
    const LIBINPUT_LOG_PRIORITY_DEBUG: core::ffi::c_uint = 10;
    unsafe {
        libinput_log_set_priority(ctx.as_raw().cast_mut().cast(), LIBINPUT_LOG_PRIORITY_DEBUG);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use input::LibinputInterface;

    use super::{
        DEFAULT_RENDER_NODE, DEFAULT_SCANOUT_NODE, DEFAULT_SEAT_NAME, DeviceAccessConfig,
        RootLibinputInterface, is_valid_input_node,
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
            .with_render_node(PathBuf::from("/dev/dri/renderD128"))
            .with_input_node(PathBuf::from("/dev/input/event5"))
            .with_input_node(PathBuf::from("/dev/input/event6"));
        assert_eq!(cfg.seat_name(), "seat1");
        assert_eq!(
            cfg.scanout_node().map(std::path::Path::to_path_buf),
            Some(PathBuf::from("/dev/dri/card0"))
        );
        assert_eq!(
            cfg.render_node().map(std::path::Path::to_path_buf),
            Some(PathBuf::from("/dev/dri/renderD128"))
        );
        assert_eq!(
            cfg.resolved_input_nodes(),
            vec![
                PathBuf::from("/dev/input/event5"),
                PathBuf::from("/dev/input/event6"),
            ]
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

    #[test]
    fn open_restricted_rejects_paths_outside_dev_input() {
        // `/dev/null` exists, so canonicalize succeeds — the rejection
        // must come from the `/dev/input/eventN` check.
        let mut interface = RootLibinputInterface;
        let err = interface
            .open_restricted(Path::new("/dev/null"), libc::O_RDONLY)
            .expect_err("BUG: /dev/null must be rejected as non-evdev");
        assert_eq!(-err, libc::EACCES);
    }

    #[test]
    fn is_valid_input_node_accepts_event_paths() {
        assert!(is_valid_input_node(Path::new("/dev/input/event0")));
        assert!(is_valid_input_node(Path::new("/dev/input/event123")));
    }

    #[test]
    fn is_valid_input_node_rejects_non_event_paths() {
        assert!(!is_valid_input_node(Path::new("/dev/input/mouse0")));
        assert!(!is_valid_input_node(Path::new("/etc/shadow")));
        assert!(!is_valid_input_node(Path::new("/dev/input/event")));
        assert!(!is_valid_input_node(Path::new("/dev/input/event0a")));
        assert!(!is_valid_input_node(Path::new("/dev/null")));
    }
}
