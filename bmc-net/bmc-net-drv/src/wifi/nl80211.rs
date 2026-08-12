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
use async_trait::async_trait;
use bmc_net_types::wifi::{EncryptionType, WifiMode, WifiScanItem, WifiStatus};
use log::{debug, warn};
use scanner::WifiScanner;
use serde::Deserialize;
use serde_json::json;
use sta::WifiSta;
use std::fmt::Debug;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration, MissedTickBehavior};
use wl_nl80211::Nl80211Handle;

use super::uci::{HtMode, UciHelper, map_uci_iface_to_wifi_status};
use super::utils::{
    ATTEMPTS_TO_GET_IP, CommandUtils, WifiCommand, WifiUtils, filter_empty_ssid,
    filter_unsupported_enc, mark_connected, wait_for_network_ip_address, wait_for_wireless_config,
};
use super::{SharedCache, WifiDriver};
use crate::WIRELESS_CONFIG_FILE_PATH;

mod scanner;
mod sta;

const WIFI_AP_CHANNEL: u32 = 1;
// Default beaconing interval in hostapd is 100, but that's too short which leads
// to clients being disconnected during wifi scanning in APSTA mode.
const WIFI_AP_BEACON_INTERVAL: u32 = 500;

pub struct OpenwrtWifiManager {
    scan_result_list: Mutex<SharedCache<Vec<WifiScanItem>>>,
    wifi_status_cache: Mutex<SharedCache<Vec<WifiStatus>>>,
    wlan_dev_syspath: String,
    nl80211_handle: Nl80211Handle,
    nl80211_task_handle: JoinHandle<()>,
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

    pub fn new(wlan_dev_syspath: &str) -> anyhow::Result<Self> {
        let (connection, nl80211_handle, _) = wl_nl80211::new_connection()?;
        let nl80211_task_handle = tokio::spawn(connection);

        Ok(Self {
            scan_result_list: Mutex::new(SharedCache::new(Self::WIFI_INTERACTION_DELAY)),
            wifi_status_cache: Mutex::new(SharedCache::new(Self::WIFI_INTERACTION_DELAY)),
            wlan_dev_syspath: wlan_dev_syspath.to_owned(),
            nl80211_handle,
            nl80211_task_handle,
        })
    }

    pub async fn get_phy_macaddress(&self) -> anyhow::Result<String> {
        let phy = WifiUtils::get_phy_path_by_syspath(&self.wlan_dev_syspath).await?;
        let mac = tokio::fs::read_to_string(phy.join("macaddress"))
            .await
            .map_err(|e| {
                anyhow!(
                    "Could not obtain phy's macaddress at {}/macaddress: {e}",
                    phy.display()
                )
            })?
            .trim()
            .to_owned();

        Ok(mac)
    }

    async fn get_wifi_filtered_scan_list(device: &str) -> Result<Vec<WifiScanItem>> {
        Ok(WifiScanner::wifi_scan(device)
            .await?
            .into_iter()
            .filter(filter_unsupported_enc)
            .filter(filter_empty_ssid)
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

    async fn configure_radio_for_ap(&self) -> Result<(), anyhow::Error> {
        let uci = UciHelper::new(&self.wlan_dev_syspath);
        uci.wifi_radio_configure_ap_channel(WIFI_AP_CHANNEL).await?;
        uci.wifi_radio_configure_beacon_int(WIFI_AP_BEACON_INTERVAL)
            .await?;
        uci.wifi_radio_configure_ht_mode(HtMode::NoHt).await?;
        uci.save_changes().await
    }

    async fn get_status_all(
        nl80211_handle: Nl80211Handle,
        wlan_dev_syspath: String,
    ) -> Result<Vec<WifiStatus>> {
        let device = WifiUtils::get_device_by_syspath(&wlan_dev_syspath).await?;
        let uci = UciHelper::new(&wlan_dev_syspath);
        let sta_link_state = WifiSta::link_details(nl80211_handle, &device)
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
}

#[async_trait]
impl WifiDriver for OpenwrtWifiManager {
    async fn ap_ssid(&self) -> Option<String> {
        let uci = UciHelper::new(&self.wlan_dev_syspath);
        let config = uci.wifi_iface_find_enabled().await?;

        if config.mode == WifiMode::Ap {
            Some(config.ssid)
        } else {
            None
        }
    }

    async fn sta_ssid(&self) -> Option<String> {
        let uci = UciHelper::new(&self.wlan_dev_syspath);
        let config = uci.wifi_iface_find_enabled().await?;

        if config.mode == WifiMode::Station {
            Some(config.ssid)
        } else {
            None
        }
    }

    async fn wifi_device_name(&self) -> Result<String, anyhow::Error> {
        WifiUtils::get_device_by_syspath(&self.wlan_dev_syspath).await
    }

    async fn enable_radio(&self, enable: bool) -> Result<()> {
        let uci = UciHelper::new(&self.wlan_dev_syspath);
        uci.wifi_radio_enable(enable).await?;
        uci.save_changes().await?;
        WifiCommand::reload().await
    }

    async fn wait_for_ap_active(&self) -> Result<()> {
        let device = self.wifi_device_name().await?;
        wait_for_ap_active(&device, ATTEMPTS_TO_ACTIVATE_AP).await
    }

    async fn configure_ap_mode(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()> {
        self.configure_radio_for_ap().await?;
        self.configure_wifi_iface(WifiMode::Ap, ssid, password, encryption)
            .await
    }

    async fn stop_ap(&self) -> Result<()> {
        // Disable only AP-mode sections: a station enabled by `save_and_connect`
        // must survive tearing down the setup AP, otherwise stopping the AP
        // right after a successful reconfiguration drops connectivity.
        let uci = UciHelper::new(&self.wlan_dev_syspath);
        uci.wifi_iface_disable_by_mode(WifiMode::Ap).await?;
        uci.save_changes().await?;
        WifiCommand::reload().await
    }

    async fn save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()> {
        let device = WifiUtils::get_device_by_syspath(&self.wlan_dev_syspath).await?;
        self.configure_wifi_iface(WifiMode::Station, ssid, password, encryption)
            .await?;
        self.enable_radio(true).await?;

        wait_for_network_ip_address(&device, ATTEMPTS_TO_GET_IP).await
    }

    async fn scan(&self) -> Result<Vec<WifiScanItem>> {
        let device = WifiUtils::get_device_by_syspath(&self.wlan_dev_syspath).await?;
        let mut items = self
            .scan_result_list
            .lock()
            .await
            .cached_or_else(Box::pin(async move {
                Self::get_wifi_filtered_scan_list(&device).await
            }))
            .await?;
        mark_connected(&mut items, self.sta_ssid().await);
        Ok(items)
    }

    async fn status(&self) -> Result<WifiStatus> {
        self.status_all()
            .await?
            .into_iter()
            .find(|s| s.enabled)
            .ok_or_else(|| {
                anyhow!("No enabled WiFi interface found. Please check your configuration.")
            })
    }

    async fn status_all(&self) -> Result<Vec<WifiStatus>> {
        let wlan_dev_syspath = self.wlan_dev_syspath.clone();
        let nl80211_handle = self.nl80211_handle.clone();
        self.wifi_status_cache
            .lock()
            .await
            .cached_or_else::<anyhow::Error>(Box::pin(async move {
                Self::get_status_all(nl80211_handle, wlan_dev_syspath).await
            }))
            .await
    }

    async fn reset_config(&self) -> Result<()> {
        debug!("Removing wireless config");
        match tokio::fs::remove_file(WIRELESS_CONFIG_FILE_PATH).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("File {WIRELESS_CONFIG_FILE_PATH} not found");
            }
            Err(e) => bail!("Unable to remove wireless config: {e}"),
        }
        WifiCommand::config().await?;
        wait_for_wireless_config().await
    }
}

/// Mode string `iwinfo` reports for an interface beaconing as an access point;
/// the same spelling the scan filter matches on.
const IWINFO_AP_MODE: &str = "Master";

const AP_ACTIVE_WAIT_INTERVAL: Duration = Duration::from_secs(1);

const ATTEMPTS_TO_ACTIVATE_AP: u8 = 20;

#[derive(Deserialize)]
struct IwinfoInfo {
    mode: String,
}

/// Polls `iwinfo` until `device` reports it is beaconing as an access point.
///
/// Individual query failures are retried rather than propagated: right after a
/// reload the interface is legitimately absent or mid-reconfiguration, so one
/// failed call says nothing about the eventual outcome.
///
/// Exhausting every attempt only fails when `iwinfo` actually answered at some
/// point and simply never reported AP mode. If it never answered at all the
/// probe itself is unavailable, which is a gap in our diagnostics rather than
/// evidence the AP is down, and bringing setup mode up must not hinge on it.
async fn wait_for_ap_active(device: &str, attempts: u8) -> Result<()> {
    let mut interval = time::interval(AP_ACTIVE_WAIT_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let ubus_param = json!({ "device": device }).to_string();
    let mut mode_observed = false;

    for i in 0..attempts {
        interval.tick().await;
        debug!("Waiting for AP on {device} to broadcast. Attempt {i}/{attempts}");

        match CommandUtils::call_ubus_cmd(&["call", "iwinfo", "info", &ubus_param]).await {
            Ok(output) => match serde_json::from_str::<IwinfoInfo>(&output) {
                Ok(info) if info.mode == IWINFO_AP_MODE => return Ok(()),
                Ok(info) => {
                    mode_observed = true;
                    debug!("{device} is in mode {}, waiting for AP", info.mode);
                }
                Err(e) => debug!("Unable to parse iwinfo info for {device}: {e}"),
            },
            Err(e) => debug!("Unable to query iwinfo info for {device}: {e}"),
        }
    }

    if mode_observed {
        bail!("Access point on {device} did not start broadcasting")
    }

    warn!("Unable to confirm the access point on {device} is broadcasting");
    Ok(())
}

impl Drop for OpenwrtWifiManager {
    fn drop(&mut self) {
        self.nl80211_task_handle.abort();
    }
}
