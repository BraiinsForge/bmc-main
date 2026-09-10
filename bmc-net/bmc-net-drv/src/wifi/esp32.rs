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
use bmc_net_types::wifi::{
    EncryptionType, WifiConfiguration, WifiLinkState, WifiMode, WifiScanItem, WifiStatus,
};
use bstr::ByteSlice;
use log::{debug, info, warn};
use std::fmt::Debug;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::Duration;

use super::uci::{UciHelper, map_uci_iface_to_wifi_status, pick_reported_status};
use super::utils::{
    ATTEMPTS_TO_ACTIVATE_AP, ATTEMPTS_TO_GET_IP, CommandUtils, WifiCommand, WifiUtils,
    filter_empty_ssid, filter_sort_by_strongest_signal, filter_unsupported_enc, mark_connected,
    wait_for_interface_up, wait_for_station_ready, wait_for_wireless_config,
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

    /// After a failed join, re-enable the station that was active before so a
    /// typo in an SSID does not leave the miner without connectivity. Errors
    /// are logged: the caller reports the join failure itself.
    async fn restore_station(&self, previous: Option<WifiConfiguration>, attempted: &str) {
        let Some(previous) = previous else {
            return;
        };
        let restored = async {
            let uci = self.uci().await?;
            uci.wifi_iface_disable_all().await?;
            if !uci
                .wifi_iface_enable(WifiMode::Station, &previous.ssid)
                .await?
            {
                bail!("no saved wifi-iface for {}", previous.ssid);
            }
            uci.save_changes().await?;
            self.enable_radio(true).await
        }
        .await;
        match restored {
            Ok(()) => warn!(
                "Joining {attempted} failed; restored the previous station {}",
                previous.ssid
            ),
            Err(e) => warn!(
                "Joining {attempted} failed and the previous station {} could not be restored: {e:#}",
                previous.ssid
            ),
        }
    }

    async fn uci(&self) -> Result<UciHelper> {
        Ok(UciHelper::new(&self.wlan_dev_syspath().await?))
    }

    async fn get_device(&self) -> Result<String> {
        let syspath = self.wlan_dev_syspath().await?;
        match WifiUtils::get_device_by_syspath(&syspath).await {
            Ok(device) => Ok(device),
            Err(e) => {
                // The cached path can go stale across a firmware swap (the
                // setup AP netdev disappears, the station netdev appears);
                // rediscover once before giving up.
                debug!("No netdev under {syspath} ({e}); rediscovering the ESP32");
                let mut cached = self.wlan_dev_syspath.lock().await;
                *cached = discover_wlan_syspath().await;
                let syspath = cached
                    .clone()
                    .ok_or_else(|| anyhow!("No wireless interface found"))?;
                drop(cached);
                WifiUtils::get_device_by_syspath(&syspath).await
            }
        }
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
/// `None` when no wireless interface is present: on the setup ("FG") firmware
/// the module only exposes virtual `ethap0`/`ethsta0` netdevs and no radio, so
/// the callers that can answer without UCI (status, radio power) must handle
/// that state themselves instead of failing.
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

/// Whether the ESP32 runs the setup ("FG") firmware, decided by the netdev the
/// module registers: "FG" brings up [`AP_INTERFACE_NAME`], "NG" a `wlan`
/// device. That is what UCI and netifd bind to.
async fn esp32_on_setup_firmware(caller: &str) -> bool {
    let on_fg = Esp32WifiManager::is_ap_mode().await;
    info!(
        "{caller}: {AP_INTERFACE_NAME} {}, ESP32 is on {} firmware",
        if on_fg { "present" } else { "absent" },
        if on_fg { "setup (FG)" } else { "station (NG)" }
    );
    on_fg
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

/// Swap the module to the setup ("FG") firmware and wait for its AP interface.
///
/// `esp32-init reload_await` takes the FG branch only while the factory-default
/// flag is set, so the service's steps are replayed here with that branch
/// forced. Takes about 20 s on a BMM101.
async fn reflash_to_setup_firmware() -> Result<()> {
    info!("Flashing the setup (FG) firmware to bring the softAP back");
    run_service_cmd(ESP32_SERVICE, &["stop"]).await?;
    run_sourced(
        "rmmod esp32-sdio 2>/dev/null || true; flash_fg_firmware && modprobe esp32-sdio-fg",
    )
    .await?;
    run_service_cmd(ESP32_SERVICE, &["start"]).await?;
    for _ in 0..ATTEMPTS_TO_ACTIVATE_AP {
        if Esp32WifiManager::is_ap_mode().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    bail!("{AP_INTERFACE_NAME} did not appear after flashing the setup firmware")
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
        pick_reported_status(&self.status_all().await?)
            .ok_or_else(|| anyhow!("No WiFi interface configured"))
    }

    async fn status_all(&self) -> Result<Vec<WifiStatus>> {
        let syspath = match self.wlan_dev_syspath().await {
            Ok(syspath) => syspath,
            // Setup ("FG") firmware: no wireless netdev and no `wireless` UCI
            // config exist yet, only the softAP. Report that instead of
            // failing, or boser hides WiFi altogether and initial setup can
            // neither list networks nor switch the radio (the pre-bmc-net
            // driver answered `enabled: false, mode: AP` here).
            Err(e) if Self::is_ap_mode().await => {
                debug!("No wireless netdev ({e}); reporting the setup AP status");
                return Ok(vec![WifiStatus {
                    enabled: false,
                    configuration: Some(WifiConfiguration {
                        mode: WifiMode::Ap,
                        ssid: get_softap_ssid().await.unwrap_or_default(),
                        encryption_type: EncryptionType::None,
                    }),
                    sta_link_state: None,
                }]);
            }
            Err(e) => return Err(e),
        };
        self.status_cache
            .lock()
            .await
            .cached_or_else::<anyhow::Error>(Box::pin(async move {
                // Saved config first, live link second: the ESP32 netdev does
                // survive the radio being disabled (verified on a BMM), but a
                // reflash renames it under the cached syspath, and the status
                // is then still the saved config rather than an error.
                let uci = UciHelper::new(&syspath);
                let (radio_enabled, ifaces) = uci.radio_state_with_ifaces().await?;
                let sta_ssid = match uci.wifi_iface_find_enabled().await {
                    Some(config) if config.mode == WifiMode::Station => Some(config.ssid),
                    _ => None,
                };
                let device = WifiUtils::get_device_by_syspath(&syspath)
                    .await
                    .inspect_err(|e| debug!("No WiFi netdev, reporting the saved config only: {e}"))
                    .ok();
                let signal = match device {
                    Some(device) => get_link_signal(&device).await,
                    None => None,
                };
                let link_state = match (sta_ssid, signal) {
                    (Some(ssid), Some(level)) => Some(WifiLinkState::new(&ssid, level)),
                    _ => None,
                };
                Ok(ifaces
                    .into_iter()
                    .map(|iface| {
                        map_uci_iface_to_wifi_status(iface, link_state.clone(), radio_enabled)
                    })
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
        // The station stack needs the "NG" firmware, so flash only on the
        // FG -> NG transition. Reflashing a settled station module every connect
        // renames its netdev (wlan0 -> wlan1 -> ...); the cached
        // `wlan_dev_syspath` still points at the old name, so the connect path
        // loses track of the module and reports a failure even though the join
        // succeeded.
        if esp32_on_setup_firmware("save_and_connect").await {
            info!("Flashing NG firmware before joining {ssid}");
            run_service_cmd(ESP32_SERVICE, &["reload_await", "--force-ng"]).await?;
            wait_for_wireless_config().await?;
        }

        let uci = self.uci().await?;
        // Remember the station we are leaving so a failed join can put it back.
        let previous = uci
            .wifi_iface_find_enabled()
            .await
            .filter(|config| config.mode == WifiMode::Station);
        uci.wifi_iface_disable_all().await?;
        uci.wifi_iface_configure(
            WifiMode::Station,
            ssid.clone(),
            encryption,
            password.unwrap_or_default(),
        )
        .await?;
        uci.save_changes().await?;

        self.enable_radio(true).await?;
        // Applying the station config resets the module, which re-registers
        // its netdev under the next free name, so resolve it on every poll.
        let joined = wait_for_station_ready(|| self.get_device(), &ssid, ATTEMPTS_TO_GET_IP).await;
        if let Err(e) = joined {
            self.restore_station(previous, &ssid).await;
            return Err(e);
        }
        Ok(())
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
        // `start_wifi_ap` returns once `ifup wifi_ap` is queued, not once the
        // interface is up: `wait_for_ap_active` covers that.
        //
        // The softAP exists only on the "FG" firmware; a board that has joined
        // a network runs "NG" and has no AP interface at all. A failed join (a
        // wrong password, most often) has to bring the setup AP back without a
        // reboot, so the swap happens right here.
        if !esp32_on_setup_firmware("configure_ap_mode").await {
            reflash_to_setup_firmware().await?;
        }

        info!("Starting ESP32 setup AP (requested ssid ignored: {ssid})");
        run_sourced(r#"start_wifi_ap "$(default_ssid)""#).await
    }

    async fn wait_for_ap_active(&self) -> Result<()> {
        // `start_wifi_ap` ends in `ifup wifi_ap`, which only queues the raise;
        // netifd reporting the interface up with its address is what makes the
        // AP joinable (and what dnsmasq needs to serve it).
        wait_for_interface_up(WifiMode::Ap.to_uci_network(), ATTEMPTS_TO_ACTIVATE_AP).await
    }

    async fn stop_ap(&self) -> Result<()> {
        // Symmetric with `configure_ap_mode`: stop the softAP and take the
        // `wifi_ap` network down again, leaving the station config untouched.
        info!("Stopping ESP32 setup AP");
        run_sourced("stop_wifi_ap").await
    }

    async fn enable_radio(&self, enable: bool) -> Result<()> {
        let uci = match self.uci().await {
            Ok(uci) => uci,
            // Setup ("FG") firmware: there is no station radio to switch, only
            // the softAP that initial setup itself depends on. Enabling is a
            // no-op success so the setup flow can proceed to the scan;
            // disabling would take the setup AP away from under the client.
            Err(e) if Self::is_ap_mode().await => {
                if enable {
                    info!("Setup AP active and no station radio yet ({e}); nothing to enable");
                    return Ok(());
                }
                bail!("the WiFi radio cannot be switched off while the setup AP is active");
            }
            Err(e) => return Err(e),
        };
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
