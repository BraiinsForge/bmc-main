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
use bmc_net_types::wifi::{EncryptionType, WifiLinkState, WifiMode, WifiScanItem, WifiStatus};
use bstr::ByteSlice;
use log::{debug, info};
use std::fmt::Debug;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::Duration;

use super::uci::{UciHelper, map_uci_iface_to_wifi_status};
use super::utils::{
    ATTEMPTS_TO_GET_IP, CommandUtils, WifiCommand, WifiUtils, filter_empty_ssid,
    filter_sort_by_strongest_signal, filter_unsupported_enc, mark_connected,
    wait_for_network_ip_address, wait_for_wireless_config,
};
use super::{SharedCache, WifiDriver};
use crate::{NetworkInterface, WIRELESS_CONFIG_FILE_PATH};

mod scanner;
mod sdio;

/// Interface brought up by the ESP32 while it serves the setup access point.
///
/// Public because consumers (boser) look the AP address up by this name; it
/// must not be re-declared as a literal anywhere else.
pub const AP_INTERFACE_NAME: &str = "ethap0";
/// SSID prefix the ESP32 "NG" firmware bakes into its setup AP (the driver
/// cannot set it).
const AP_SSID_PREFIX: &str = "Mini Miner Setup";
const ESP32_SERVICE: &str = "/etc/init.d/esp32-init";
/// Platform helpers that own the setup AP: `start_wifi_ap` sets the softAP MAC,
/// starts the softAP under the branded SSID and brings up the `wifi_ap` network
/// (which is what creates [`AP_INTERFACE_NAME`] and gives it the setup address).
const ESP32_WIFI_LIB: &str = "/lib/functions/esp32-wifi.sh";
/// Provides `default_ssid`, the branded name the platform advertises.
const BOS_DEFAULTS_LIB: &str = "/lib/functions/bos-defaults.sh";
/// Hosted-control node exposed only by the ESP32 "FG" firmware. `esp32-sdio-cli`
/// talks to the module through it, so without it there is no setup AP: the "NG"
/// firmware a provisioned board runs presents a plain station instead.
const ESP_CONTROL_NODE: &str = "/dev/esps0";
const FACTORY_DEFAULT_WIFI_SERVICE: &str = "/etc/init.d/factory-default-wifi";

const WIFI_INTERACTION_DELAY: Duration = Duration::from_secs(5);

/// WiFi driver for boards using the `iwlist`/UCI station stack with an ESP32-SDIO
/// module for the setup access point (mini-miner / display class hardware).
pub struct Esp32WifiManager {
    /// `None` until the wireless interface enumerates; rediscovered lazily by
    /// [`Self::wlan_dev_syspath`] because the ESP32 may come up after boot.
    wlan_dev_syspath: Mutex<Option<String>>,
    scan_cache: Mutex<SharedCache<Vec<WifiScanItem>>>,
    status_cache: Mutex<SharedCache<Vec<WifiStatus>>>,
}

#[expect(clippy::missing_fields_in_debug)]
impl Debug for Esp32WifiManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32WifiManager")
            .field("wlan_dev_syspath", &self.wlan_dev_syspath)
            .finish()
    }
}

impl Esp32WifiManager {
    /// Discovers the wireless device syspath so callers need not supply one.
    pub async fn new() -> Self {
        Self {
            wlan_dev_syspath: Mutex::new(discover_wlan_syspath().await),
            scan_cache: Mutex::new(SharedCache::new(WIFI_INTERACTION_DELAY)),
            status_cache: Mutex::new(SharedCache::new(WIFI_INTERACTION_DELAY)),
        }
    }

    /// Returns the cached wireless device syspath, retrying discovery when the
    /// interface was not present yet at construction time.
    async fn wlan_dev_syspath(&self) -> Result<String> {
        let mut syspath = self.wlan_dev_syspath.lock().await;
        if syspath.is_none() {
            *syspath = discover_wlan_syspath().await;
        }
        syspath
            .clone()
            .ok_or_else(|| anyhow!("No wireless interface found"))
    }

    async fn uci(&self) -> Result<UciHelper> {
        Ok(UciHelper::new(&self.wlan_dev_syspath().await?))
    }

    async fn get_device(&self) -> Result<String> {
        WifiUtils::get_device_by_syspath(&self.wlan_dev_syspath().await?).await
    }

    /// The ESP32 setup AP exposes its own bridged interface; its presence marks
    /// AP mode, otherwise the station stack is active.
    ///
    /// The lookup is a `getifaddrs(3)` walk that can stall while the kernel
    /// holds the rtnl lock, so it runs on the blocking pool.
    async fn is_ap_mode() -> bool {
        tokio::task::spawn_blocking(|| NetworkInterface::get_by_name(AP_INTERFACE_NAME).is_some())
            .await
            .unwrap_or(false)
    }
}

/// Resolve the sysfs device path of the wireless interface (the directory holding
/// `net/` and `ieee80211/`) so the shared UCI helper can locate the radio. Returns
/// `None` when no wireless interface is present yet.
async fn discover_wlan_syspath() -> Option<String> {
    let mut interfaces = tokio::fs::read_dir("/sys/class/net").await.ok()?;
    while let Ok(Some(interface)) = interfaces.next_entry().await {
        // A wireless interface exposes a `phy80211` link.
        if tokio::fs::metadata(interface.path().join("phy80211"))
            .await
            .is_ok()
            && let Ok(device) = tokio::fs::canonicalize(interface.path().join("device")).await
        {
            return Some(device.to_string_lossy().into_owned());
        }
    }
    None
}

/// Read the current station link RSSI from `iw dev <device> link`.
async fn get_link_signal(device: &str) -> Option<i32> {
    let output = CommandUtils::call_iw_cmd(&["dev", device, "link"])
        .await
        .ok()?;
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("signal:").map(str::trim))
        .and_then(|signal| signal.split_whitespace().next())
        .and_then(|level| level.parse().ok())
}

/// Query the ESP32 setup AP SSID via its control CLI. The CLI prints the SSID on
/// the first line starting at the third whitespace-separated field.
async fn get_softap_ssid() -> Option<String> {
    let output = Command::new(sdio::CLI_COMMAND)
        .arg(sdio::GET_SOFTAP_CONFIG)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = output.stdout.to_str_lossy();
    let ssid = stdout.lines().next()?.splitn(3, ' ').nth(2)?;
    ssid.contains(AP_SSID_PREFIX)
        .then(|| ssid.trim().to_owned())
}

async fn ap_scan() -> Result<Vec<WifiScanItem>> {
    Ok(sdio::Esp32Sdio::get_ap_scan_list()
        .await?
        .into_iter()
        .map(|sdio::Ap { ssid, rssi, auth }| {
            WifiScanItem::new(ssid, rssi, auth_to_encryption(&auth))
        })
        .collect())
}

fn auth_to_encryption(auth: &sdio::AuthMode) -> EncryptionType {
    match auth {
        sdio::AuthMode::Open | sdio::AuthMode::Unknown => EncryptionType::None,
        sdio::AuthMode::Wep => EncryptionType::Wep,
        sdio::AuthMode::WpaPsk => EncryptionType::Wpa,
        sdio::AuthMode::Wpa2Psk | sdio::AuthMode::Wpa2Enterprise => EncryptionType::Wpa2,
        sdio::AuthMode::WpaWpa2Psk => EncryptionType::Wpa1_2,
        sdio::AuthMode::Wpa3Psk => EncryptionType::Wpa3,
        sdio::AuthMode::Wpa2Wpa3Psk => EncryptionType::Wpa2_3,
    }
}

/// Runs `snippet` with the platform's WiFi shell libraries sourced.
///
/// The setup AP is owned by these helpers rather than by any UCI section, so
/// the driver calls them instead of reimplementing `esp32-sdio-cli` handling.
async fn run_sourced(snippet: &str) -> Result<()> {
    let script = format!(". {BOS_DEFAULTS_LIB} && . {ESP32_WIFI_LIB} && {snippet}");
    let status = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .status()
        .await
        .map_err(|e| anyhow!("failed to run `{snippet}`: {e}"))?;
    if !status.success() {
        bail!("`{snippet}` failed with {status}");
    }
    Ok(())
}

async fn run_service_cmd(path: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(path)
        .args(args)
        .status()
        .await
        .map_err(|e| anyhow!("failed to run {path}: {e}"))?;
    if !status.success() {
        bail!("{path} {args:?} failed with {status}");
    }
    Ok(())
}

#[async_trait]
impl WifiDriver for Esp32WifiManager {
    async fn scan(&self) -> Result<Vec<WifiScanItem>> {
        let items = if Self::is_ap_mode().await {
            ap_scan().await?
        } else {
            let device = self.get_device().await?;
            self.scan_cache
                .lock()
                .await
                .cached_or_else::<anyhow::Error>(Box::pin(async move {
                    scanner::wifi_scan(&device).await
                }))
                .await?
        };

        let items = items
            .into_iter()
            .filter(filter_unsupported_enc)
            .filter(filter_empty_ssid)
            .collect();
        let mut items = filter_sort_by_strongest_signal(items);
        mark_connected(&mut items, self.sta_ssid().await);
        Ok(items)
    }

    async fn status(&self) -> Result<WifiStatus> {
        self.status_all()
            .await?
            .into_iter()
            .find(|status| status.enabled)
            .ok_or_else(|| anyhow!("No enabled WiFi interface found"))
    }

    async fn status_all(&self) -> Result<Vec<WifiStatus>> {
        let syspath = self.wlan_dev_syspath().await?;
        self.status_cache
            .lock()
            .await
            .cached_or_else::<anyhow::Error>(Box::pin(async move {
                let uci = UciHelper::new(&syspath);
                let device = WifiUtils::get_device_by_syspath(&syspath).await?;
                let sta_ssid = match uci.wifi_iface_find_enabled().await {
                    Some(config) if config.mode == WifiMode::Station => Some(config.ssid),
                    _ => None,
                };
                let link_state = match (sta_ssid, get_link_signal(&device).await) {
                    (Some(ssid), Some(level)) => Some(WifiLinkState::new(&ssid, level)),
                    _ => None,
                };
                Ok(uci
                    .get_all_wifi_ifaces()
                    .await?
                    .into_iter()
                    .map(|iface| map_uci_iface_to_wifi_status(iface, link_state.clone()))
                    .collect())
            }))
            .await
    }

    async fn save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()> {
        // The station stack only exists once the ESP32 runs its "NG" firmware,
        // so flash it before writing any wireless config; skipping it leaves a
        // factory module with no station capability to configure.
        info!("Flashing ESP32 NG firmware before joining {ssid}");
        run_service_cmd(ESP32_SERVICE, &["reload_await", "--force-ng"]).await?;
        wait_for_wireless_config().await?;

        let device = self.get_device().await?;
        let uci = self.uci().await?;
        uci.wifi_iface_disable_all().await?;
        uci.wifi_iface_configure(
            WifiMode::Station,
            ssid,
            encryption,
            password.unwrap_or_default(),
        )
        .await?;
        uci.save_changes().await?;

        self.enable_radio(true).await?;
        wait_for_network_ip_address(&device, ATTEMPTS_TO_GET_IP).await
    }

    async fn configure_ap_mode(
        &self,
        ssid: String,
        _password: Option<String>,
        _encryption: EncryptionType,
    ) -> Result<()> {
        // Raise the AP the way the platform does: `start_wifi_ap` starts the
        // ESP32 softAP and brings up the `wifi_ap` network. Reflashing the
        // module only swaps firmware - it never starts an access point, so
        // doing that here leaves the board with no setup AP at all.
        //
        // The SSID argument is ignored on purpose: the platform advertises
        // `default_ssid`, and taking it from the shell keeps this driver and
        // the boot-time `factory-default-wifi` service on one name.
        //
        // The softAP lives behind the hosted-control node, which only the "FG"
        // firmware exposes; a board that has left factory default runs "NG" and
        // has none. `esp32-init` writes the right firmware at boot, and that is
        // a minutes-long UART transfer needing the port's service stopped
        // first, so report the state rather than reflashing from in here.
        if tokio::fs::metadata(ESP_CONTROL_NODE).await.is_err() {
            bail!(
                "{ESP_CONTROL_NODE} is missing: the ESP32 is running station firmware, so no \
                 setup AP can be started until the board boots in factory-default mode"
            );
        }

        info!("Starting ESP32 setup AP (requested ssid ignored: {ssid})");
        run_sourced(r#"start_wifi_ap "$(default_ssid)""#).await
    }

    async fn stop_ap(&self) -> Result<()> {
        // Symmetric with `configure_ap_mode`: stop the softAP and take the
        // `wifi_ap` network down again, leaving the station config untouched.
        info!("Stopping ESP32 setup AP");
        run_sourced("stop_wifi_ap").await
    }

    async fn enable_radio(&self, enable: bool) -> Result<()> {
        let uci = self.uci().await?;
        uci.wifi_radio_enable(enable).await?;
        uci.save_changes().await?;
        WifiCommand::reload().await
    }

    async fn reset_config(&self) -> Result<()> {
        debug!("Removing wireless config");
        if let Err(e) = tokio::fs::remove_file(WIRELESS_CONFIG_FILE_PATH).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            bail!("Unable to remove wireless config: {e}");
        }
        run_service_cmd(FACTORY_DEFAULT_WIFI_SERVICE, &["start"]).await
    }

    async fn ap_ssid(&self) -> Option<String> {
        get_softap_ssid().await
    }

    async fn sta_ssid(&self) -> Option<String> {
        match self.uci().await.ok()?.wifi_iface_find_enabled().await {
            Some(config) if config.mode == WifiMode::Station => Some(config.ssid),
            _ => None,
        }
    }

    async fn wifi_device_name(&self) -> Result<String> {
        self.get_device().await
    }
}
