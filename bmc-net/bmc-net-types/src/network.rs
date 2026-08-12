// Copyright (C) 2024, 2026  Braiins Systems s.r.o.
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

use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, Ipv4Addr};

use serde::{Deserialize, Serialize};
use strum_macros::Display;
use thiserror::Error;

use crate::MacAddr;
use crate::wifi::{EncryptionType, WifiStatus};

/// Lifecycle events emitted by a Wi-Fi driver.
///
/// Deliberately exhaustive (no `#[non_exhaustive]`): consumers match every
/// variant, so adding one should be a compile error that forces every handler
/// to be updated.
#[derive(Clone, Debug)]
pub enum WifiEvent {
    /// A scan has begun.
    ScanStarted,
    /// A scan has finished (emitted even if the scan future is cancelled).
    ScanEnded,
}

/// High-level provisioning state of the device.
///
/// Deliberately exhaustive (no `#[non_exhaustive]`): the state machine is
/// matched exhaustively across the codebase, so a new state must force every
/// handler to be revisited.
#[derive(Debug, Display, PartialEq)]
pub enum BmcState {
    #[strum(serialize = "factory default")]
    FactoryDefault,
    #[strum(serialize = "device setup")]
    SetupPending,
    #[strum(serialize = "operational")]
    Operational,
    #[strum(serialize = "wifi reconfiguration")]
    WifiReconfiguration,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Default)]
#[serde(tag = "proto", rename_all = "lowercase")]
pub enum NetworkProtocolConfig {
    #[default]
    Dhcp,
    Static(NetworkProtocolConfigStatic),
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct NetworkProtocolConfigStatic {
    pub address: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns_servers: Vec<Ipv4Addr>,
}

/// `Display` renders the lower-case UCI/PAPI spelling (`dhcp`, `static`);
/// it is serialized verbatim into the btctools status endpoint, so the case
/// is part of an external contract.
#[derive(Debug, Display, Eq, PartialEq)]
#[strum(serialize_all = "lowercase")]
pub enum NetworkProtocol {
    Dhcp,
    Static,
}

impl From<&NetworkProtocolConfig> for NetworkProtocol {
    fn from(network_config: &NetworkProtocolConfig) -> Self {
        match network_config {
            NetworkProtocolConfig::Dhcp => NetworkProtocol::Dhcp,
            NetworkProtocolConfig::Static(_) => NetworkProtocol::Static,
        }
    }
}

#[derive(Error, Debug)]
pub enum InitialSetupError {
    #[error("Initial setup is not supported")]
    NotSupported,
    #[error("Unexpected error during initial setup. {0}")]
    UnexpectedFailure(String),
    #[error("Connection to wifi was not successful. {0}")]
    WifiConnectionFailure(String),
}

/// Credentials for joining or hosting a Wi-Fi network.
///
/// `Debug` is hand-written to redact `password`, so credentials never reach
/// logs or error chains through `{:?}`.
pub struct WifiNetworkConfig {
    pub ssid: String,
    pub password: Option<String>,
    pub encryption: EncryptionType,
}

impl Debug for WifiNetworkConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiNetworkConfig")
            .field("ssid", &self.ssid)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("encryption", &self.encryption)
            .finish()
    }
}

#[derive(Debug, Default)]
pub struct NetworkInfo {
    pub interface_name: String,
    pub mac_address: Option<MacAddr>,
    pub hostname: Option<String>,
    pub protocol: Option<NetworkProtocol>,
    pub dns_servers: Vec<Ipv4Addr>,
    pub networks: Vec<IpNetwork>,
    pub default_gateway: Option<Ipv4Addr>,
}

#[derive(Debug, Clone)]
pub struct IpNetwork {
    pub address: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IfaceData {
    pub ip: Option<IpAddr>,
    pub mac: Option<MacAddr>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WifiData {
    pub iface: IfaceData,
    pub status: WifiStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_protocol_renders_lowercase() {
        // Serialized verbatim into the btctools status endpoint's `wan.proto`,
        // so the casing is an external contract, not a cosmetic detail.
        assert_eq!(NetworkProtocol::Dhcp.to_string(), "dhcp");
        assert_eq!(NetworkProtocol::Static.to_string(), "static");
    }
}
