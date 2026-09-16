// Copyright (C) 2024  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

use anyhow::Result;
use async_trait::async_trait;
pub use bmc_net_types::wifi::WifiScanItem;
pub use bmc_net_types::wifi::{EncryptionType, SignalStrength, WifiMode, WifiStatus};
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::time::{Duration, Instant};

// One directory per WiFi driver; add a sibling module to plug in a new one.
// `esp32` is crate-private (reachable only via [`wifi_driver`]); `nl80211`
// stays public for callers that construct it directly.
mod esp32;
pub use esp32::AP_INTERFACE_NAME;
pub mod nl80211;
// Shared building blocks used by every driver.
mod supplicant;
mod uci;
pub mod utils;

pub type AsyncUpdate<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// WiFi chip a platform is wired with; [`wifi_driver`] maps it to a
/// [`WifiDriver`] backend so callers select a driver by chip, not by type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WifiChip {
    /// No WiFi hardware.
    None,
    /// ESP32 companion radio, driven through its init service.
    Esp32,
    /// nl80211/`mac80211` radio driven through UCI; carries the radio's sysfs path.
    Nl80211 { syspath: String },
}

/// Builds the [`WifiDriver`] for a [`WifiChip`], or `None` when the platform has
/// no WiFi.
pub async fn wifi_driver(chip: WifiChip) -> Result<Option<Arc<dyn WifiDriver>>> {
    Ok(match chip {
        WifiChip::None => None,
        WifiChip::Esp32 => {
            Some(Arc::new(esp32::Esp32WifiManager::new().await) as Arc<dyn WifiDriver>)
        }
        WifiChip::Nl80211 { syspath } => {
            Some(Arc::new(nl80211::OpenwrtWifiManager::new(&syspath)?) as Arc<dyn WifiDriver>)
        }
    })
}

/// Platform-independent WiFi station/AP control surface, implemented per
/// backend and selected via [`wifi_driver`].
#[async_trait]
pub trait WifiDriver: Debug + Send + Sync {
    /// Scans for visible access points, strongest signal first, flagging the
    /// one the station is currently connected to.
    async fn scan(&self) -> Result<Vec<WifiScanItem>>;
    /// Current status of the active WiFi interface.
    async fn status(&self) -> Result<WifiStatus>;
    /// Status of every configured WiFi interface.
    async fn status_all(&self) -> Result<Vec<WifiStatus>>;
    /// Saves station credentials and connects to the network.
    async fn save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()>;
    /// Brings up an access point.
    ///
    /// Applies the config itself, so callers must not follow it with
    /// [`enable_radio`]. The `esp32` backend hosts a fixed open setup AP and
    /// ignores `password`/`encryption`; `nl80211` honours them.
    ///
    /// Returns once the raise is *requested* - a `wifi reload` queued
    /// (`nl80211`) or `ifup wifi_ap` issued (`esp32`) - not once the AP is on
    /// air. Callers that need that follow it with [`wait_for_ap_active`].
    ///
    /// [`enable_radio`]: WifiDriver::enable_radio
    /// [`wait_for_ap_active`]: WifiDriver::wait_for_ap_active
    async fn configure_ap_mode(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()>;
    /// Tears down the access point brought up by [`configure_ap_mode`], leaving
    /// the radio idle. Idempotent: a no-op when no AP is active.
    ///
    /// [`configure_ap_mode`]: WifiDriver::configure_ap_mode
    async fn stop_ap(&self) -> Result<()>;
    /// Enables or disables the radio.
    async fn enable_radio(&self, enable: bool) -> Result<()>;
    /// Waits until the access point brought up by [`configure_ap_mode`] is
    /// actually up: beaconing (`nl80211`, via `iwinfo`) or reported up with
    /// its address by netifd (`esp32`). Fails when that does not happen within
    /// the backend's window, so a caller never announces an AP nobody can join.
    ///
    /// [`configure_ap_mode`]: WifiDriver::configure_ap_mode
    async fn wait_for_ap_active(&self) -> Result<()>;
    /// Resets the WiFi configuration to defaults.
    async fn reset_config(&self) -> Result<()>;
    /// SSID currently advertised in AP mode, if any.
    async fn ap_ssid(&self) -> Option<String>;
    /// SSID currently joined in station mode, if any.
    async fn sta_ssid(&self) -> Option<String>;
    /// Name of the underlying WiFi device (e.g. "wlan0").
    async fn wifi_device_name(&self) -> Result<String>;
}

/// Short-lived cache shared by the drivers to throttle repeated WiFi queries.
#[derive(Default)]
pub(crate) struct SharedCache<T> {
    timeout: Duration,
    value_with_timestamp: Option<(T, Instant)>,
}

impl<T> SharedCache<T>
where
    T: Debug + Clone,
{
    pub(crate) fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            value_with_timestamp: None,
        }
    }

    pub async fn cached_or_else<E>(&mut self, update: AsyncUpdate<Result<T, E>>) -> Result<T, E> {
        if let Some(value) = self
            .value_with_timestamp
            .as_ref()
            .filter(|(_, timestamp)| timestamp.elapsed() < self.timeout)
            .map(|(value, _)| value.clone())
        {
            return Ok(value);
        }

        let value = update.await?;
        self.value_with_timestamp = Some((value.clone(), Instant::now()));
        Ok(value)
    }
}
