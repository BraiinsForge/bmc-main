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

use strum_macros::{Display, EnumIter};
use thiserror::Error;

/// Returned by network-manager backends for boards without Wi-Fi hardware
/// when a Wi-Fi operation is requested.
#[derive(Debug, Error)]
#[error("This board does not support Wi-Fi")]
pub struct WifiUnsupportedError;

#[derive(Default, Debug, Display, Eq, PartialEq, Clone)]
pub enum WifiMode {
    #[default]
    Station,
    Ap,
}

impl WifiMode {
    #[must_use]
    pub fn to_uci_mode(&self) -> &'static str {
        match self {
            WifiMode::Ap => "ap",
            WifiMode::Station => "sta",
        }
    }

    #[must_use]
    pub fn to_uci_network(&self) -> &'static str {
        match self {
            WifiMode::Ap => "wifi_ap",
            WifiMode::Station => "wifi_sta",
        }
    }
}

/// Coarse signal-strength bucket, ordered `Offline < Low < Fair < Excellent`
/// (the derived `Ord` relies on this variant order).
#[derive(Debug, Eq, PartialEq, Clone, Copy, Default, Display, PartialOrd, Ord)]
pub enum SignalStrength {
    #[default]
    Offline,
    Low,
    Fair,
    Excellent,
}

/// dBm level reported when there is no signal at all (offline).
const NO_SIGNAL_DBM: i32 = 0;
/// Minimum dBm level bucketed as [`SignalStrength::Excellent`].
const EXCELLENT_MIN_DBM: i32 = -60;
/// Minimum dBm level bucketed as [`SignalStrength::Fair`].
const FAIR_MIN_DBM: i32 = -75;

impl SignalStrength {
    /// Buckets a signal level given in **dBm**. A level of `0` means "no
    /// signal / offline"; thresholds are `>= -60` Excellent, `>= -75` Fair,
    /// otherwise Low.
    #[must_use]
    pub fn new(signal: i32) -> Self {
        match signal {
            NO_SIGNAL_DBM => SignalStrength::Offline,
            x if x >= EXCELLENT_MIN_DBM => SignalStrength::Excellent,
            x if x >= FAIR_MIN_DBM => SignalStrength::Fair,
            _ => SignalStrength::Low,
        }
    }
}

#[derive(Debug, EnumIter, PartialEq, Eq, PartialOrd, Ord, Default, Clone, Copy, Display)]
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
    pub fn to_uci_str(&self) -> &'static str {
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

    /// Parses a UCI encryption keyword (the inverse of [`to_uci_str`]).
    ///
    /// Returns `None` for an unrecognized keyword. This is a pure conversion:
    /// the caller is responsible for logging/handling the unknown case (the
    /// types crate is intentionally free of logging side effects).
    ///
    /// [`to_uci_str`]: EncryptionType::to_uci_str
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
            _ => None,
        }
    }
}

/// A single access point returned by a scan.
#[derive(Debug, Clone)]
pub struct WifiScanItem {
    pub ssid: String,
    /// Received signal level in dBm (negative; closer to 0 is stronger).
    pub signal_level: i32,
    pub encryption_type: EncryptionType,
    /// Whether the station is currently connected to this network.
    pub connected: bool,
}

impl WifiScanItem {
    #[must_use]
    pub fn new(ssid: String, signal_level: i32, encryption_type: EncryptionType) -> Self {
        Self {
            ssid,
            signal_level,
            encryption_type,
            connected: false,
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

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn encryption_type_uci_roundtrip() {
        for encryption_type in EncryptionType::iter() {
            assert_eq!(
                EncryptionType::from_uci_str(encryption_type.to_uci_str()),
                Some(encryption_type)
            );
        }
    }
}
