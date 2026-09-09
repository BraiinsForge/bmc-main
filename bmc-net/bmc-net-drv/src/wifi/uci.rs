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

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use bmc_net_types::wifi::{EncryptionType, WifiConfiguration, WifiLinkState, WifiMode, WifiStatus};
use log::debug;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use strum::{Display, EnumString};

use super::utils::{CommandUtils, redact_wifi_key};

#[derive(Deserialize, Clone)]
struct UciWirelessRadio {
    #[serde(alias = ".name")]
    name: String,
    path: String,
    disabled: Option<String>,
}

/// UCI `disabled` semantics: an absent flag, or `"0"`, means enabled.
fn uci_enabled(disabled: Option<&str>) -> bool {
    disabled.is_none_or(|value| value == "0")
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct UciWirelessIface {
    #[serde(alias = ".name", skip_serializing)]
    name: String,
    device: String,
    network: String,
    mode: String,
    ssid: String,
    encryption: String,
    key: Option<String>,
    disabled: Option<String>,
}

impl From<UciWirelessIface> for WifiConfiguration {
    fn from(iface: UciWirelessIface) -> Self {
        let mode = if iface.mode == "ap" {
            WifiMode::Ap
        } else {
            WifiMode::Station
        };

        // `from_uci_str` is a pure conversion: `bmc-net-types` deliberately has
        // no logging, so reporting the unrecognized keyword is this call site's
        // job. `unknown_encryption_keyword_falls_back_to_none` pins the
        // fallback so the diagnostic cannot be dropped again unnoticed.
        let encryption_type =
            EncryptionType::from_uci_str(&iface.encryption).unwrap_or_else(|| {
                log::warn!("Encryption type not recognized: {}", iface.encryption);
                EncryptionType::None
            });

        Self {
            mode,
            ssid: iface.ssid,
            encryption_type,
        }
    }
}

/// Build the reported [`WifiStatus`] for one `wifi-iface` section. `enabled`
/// folds in `radio_enabled`: the radio's `disabled` flag lives on the
/// `wifi-device`, so reading the iface flag alone reports a switched-off radio
/// as enabled.
pub(crate) fn map_uci_iface_to_wifi_status(
    iface: UciWirelessIface,
    link_state: Option<WifiLinkState>,
    radio_enabled: bool,
) -> WifiStatus {
    let iface_enabled = uci_enabled(iface.disabled.as_deref());
    let mut status = WifiStatus {
        enabled: radio_enabled && iface_enabled,
        configuration: Some(WifiConfiguration::from(iface)),
        sta_link_state: None,
    };

    let Some(link_state) = link_state else {
        return status;
    };

    let Some(config) = &status.configuration else {
        return status;
    };

    if config.ssid == link_state.ssid {
        status.sta_link_state = Some(link_state);
    }

    status
}

/// The interface `status()` reports out of `status_all()`: the live one, else a
/// deterministic pick among the disabled ones (station first, then by SSID)
/// rather than `HashMap` iteration order. A radio-off device must answer
/// `enabled: false` with its saved SSID, not an error indistinguishable from a
/// broken config, so this only fails when nothing is configured at all.
pub(crate) fn pick_reported_status(statuses: &[WifiStatus]) -> Option<WifiStatus> {
    statuses
        .iter()
        .find(|status| status.enabled)
        .or_else(|| {
            statuses.iter().min_by_key(|status| {
                let cfg = status.configuration.as_ref();
                let is_station = cfg.is_some_and(|c| c.mode == WifiMode::Station);
                (!is_station, cfg.map(|c| c.ssid.clone()).unwrap_or_default())
            })
        })
        .cloned()
}

#[derive(Display, EnumString)]
enum UciType {
    #[strum(serialize = "wifi-device")]
    WifiDevice,
    #[strum(serialize = "wifi-iface")]
    WifiIface,
}

#[derive(Display, EnumString)]
enum UciCommand {
    #[strum(serialize = "get")]
    Get,
    #[strum(serialize = "set")]
    Set,
    #[strum(serialize = "add")]
    Add,
    #[strum(serialize = "commit")]
    Commit,
}

impl UciCommand {
    const CONFIG: &str = "wireless";

    async fn call_ubus(mode: Self, params: Value) -> Result<String> {
        debug!(
            "Ubus uci {mode} command invoked with params: {}",
            redact_wifi_key(&params.to_string())
        );
        CommandUtils::call_ubus_cmd(&["call", "uci", &mode.to_string(), &params.to_string()]).await
    }

    pub async fn get<T: DeserializeOwned>(uci_type: UciType) -> Result<T> {
        let ubus_param = json!({"config": Self::CONFIG, "type": uci_type.to_string()});
        let ubus_out = Self::call_ubus(Self::Get, ubus_param).await?;

        let value = serde_json::from_str::<Value>(&ubus_out)?
            .get("values")
            .cloned()
            .ok_or_else(|| anyhow!("No values field in json"))?;

        serde_json::from_value::<T>(value).map_err(|e| anyhow!(e))
    }

    pub async fn set(uci_section: String, values: Value) -> Result<()> {
        let ubus_param = json!({"config": Self::CONFIG, "section": uci_section, "values": values});
        _ = Self::call_ubus(Self::Set, ubus_param).await?;

        Ok(())
    }

    pub async fn add(uci_type: UciType) -> Result<String> {
        let ubus_param = json!({"config": Self::CONFIG, "type": uci_type.to_string()});
        let ubus_out = Self::call_ubus(Self::Add, ubus_param).await?;

        Ok(serde_json::from_str::<HashMap<String, String>>(&ubus_out)?
            .get("section")
            .ok_or_else(|| anyhow!("Cannot parse new uci section name"))?
            .to_owned())
    }

    pub async fn commit() -> Result<()> {
        let ubus_param = json!({"config": Self::CONFIG});
        _ = Self::call_ubus(Self::Commit, ubus_param).await?;

        Ok(())
    }
}

#[derive(Display, EnumString)]
pub enum HtMode {
    #[strum(serialize = "NOHT")]
    NoHt,
}
pub struct UciHelper {
    wifi_device_syspath: String,
}

impl UciHelper {
    pub fn new(device_syspath: &str) -> Self {
        Self {
            wifi_device_syspath: device_syspath.to_owned(),
        }
    }

    async fn get_radio(&self) -> Result<UciWirelessRadio> {
        let radios: Vec<UciWirelessRadio> =
            UciCommand::get::<HashMap<String, UciWirelessRadio>>(UciType::WifiDevice)
                .await?
                .into_values()
                .collect();
        if let Some(radio) = radios
            .iter()
            .find(|radio| self.wifi_device_syspath.contains(&radio.path))
        {
            return Ok(radio.clone());
        }
        // The path recorded in UCI can differ from the live sysfs path (the
        // ESP32 on its setup firmware, a re-enumerated USB adapter). A board
        // with exactly one radio has nothing to disambiguate, so use it — the
        // pre-bmc-net driver always addressed `radio0` and never failed here.
        match radios.as_slice() {
            [radio] => {
                debug!(
                    "No radio matches {}; using the only configured radio {}",
                    self.wifi_device_syspath, radio.name
                );
                Ok(radio.clone())
            }
            _ => Err(anyhow!("Specified radio not found")),
        }
    }

    /// The radio's enabled state plus the `wifi-iface` sections bound to it,
    /// from one lookup — callers reporting WiFi status need both halves (see
    /// [`map_uci_iface_to_wifi_status`]).
    pub(crate) async fn radio_state_with_ifaces(&self) -> Result<(bool, Vec<UciWirelessIface>)> {
        let radio = self.get_radio().await?;
        let ifaces = UciCommand::get::<HashMap<String, UciWirelessIface>>(UciType::WifiIface)
            .await?
            .into_values()
            .filter(|iface| iface.device == radio.name)
            .collect();

        Ok((uci_enabled(radio.disabled.as_deref()), ifaces))
    }

    pub(crate) async fn get_all_wifi_ifaces(&self) -> Result<Vec<UciWirelessIface>> {
        Ok(self.radio_state_with_ifaces().await?.1)
    }

    /// The first enabled `wifi-iface` section, or `None` when the radio itself
    /// is disabled: the same rule [`map_uci_iface_to_wifi_status`] applies, so
    /// the SSID helpers and `status()` never disagree on what is active.
    pub async fn wifi_iface_find_enabled(&self) -> Option<WifiConfiguration> {
        match self.radio_state_with_ifaces().await {
            Ok((false, _)) => None,
            Ok((true, ifaces)) => ifaces
                .into_iter()
                .find(|iface| uci_enabled(iface.disabled.as_deref()))
                .map(Into::into),
            Err(e) => {
                log::warn!("Cannot get iface from uci: {e}");
                None
            }
        }
    }

    pub async fn wifi_iface_disable_all(&self) -> Result<()> {
        let iface_section_names = self
            .get_all_wifi_ifaces()
            .await?
            .into_iter()
            .map(|iface| iface.name);

        for section in iface_section_names {
            UciCommand::set(section, json!({"disabled": "1"})).await?;
        }

        Ok(())
    }

    /// Re-enable the wifi-iface section saved for `mode` and `ssid`, keeping its
    /// stored key. Returns `false` when no such section exists.
    pub async fn wifi_iface_enable(&self, mode: WifiMode, ssid: &str) -> Result<bool> {
        let uci_mode = mode.to_uci_mode();
        let Some(section) = self
            .get_all_wifi_ifaces()
            .await?
            .into_iter()
            .find(|iface| iface.ssid == ssid && iface.mode == uci_mode)
            .map(|iface| iface.name)
        else {
            return Ok(false);
        };
        UciCommand::set(section, json!({"disabled": "0"})).await?;
        Ok(true)
    }

    /// Disables only the wifi-iface sections configured for `mode`, leaving
    /// sections in other modes (e.g. an active station) untouched.
    pub async fn wifi_iface_disable_by_mode(&self, mode: WifiMode) -> Result<()> {
        let ifaces = self.get_all_wifi_ifaces().await?;

        for section in iface_sections_with_mode(ifaces, &mode) {
            UciCommand::set(section, json!({"disabled": "1"})).await?;
        }

        Ok(())
    }

    pub async fn wifi_radio_enable(&self, enabled: bool) -> Result<()> {
        let radio = self.get_radio().await?;
        let disabled = if enabled { "0" } else { "1" };

        UciCommand::set(radio.name, json!({"disabled": disabled})).await
    }

    pub async fn wifi_radio_configure_beacon_int(&self, beacon_int: u32) -> Result<()> {
        let radio = self.get_radio().await?;
        UciCommand::set(radio.name, json!({"beacon_int": beacon_int.to_string()})).await
    }

    pub async fn wifi_radio_configure_ht_mode(&self, ht_mode: HtMode) -> Result<()> {
        let radio = self.get_radio().await?;
        UciCommand::set(radio.name, json!({"htmode": ht_mode.to_string()})).await
    }

    pub async fn wifi_radio_configure_ap_channel(&self, channel: u32) -> Result<()> {
        let max_2g_channel = 14;
        let radio = self.get_radio().await?;
        let band = if channel <= max_2g_channel {
            "2g"
        } else {
            "5g"
        };

        UciCommand::set(radio.name.clone(), json!({"channel": channel.to_string()})).await?;
        UciCommand::set(radio.name, json!({"band": band.to_string()})).await
    }

    pub async fn wifi_iface_configure(
        &self,
        mode: WifiMode,
        ssid: String,
        encryption: EncryptionType,
        password: String,
    ) -> Result<()> {
        let device = self.get_radio().await?.name;
        let iface_name = match self
            .get_all_wifi_ifaces()
            .await?
            .into_iter()
            .find(|iface| iface.ssid == ssid && iface.mode == mode.to_uci_mode())
            .map(|iface| iface.name)
        {
            Some(iface_name) => iface_name,
            None => UciCommand::add(UciType::WifiIface).await?,
        };

        debug!("Configure iface: {iface_name}, for radio: {device}");

        let values = UciWirelessIface {
            name: "~unused~".to_owned(),
            device,
            network: mode.to_uci_network().to_owned(),
            mode: mode.to_uci_mode().to_owned(),
            ssid,
            key: Some(password),
            encryption: encryption.to_uci_str().to_owned(),
            disabled: Some("0".to_owned()),
        };

        UciCommand::set(iface_name, serde_json::to_value(values)?).await
    }

    pub async fn save_changes(self) -> Result<()> {
        UciCommand::commit().await
    }
}

fn iface_sections_with_mode(ifaces: Vec<UciWirelessIface>, mode: &WifiMode) -> Vec<String> {
    ifaces
        .into_iter()
        .filter(|iface| iface.mode == mode.to_uci_mode())
        .map(|iface| iface.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    fn iface(name: &str, mode: &str) -> UciWirelessIface {
        UciWirelessIface {
            name: name.to_owned(),
            device: "radio0".to_owned(),
            network: "lan".to_owned(),
            mode: mode.to_owned(),
            ssid: "test".to_owned(),
            encryption: "none".to_owned(),
            key: None,
            disabled: None,
        }
    }

    fn status(enabled: bool, mode: WifiMode, ssid: &str) -> WifiStatus {
        WifiStatus {
            enabled,
            configuration: Some(WifiConfiguration {
                mode,
                ssid: ssid.to_owned(),
                encryption_type: EncryptionType::None,
            }),
            sta_link_state: None,
        }
    }

    #[test]
    fn reported_status_prefers_the_live_interface() {
        let statuses = [
            status(false, WifiMode::Station, "aaa"),
            status(true, WifiMode::Ap, "zzz"),
        ];
        let picked = pick_reported_status(&statuses).expect("BUG: a status is configured");
        assert!(picked.enabled);
        assert_eq!(picked.configuration.map(|c| c.ssid), Some("zzz".to_owned()));
    }

    #[test]
    fn reported_status_falls_back_to_the_station_then_the_ssid() {
        let statuses = [
            status(false, WifiMode::Ap, "aaa"),
            status(false, WifiMode::Station, "zzz"),
            status(false, WifiMode::Station, "mmm"),
        ];
        let picked = pick_reported_status(&statuses).expect("BUG: a status is configured");
        assert!(!picked.enabled);
        assert_eq!(picked.configuration.map(|c| c.ssid), Some("mmm".to_owned()));
        assert!(pick_reported_status(&[]).is_none());
    }

    #[test]
    fn disabling_ap_sections_leaves_station_sections_alone() {
        let ifaces = vec![iface("cfg_ap", "ap"), iface("cfg_sta", "sta")];

        let sections = iface_sections_with_mode(ifaces, &WifiMode::Ap);

        assert_eq!(sections, vec!["cfg_ap".to_owned()]);
    }

    #[test]
    fn every_known_encryption_keyword_survives_the_conversion() {
        for encryption_type in EncryptionType::iter() {
            let mut section = iface("cfg_sta", "sta");
            section.encryption = encryption_type.to_uci_str().to_owned();

            assert_eq!(
                WifiConfiguration::from(section).encryption_type,
                encryption_type
            );
        }
    }

    #[test]
    fn radio_disabled_reports_the_iface_as_not_enabled() {
        // A disabled radio with an untouched iface must read as not enabled.
        let section = iface("cfg_sta", "sta");
        assert!(section.disabled.is_none(), "BUG: fixture must be enabled");

        let status = map_uci_iface_to_wifi_status(section.clone(), None, false);

        assert!(!status.enabled);
        // The saved configuration still comes back with the radio off.
        assert_eq!(
            status.configuration.map(|config| config.ssid),
            Some(section.ssid)
        );
    }

    #[test]
    fn enabled_needs_both_the_radio_and_the_iface() {
        let enabled_iface = iface("cfg_sta", "sta");
        let mut disabled_iface = iface("cfg_ap", "ap");
        disabled_iface.disabled = Some("1".to_owned());

        assert!(map_uci_iface_to_wifi_status(enabled_iface.clone(), None, true).enabled);
        assert!(!map_uci_iface_to_wifi_status(enabled_iface, None, false).enabled);
        assert!(!map_uci_iface_to_wifi_status(disabled_iface.clone(), None, true).enabled);
        assert!(!map_uci_iface_to_wifi_status(disabled_iface, None, false).enabled);
    }

    #[test]
    fn uci_disabled_flag_semantics() {
        assert!(uci_enabled(None));
        assert!(uci_enabled(Some("0")));
        assert!(!uci_enabled(Some("1")));
    }

    #[test]
    fn unknown_encryption_keyword_falls_back_to_none() {
        // This is the only caller of `EncryptionType::from_uci_str`, and it owns
        // the "unrecognized keyword" diagnostic because the types crate is
        // logging-free. Pin the fallback so a future edit cannot silently turn
        // an unparseable section into a network reported as unencrypted.
        let mut section = iface("cfg_sta", "sta");
        section.encryption = "owe-transition".to_owned();

        assert_eq!(
            WifiConfiguration::from(section).encryption_type,
            EncryptionType::None
        );
    }
}
