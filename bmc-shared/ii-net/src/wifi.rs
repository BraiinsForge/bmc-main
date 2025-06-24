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

use strum_macros::{Display, EnumString};

#[derive(Default, EnumString, Debug, Display, Eq, PartialEq, Clone)]
pub enum WifiMode {
    #[default]
    Station,
    Ap,
}

impl WifiMode {
    #[must_use]
    pub fn to_uci_mode(&self) -> String {
        match self {
            WifiMode::Ap => "ap".to_owned(),
            WifiMode::Station => "sta".to_owned(),
        }
    }

    #[must_use]
    pub fn to_uci_network(&self) -> String {
        match self {
            WifiMode::Ap => "wifi_ap".to_owned(),
            WifiMode::Station => "wifi_sta".to_owned(),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Default, Display, PartialOrd, Ord)]
pub enum SignalStrength {
    #[default]
    Offline,
    Low,
    Fair,
    Excellent,
}

impl SignalStrength {
    #[must_use]
    pub fn new(signal: i32) -> Self {
        match signal {
            0 => SignalStrength::Offline,
            x if x >= -60 => SignalStrength::Excellent,
            x if x >= -75 => SignalStrength::Fair,
            _ => SignalStrength::Low,
        }
    }
}

#[derive(Debug, EnumString, PartialEq, Eq, PartialOrd, Ord, Default, Clone, Copy, Display)]
pub enum EncryptionType {
    #[default]
    None,
    Wep,
    WepShared,
    Wpa,
    Wpa1_2,
    Wpa2,
    Wpa2_3,
    Wpa3, // Currently not supported on BMM101
}

impl EncryptionType {
    #[must_use]
    pub fn to_uci_str(&self) -> &str {
        match self {
            EncryptionType::None => "none",
            EncryptionType::Wep => "wep",
            EncryptionType::WepShared => "wep+shared",
            EncryptionType::Wpa => "psk",
            EncryptionType::Wpa1_2 => "psk-mixed",
            EncryptionType::Wpa2 => "psk2",
            EncryptionType::Wpa2_3 => "sae-mixed",
            EncryptionType::Wpa3 => "sae",
        }
    }

    #[must_use]
    pub fn from_uci_str(s: &str) -> Option<Self> {
        match s {
            "none" => Some(EncryptionType::None),
            "wep" => Some(EncryptionType::Wep),
            "wep+shared" => Some(EncryptionType::WepShared),
            "psk" => Some(EncryptionType::Wpa),
            "psk-mixed" => Some(EncryptionType::Wpa1_2),
            "psk2" => Some(EncryptionType::Wpa2),
            "sae-mixed" => Some(EncryptionType::Wpa2_3),
            "sae" => Some(EncryptionType::Wpa3),
            s => {
                log::warn!("Encryption type not recognized: {s}");
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WifiScanItem {
    pub ssid: String,
    pub signal_level: i32,
    pub encryption_type: EncryptionType,
}

impl WifiScanItem {
    #[must_use]
    pub fn new(ssid: String, signal_level: i32, encryption_type: EncryptionType) -> Self {
        Self {
            ssid,
            signal_level,
            encryption_type,
        }
    }

    #[must_use]
    pub fn signal_strength(&self) -> SignalStrength {
        SignalStrength::new(self.signal_level)
    }
}

#[derive(Default, Debug, PartialEq, Clone, Eq)]
pub struct WifiStatus {
    pub enabled: bool,
    pub configuration: Option<WifiConfiguration>,
    pub sta_link_state: Option<WifiLinkState>,
}

#[derive(Default, Debug, PartialEq, Clone, Eq)]
pub struct WifiConfiguration {
    pub mode: WifiMode,
    pub ssid: String,
    pub encryption_type: EncryptionType,
}

#[derive(Default, Debug, PartialEq, Clone, Eq)]
pub struct WifiLinkState {
    pub ssid: String,
    pub signal_level: i32,
}

impl WifiLinkState {
    #[must_use]
    pub fn new(ssid: &str, signal_level: i32) -> Self {
        Self {
            ssid: ssid.to_owned(),
            signal_level,
        }
    }

    #[must_use]
    pub fn signal_strength(&self) -> SignalStrength {
        SignalStrength::new(self.signal_level)
    }
}
