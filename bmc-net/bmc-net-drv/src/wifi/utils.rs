// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::path::{Path, PathBuf};

use anyhow::{Error, Result, anyhow, bail};
use bmc_net_types::wifi::{EncryptionType, WifiScanItem};
use bstr::ByteSlice;
use log::debug;
use strum::{Display, EnumString};
use tokio::process::Command;
use tokio::time::{self, Duration, MissedTickBehavior};

use crate::{NetworkInterface, WIRELESS_CONFIG_FILE_PATH};

/// How many times [`wait_for_network_ip_address`] polls before giving up.
pub(crate) const ATTEMPTS_TO_GET_IP: u8 = 30;
/// Delay between IP-assignment polls.
const IP_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Delay between wireless-config existence polls.
const WIRELESS_CONFIG_WAIT_INTERVAL: Duration = Duration::from_secs(1);
/// How many times [`wait_for_wireless_config`] polls before giving up.
const WIRELESS_CONFIG_GET_ATTEMPTS: u8 = 20;
/// The default OpenWRT `/etc/config/wireless` is always at least this large, so
/// a smaller file means the (asynchronous) write is still in progress.
const WIRELESS_CONFIG_MIN_SIZE: u64 = 300;

/// Wait until `device` has an IPv4 address assigned, polling once per second.
///
/// Shared by every backend so a retry budget tuned for one does not silently
/// diverge from the other. The `getifaddrs(3)` walk runs on the blocking pool:
/// it can stall for seconds while the kernel holds the rtnl lock.
pub(crate) async fn wait_for_network_ip_address(device: &str, attempts: u8) -> Result<()> {
    debug!("Wifi connected, waiting for IP address...");
    let mut interval = time::interval(IP_CHECK_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    for i in 0..attempts {
        debug!("{i}/{attempts} attempt to get IP address from {device}");
        interval.tick().await;
        let device = device.to_owned();
        let ip = tokio::task::spawn_blocking(move || {
            NetworkInterface::get_by_substr(&device).and_then(|network| network.ipv4_address())
        })
        .await
        .map_err(|e| anyhow!("interface walk task panicked: {e}"))?;
        if let Some(ip) = ip {
            debug!("IP is assigned: {ip}, connection is complete");
            return Ok(());
        }
    }
    Err(anyhow!("IP cannot be assigned. Failed to setup wifi"))
}

/// Wait until `/etc/config/wireless` exists and looks fully written.
///
/// Freshly flashed firmware writes the file asynchronously, so the size check
/// keeps callers from loading a half-written config.
pub(crate) async fn wait_for_wireless_config() -> Result<()> {
    let mut interval = time::interval(WIRELESS_CONFIG_WAIT_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    for i in 0..WIRELESS_CONFIG_GET_ATTEMPTS {
        interval.tick().await;
        debug!("Checking if wireless config exists. Attempt {i}/{WIRELESS_CONFIG_GET_ATTEMPTS}");
        if let Ok(metadata) = tokio::fs::metadata(WIRELESS_CONFIG_FILE_PATH).await
            && metadata.len() >= WIRELESS_CONFIG_MIN_SIZE
        {
            return Ok(());
        }
    }

    bail!("Wi-Fi config is not present")
}

/// Flags the scan item matching `connected_ssid` as the connected network.
pub(crate) fn mark_connected(items: &mut [WifiScanItem], connected_ssid: Option<String>) {
    if let Some(ssid) = connected_ssid {
        for item in items.iter_mut() {
            item.connected = item.ssid == ssid;
        }
    }
}

/// Deduplicate by SSID+encryption keeping the strongest signal, then order the
/// list strongest-first (ties broken alphabetically). Shared by every scanner.
pub(crate) fn filter_sort_by_strongest_signal(mut items: Vec<WifiScanItem>) -> Vec<WifiScanItem> {
    // Sort by SSID + encryption first so dedup keeps the strongest signal per
    // network. Matching SSIDs with different encryption types survive, since the
    // WPA3-only filter would otherwise drop the SSID entirely (BOS-2753).
    items.sort_by(|a, b| {
        b.ssid
            .cmp(&a.ssid)
            .then_with(|| b.encryption_type.cmp(&a.encryption_type))
            .then_with(|| b.signal_level.cmp(&a.signal_level))
    });
    items.dedup_by_key(|item| (item.ssid.clone(), item.encryption_type));
    // Present the strongest signal first, ties broken alphabetically by SSID.
    items.sort_by(|a, b| {
        b.signal_level
            .cmp(&a.signal_level)
            .then_with(|| a.ssid.cmp(&b.ssid))
    });
    items
}

/// Drops WPA3-only networks, which the station stack cannot yet join (BOS-2753).
pub(crate) fn filter_unsupported_enc(item: &WifiScanItem) -> bool {
    item.encryption_type != EncryptionType::Wpa3
}

/// Drops hidden networks whose SSID is empty.
pub(crate) fn filter_empty_ssid(item: &WifiScanItem) -> bool {
    !item.ssid.is_empty()
}

#[derive(Debug)]
pub struct WifiUtils;

impl WifiUtils {
    pub async fn get_device_by_syspath(syspath: &str) -> Result<String> {
        tokio::fs::read_dir(Path::new(syspath).join("net"))
            .await
            .map_err(|e| anyhow!("Could not access `net` under {syspath}: {e}"))?
            .next_entry()
            .await
            .map_err(|e| anyhow!("Could not access entries under {syspath}/net: {e}"))?
            .ok_or(anyhow!("No wifi device in specified syspath: {syspath}"))?
            .file_name()
            .into_string()
            .map_err(|e| Error::msg(format!("non-UTF-8 wifi device name: {}", e.display())))
    }

    pub async fn get_phy_path_by_syspath(syspath: &str) -> Result<PathBuf> {
        Ok(tokio::fs::read_dir(Path::new(syspath).join("ieee80211"))
            .await
            .map_err(|e| anyhow!("Could not access `ieee80211` under {syspath}: {e}"))?
            .next_entry()
            .await
            .map_err(|e| anyhow!("Could not access entries under {syspath}/ieee80211: {e}"))?
            .ok_or(anyhow!("No phy device in specified syspath: {syspath}"))?
            .path())
    }
}

/// Masks the value of every `"key"` field in a JSON-like string so WiFi
/// passwords never reach the logs. Handles both compact serde_json output
/// (`"key":"pass"`) and pretty-printed ubus output (`"key": "pass"`),
/// including escaped quotes inside the value.
#[expect(
    clippy::string_slice,
    reason = "every offset here comes from find(), char_indices() or a suffix of the               same &str, so all of them are char boundaries"
)]
pub(crate) fn redact_wifi_key(input: &str) -> String {
    const FIELD: &str = "\"key\"";

    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(pos) = rest.find(FIELD) {
        let field_end = pos + FIELD.len();
        out.push_str(&rest[..field_end]);
        rest = &rest[field_end..];

        let Some(value) = rest
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
            .and_then(|after_colon| after_colon.strip_prefix('"'))
        else {
            continue;
        };

        let Some(value_len) = json_string_value_len(value) else {
            continue;
        };

        // Emit everything up to and including the opening quote, then the
        // placeholder; leave the closing quote in `rest` for the next round.
        out.push_str(&rest[..rest.len() - value.len()]);
        out.push_str("<redacted>");
        rest = &value[value_len..];
    }

    out.push_str(rest);
    out
}

/// Length of a JSON string value up to (not including) its closing quote,
/// respecting backslash escapes; `None` if the string is unterminated.
fn json_string_value_len(s: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(i);
        }
    }
    None
}

#[derive(Debug)]
pub struct CommandUtils;

impl CommandUtils {
    const DEFAULT_DELAY: u64 = 20;

    async fn call_command_to_string(
        command_name: &str,
        args: &[&str],
        timeout: u64,
    ) -> Result<String> {
        let mut command = Command::new(command_name);
        for arg in args {
            command.arg(arg);
        }
        // On timeout the `output()` future is dropped; make that kill the
        // child instead of leaving it running in the background.
        command.kill_on_drop(true);

        tokio::time::timeout(tokio::time::Duration::from_secs(timeout), command.output())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout waiting for {command_name} result!"))?
            .map(|res| {
                debug!(
                    "Command '{} {:?}' returned:{:?}\nstdout:\n{:?}\nstderr:\n{:?}\n",
                    command_name,
                    args.iter()
                        .map(|arg| redact_wifi_key(arg))
                        .collect::<Vec<_>>(),
                    res.status.code(),
                    res.stdout.to_str().map(redact_wifi_key),
                    res.stderr.to_str().map(redact_wifi_key)
                );

                res.status
                    .success()
                    .then(|| String::from_utf8_lossy(&res.stdout).to_string())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Command {command_name} returned error {:?}",
                            res.status.code()
                        )
                    })
            })?
    }

    pub async fn call_iw_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/usr/sbin/iw", args, Self::DEFAULT_DELAY).await
    }

    pub async fn call_ifconfig_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/sbin/ifconfig", args, Self::DEFAULT_DELAY).await
    }

    /// `iwlist` scans can stall in the driver, so the caller picks the timeout.
    pub async fn call_iwlist_cmd(args: &[&str], timeout: u64) -> Result<String> {
        Self::call_command_to_string("/usr/sbin/iwlist", args, timeout).await
    }

    pub async fn call_ubus_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/bin/ubus", args, Self::DEFAULT_DELAY).await
    }

    pub async fn call_wifi_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/sbin/wifi", args, Self::DEFAULT_DELAY).await
    }
}

#[derive(Display, EnumString, Debug)]
pub enum WifiCommand {
    #[strum(serialize = "up")]
    Up,
    #[strum(serialize = "down")]
    Down,
    #[strum(serialize = "reload")]
    Reload,
    #[strum(serialize = "config")]
    Config,
}

impl WifiCommand {
    pub async fn restart() -> Result<()> {
        WifiCommand::down().await?;
        WifiCommand::up().await
    }

    pub async fn up() -> Result<()> {
        Self::run(Self::Up).await
    }

    pub async fn down() -> Result<()> {
        Self::run(Self::Down).await
    }

    pub async fn reload() -> Result<()> {
        Self::run(Self::Reload).await
    }

    pub async fn config() -> Result<()> {
        Self::run(Self::Config).await
    }

    async fn run(command: WifiCommand) -> Result<()> {
        _ = CommandUtils::call_wifi_cmd(&[&command.to_string()]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{filter_sort_by_strongest_signal, mark_connected, redact_wifi_key};
    use bmc_net_types::wifi::{EncryptionType, WifiScanItem};

    #[test]
    fn dedups_matching_networks_keeping_strongest_signal() {
        let items = vec![
            WifiScanItem::new("test".into(), -80, EncryptionType::None),
            WifiScanItem::new("test".into(), -50, EncryptionType::None),
        ];
        let sorted = filter_sort_by_strongest_signal(items);
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].signal_level, -50);
    }

    #[test]
    fn marks_only_the_connected_ssid() {
        let mut items = vec![
            WifiScanItem::new("home".into(), -50, EncryptionType::Wpa2),
            WifiScanItem::new("other".into(), -70, EncryptionType::Wpa2),
        ];
        mark_connected(&mut items, Some("home".into()));
        assert!(items[0].connected);
        assert!(!items[1].connected);
    }

    #[test]
    fn marks_nothing_when_not_connected() {
        let mut items = vec![WifiScanItem::new("home".into(), -50, EncryptionType::Wpa2)];
        mark_connected(&mut items, None);
        assert!(!items[0].connected);
    }

    #[test]
    fn redacts_wifi_key_values() {
        assert_eq!(
            redact_wifi_key(r#"{"ssid":"net","key":"secret","disabled":"0"}"#),
            r#"{"ssid":"net","key":"<redacted>","disabled":"0"}"#
        );
        assert_eq!(
            redact_wifi_key("{\n\t\"key\": \"se\\\"cret\",\n\t\"mode\": \"ap\"\n}"),
            "{\n\t\"key\": \"<redacted>\",\n\t\"mode\": \"ap\"\n}"
        );
        assert_eq!(
            redact_wifi_key(r#"{"a":"1","b":"2"}"#),
            r#"{"a":"1","b":"2"}"#
        );
    }
}
