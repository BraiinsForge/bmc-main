// Copyright (C) 2026  Braiins Systems s.r.o.

use slint::{ComponentHandle as _, Global as _};

use crate::InitWindow;
use crate::WifiAdapter;
use crate::state::{InitState, InitStateObserver};
use crate::utils::{AP_IP, ip_as_qrcode};

/// Slint-based display observer for init progress.
///
/// Updates the `InitWindow` properties from any thread via
/// `slint::invoke_from_event_loop`. Safe to call from the tokio
/// runtime thread.
#[expect(missing_debug_implementations)]
pub struct SlintObserver {
    window: slint::Weak<InitWindow>,
}

impl SlintObserver {
    #[must_use]
    pub fn new(window: &InitWindow) -> Self {
        Self {
            window: window.as_weak(),
        }
    }
}

impl InitStateObserver for SlintObserver {
    fn on_state_change(&self, state: &InitState) {
        let state = state.clone();
        let weak = self.window.clone();
        slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            // Reset all state properties so stale values from a previous
            // Error state do not leak through after retry/reconfigure.
            window.set_show_wifi_setup(false);
            window.set_show_wifi_subtitle(false);
            window.set_detail_text(slint::SharedString::default());
            window.set_show_retry_button(false);
            window.set_show_reconfigure_button(false);

            match &state {
                InitState::NoWifi { ap_ssid } => {
                    let wifi_adapter = WifiAdapter::get(&window);
                    wifi_adapter.set_ssid(ap_ssid.into());
                    wifi_adapter.set_ap_ip(AP_IP.to_string().into());
                    wifi_adapter.set_ap_ip_qr_code(ip_as_qrcode(Some(AP_IP)));
                    window.set_show_wifi_setup(true);
                    window.set_show_progress(false);
                }
                InitState::Connecting { ssid } => {
                    let wifi_adapter = WifiAdapter::get(&window);
                    wifi_adapter.set_ssid(ssid.into());
                    window.set_status_text("Connecting...".into());
                    window.set_show_progress(false);
                    window.set_show_wifi_subtitle(true);
                }
                InitState::ConnectionFailed { ssid } => {
                    let wifi_adapter = WifiAdapter::get(&window);
                    wifi_adapter.set_ssid(ssid.into());
                    window.set_status_text("Connection failed".into());
                    window.set_show_progress(false);
                    window.set_show_wifi_subtitle(true);
                }
                InitState::FetchingIndex => {
                    window.set_status_text("Checking for updates...".into());
                    window.set_show_progress(false);
                }
                InitState::Downloading {
                    downloaded_bytes,
                    total_bytes,
                } => {
                    #[expect(clippy::cast_precision_loss)]
                    let mb = *downloaded_bytes as f64 / 1_048_576.0;
                    if let Some(total) = total_bytes {
                        #[expect(clippy::cast_precision_loss)]
                        let total_mb = *total as f64 / 1_048_576.0;
                        window.set_status_text(
                            format!("Downloading: {mb:.1} / {total_mb:.1} MB").into(),
                        );
                        #[expect(clippy::cast_precision_loss)]
                        window.set_progress(*downloaded_bytes as f32 / *total as f32);
                    } else {
                        window.set_status_text(format!("Downloading: {mb:.1} MB").into());
                    }
                    window.set_show_progress(true);
                }
                InitState::Extracting => {
                    window.set_status_text("Installing packages...".into());
                    window.set_show_progress(false);
                }
                InitState::Activating => {
                    window.set_status_text("Finalizing...".into());
                    window.set_show_progress(false);
                }
                InitState::UpgradingFirmware {
                    downloaded_mb,
                    total_mb,
                } => {
                    window.set_status_text(
                        format!("Upgrading firmware: {downloaded_mb:.1} / {total_mb:.1} MB").into(),
                    );
                    if *total_mb > 0.0 {
                        window.set_progress(downloaded_mb / total_mb);
                    }
                    window.set_show_progress(true);
                }
                InitState::Rebooting => {
                    window.set_status_text("Restarting...".into());
                    window.set_show_progress(false);
                }
                InitState::Done => {
                    window.set_status_text("Ready!".into());
                    window.set_show_progress(false);
                }
                InitState::Error {
                    message,
                    retryable,
                    reconfigurable,
                } => {
                    window.set_status_text("Error".into());
                    window.set_detail_text(message.clone().into());
                    window.set_show_progress(false);
                    window.set_show_retry_button(*retryable);
                    window.set_show_reconfigure_button(*reconfigurable);
                }
            }
        })
        .expect("BUG: event loop should be running");
    }
}
