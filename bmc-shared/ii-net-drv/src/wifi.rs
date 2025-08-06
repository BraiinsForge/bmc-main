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

use anyhow::{Result, anyhow, bail};
pub use ii_net::wifi::WifiScanItem;
pub use ii_net::wifi::{EncryptionType, SignalStrength, WifiMode, WifiStatus};
use log::debug;
use scanner::WifiScanner;
use sta::WifiSta;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tokio::time::{self, Instant, MissedTickBehavior};
use uci::UciHelper;
use utils::{WifiCommand, WifiUtils};

use crate::wifi::uci::map_uci_iface_to_wifi_status;
use crate::{NetworkInterface, WIRELESS_CONFIG_FILE_PATH};

mod scanner;
mod sta;
mod uci;
pub mod utils;

const ATTEMPTS_TO_GET_IP: u8 = 30;
const IP_CHECK_INTERVAL: Duration = Duration::from_secs(1);

const WIRELESS_CONFIG_WAIT_INTERVAL: Duration = Duration::from_secs(1);
const WIRELESS_CONFIG_GET_ATTEMPTS: u8 = 20;
const WIRELESS_CONFIG_MIN_SIZE: u64 = 300;

pub type AsyncUpdate<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

#[derive(Default)]
struct SharedCache<T> {
    timeout: Duration,
    value_with_timestamp: Option<(T, Instant)>,
}

impl<T> SharedCache<T>
where
    T: Debug + Clone,
{
    fn new(timeout: Duration) -> Self {
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

pub struct OpenwrtWifiManager {
    scan_result_list: Mutex<SharedCache<Vec<WifiScanItem>>>,
    wifi_status_cache: Mutex<SharedCache<Vec<WifiStatus>>>,
    wlan_dev_syspath: String,
}

#[expect(clippy::missing_fields_in_debug)]
impl Debug for OpenwrtWifiManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenwrtWifiManager")
            .field("wlan_dev_syspath", &self.wlan_dev_syspath)
            .finish()
    }
}

impl OpenwrtWifiManager {
    const WIFI_INTERACTION_DELAY: Duration = Duration::from_secs(5);

    #[must_use]
    pub fn new(wlan_dev_syspath: &str) -> Self {
        Self {
            scan_result_list: Mutex::new(SharedCache::new(Self::WIFI_INTERACTION_DELAY)),
            wifi_status_cache: Mutex::new(SharedCache::new(Self::WIFI_INTERACTION_DELAY)),
            wlan_dev_syspath: wlan_dev_syspath.to_owned(),
        }
    }

    async fn get_wifi_filtered_scan_list(
        device: &str,
        unsupported_enc: Vec<EncryptionType>,
    ) -> Result<Vec<WifiScanItem>> {
        Ok(WifiScanner::wifi_scan(device)
            .await?
            .into_iter()
            .filter(|result| !unsupported_enc.contains(&result.encryption_type))
            .collect())
    }

    async fn configure_wifi_iface(
        &self,
        mode: WifiMode,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<(), anyhow::Error> {
        let uci = UciHelper::new(&self.wlan_dev_syspath);
        uci.wifi_iface_disable_all().await?;
        uci.wifi_iface_configure(mode, ssid, encryption, password.unwrap_or_default())
            .await?;

        uci.save_changes().await
    }

    async fn wait_for_network_ip_address(&self, device: &str, attempts: u8) -> Result<()> {
        debug!("Wifi connected, waiting for IP address...");
        let mut interval = time::interval(IP_CHECK_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        for i in 0..attempts {
            debug!("{i}/{attempts} attempt to get IP address from {device}");
            interval.tick().await;
            if let Some(ip) =
                NetworkInterface::get_by_substr(device).and_then(|network| network.ipv4_address())
            {
                debug!("IP is assigned: {ip}, connection is complete");
                return Ok(());
            }
        }
        Err(anyhow!("IP cannot be assigned. Failed to setup wifi"))
    }

    pub async fn get_wifi_device_name(&self) -> Result<String, anyhow::Error> {
        WifiUtils::get_device_by_syspath(&self.wlan_dev_syspath).await
    }

    pub async fn enable_radio(&self, enable: bool) -> Result<()> {
        let uci = UciHelper::new(&self.wlan_dev_syspath);
        uci.wifi_radio_enable(enable).await?;
        uci.save_changes().await
    }

    async fn wifi_config_exists(&self) -> Result<()> {
        let mut interval = time::interval(WIRELESS_CONFIG_WAIT_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        for i in 0..WIRELESS_CONFIG_GET_ATTEMPTS {
            interval.tick().await;
            debug!(
                "Checking if wireless config exists. Attempt {i}/{WIRELESS_CONFIG_GET_ATTEMPTS}"
            );
            // Hack: We check the size of /etc/config/wireless, which should be always "at least" 300 bytes
            // since it uses default OpenWRT configuration
            // This is necessary workaround so we don't open/load the file via UCI library before it's fully created
            if let Ok(metadata) = tokio::fs::metadata(WIRELESS_CONFIG_FILE_PATH).await {
                if metadata.len() >= WIRELESS_CONFIG_MIN_SIZE {
                    return Ok(());
                }
            }
        }

        bail!("Wi-Fi config is not present")
    }

    #[allow(dead_code)]
    async fn get_status(wlan_dev_syspath: String) -> Result<WifiStatus> {
        let device = WifiUtils::get_device_by_syspath(&wlan_dev_syspath).await?;
        let uci = UciHelper::new(&wlan_dev_syspath);

        Ok(WifiStatus {
            enabled: uci.wifi_enabled().await?,
            configuration: uci.wifi_iface_find_enabled().await,
            sta_link_state: WifiSta::link_details(&device)
                .await
                .inspect_err(|e| debug!("Unable to get WiFi STA link details: {e}"))
                .ok(),
        })
    }

    async fn get_status_all(wlan_dev_syspath: String) -> Result<Vec<WifiStatus>> {
        let device = WifiUtils::get_device_by_syspath(&wlan_dev_syspath).await?;
        let uci = UciHelper::new(&wlan_dev_syspath);
        let sta_link_state = WifiSta::link_details(&device)
            .await
            .inspect_err(|e| debug!("Unable to get WiFi STA link details: {e}"))
            .ok();
        let wifi_ifaces = uci.get_all_wifi_ifaces().await?;
        let wifi_statuses = wifi_ifaces
            .into_iter()
            .map(|iface| map_uci_iface_to_wifi_status(iface, sta_link_state.clone()))
            .collect();

        Ok(wifi_statuses)
    }

    pub async fn configure_ap_mode(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()> {
        self.configure_wifi_iface(WifiMode::Ap, ssid, password, encryption)
            .await
    }

    pub async fn save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()> {
        let device = WifiUtils::get_device_by_syspath(&self.wlan_dev_syspath).await?;
        self.configure_wifi_iface(WifiMode::Station, ssid, password, encryption)
            .await?;
        self.enable_radio(true).await?;
        WifiCommand::reload().await?;

        self.wait_for_network_ip_address(&device, ATTEMPTS_TO_GET_IP)
            .await
    }

    pub async fn enable(&self, enable: bool) -> Result<()> {
        self.enable_radio(enable).await?;
        WifiCommand::reload().await
    }

    pub async fn reload(&self) -> Result<()> {
        WifiCommand::reload().await
    }

    pub async fn scan(&self) -> Result<Vec<WifiScanItem>> {
        let device = WifiUtils::get_device_by_syspath(&self.wlan_dev_syspath).await?;
        let unsupported_enc = vec![EncryptionType::Wpa3];

        self.scan_result_list
            .lock()
            .await
            .cached_or_else(Box::pin(async move {
                Self::get_wifi_filtered_scan_list(&device, unsupported_enc).await
            }))
            .await
    }

    pub async fn status(&self) -> Result<WifiStatus> {
        self.status_all()
            .await?
            .into_iter()
            .find(|s| s.enabled)
            .ok_or_else(|| {
                anyhow!("No enabled WiFi interface found. Please check your configuration.")
            })
    }

    pub async fn status_all(&self) -> Result<Vec<WifiStatus>> {
        let wlan_dev_syspath = self.wlan_dev_syspath.clone();
        self.wifi_status_cache
            .lock()
            .await
            .cached_or_else::<anyhow::Error>(Box::pin(async move {
                Self::get_status_all(wlan_dev_syspath).await
            }))
            .await
    }

    pub async fn reset_config(&self) -> Result<()> {
        debug!("Removing wireless config");
        tokio::fs::remove_file(WIRELESS_CONFIG_FILE_PATH).await?;
        WifiCommand::config().await?;
        self.wifi_config_exists().await
    }
}
