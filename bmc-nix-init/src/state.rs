// Copyright (C) 2026  Braiins Systems s.r.o.

/// States of the Nix store initialization process.
#[derive(Debug, Clone)]
pub enum InitState {
    /// No WiFi connection, entering AP mode
    NoWifi { ap_ssid: String },
    /// WiFi credentials submitted, connecting to network
    Connecting { ssid: String },
    /// WiFi connection failed, shown briefly before reverting to AP
    ConnectionFailed { ssid: String },
    /// Connected, fetching factory index
    FetchingIndex,
    /// Downloading tarball from factory server
    Downloading {
        downloaded_bytes: usize,
        total_bytes: Option<usize>,
    },
    /// Extracting tarball to Nix store
    Extracting,
    /// Activating the initial profile
    Activating,
    /// Upgrading BOS firmware (downloading image)
    UpgradingFirmware { downloaded_mb: f32, total_mb: f32 },
    /// BOS upgrade complete, device is rebooting
    Rebooting,
    /// Initialization complete
    Done,
    /// Error occurred, may retry
    Error {
        message: String,
        retryable: bool,
        /// Show "Reconfigure WiFi" button to re-enter AP mode
        reconfigurable: bool,
    },
}

/// Callback for state changes, used by UI layer.
pub trait InitStateObserver: Send + Sync {
    fn on_state_change(&self, state: &InitState);
}
