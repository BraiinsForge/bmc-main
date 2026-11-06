// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
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
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use bmc_grpc::google::rpc::bad_request::FieldViolation;
use bmc_grpc::web::{
    EncryptionType as GrpcEncryptionType, NetworkConfig, NetworkConfigStatic, NetworkInfoResponse,
    NetworkService as GrpcNetworkService, ScanWifiResponse, SetWifiRequest,
    SignalStrength as GrpcSignalStrength, WifiNetwork as GrpcWifiNetwork, WifiNetwork,
    WifiSavedNetworksResponse, WifiStatus as GrpcWifiStatus, WifiStatusResponse,
    network_config::Protocol,
};
use bmc_intl::{Intl, format_message};
use bmc_net::{NetworkManager, WifiControl};
use bmc_net_types::wifi::{EncryptionType, SignalStrength, WifiJoinError};
use buffa::EnumValue;
use buffa_types::google::protobuf::Empty;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tracing::warn;

use super::intl::intl;
use super::shared::{
    StatusExt as _, ToStatus, bad_request, missing_value, require_state, violation,
};
use crate::initial_setup::{WifiJoins, WifiSetupError};
use crate::manager::{NetworkProtocolConfig, NetworkProtocolConfigStatic};
use crate::{
    BmcManager,
    manager::{BmcState, WifiNetworkConfig},
};

#[derive(Clone)]
pub(crate) struct NetworkService<T>
where
    T: BmcManager,
{
    manager: Arc<T>,
    wifi_joins: WifiJoins<T>,
}

impl<T> NetworkService<T>
where
    T: BmcManager,
{
    pub(crate) fn new(manager: Arc<T>, wifi_joins: WifiJoins<T>) -> Self {
        Self {
            manager,
            wifi_joins,
        }
    }

    async fn require_setup_finished(&self, intl: &Intl) -> Result<BmcState, ConnectError> {
        require_state(
            &*self.manager,
            // A device with no setup flow at all is as settled as a provisioned one.
            |state| match state {
                BmcState::Operational | BmcState::Unsupported => true,
                BmcState::FactoryDefault
                | BmcState::SetupPending
                | BmcState::WifiReconfiguration => false,
            },
            || {
                format_message!(
                    intl,
                    default_message: "This is only available once the device has finished its setup.",
                    description: "Error when a network setting is used before the device's first-time setup is done",
                )
            },
        )
        .await
    }
}

impl<T> GrpcNetworkService for NetworkService<T>
where
    T: BmcManager,
{
    async fn get_network_info(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<NetworkInfoResponse> {
        let net_man = self.manager.network_manager();
        let hostname = net_man.hostname().await.ok_or_else(|| {
            ConnectError::internal(format_message!(
                intl(&ctx),
                default_message: "Failed to get hostname!",
                description: "Error when the device could not read its own network name",
            ))
        })?;

        let ip_address = net_man
            .ip_address()
            .await
            .ok_or_else(|| {
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to get IP address!",
                    description: "Error when the device could not read its own IP address",
                ))
            })?
            .to_string();

        let mac_address = net_man.mac_address().ok_or_else(|| {
            ConnectError::internal(format_message!(
                intl(&ctx),
                default_message: "Failed to get MAC address!",
                description: "Error when the device could not read its own MAC address, the network card's hardware address",
            ))
        })?;

        Response::ok(NetworkInfoResponse {
            hostname,
            mac_address,
            ip_address,
        })
    }

    async fn get_network_config(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<NetworkConfig> {
        let network_config = self
            .manager
            .network_manager()
            .network_config()
            .await
            .ok_or_else(|| {
                warn!("Failed to get network config");
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to read the network settings!",
                    description: "Error when the device could not read its network settings",
                ))
            })?;

        Response::ok(into_network_config(&network_config))
    }

    async fn set_network_config(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, NetworkConfig>,
    ) -> ServiceResult<Empty> {
        let request = request.to_owned_message();
        let intl = intl(&ctx);

        let config = try_from_network_config(&intl, &request)?;

        self.manager
            .network_manager()
            .set_network_config(config)
            .await
            .map_err(|e| {
                warn!("Failed to set network config: {e}");
                ConnectError::internal(format_message!(
                    intl,
                    default_message: "Failed to apply the network settings!",
                    description: "Error when the device could not apply the network settings just saved",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn get_wifi_status(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WifiStatusResponse> {
        let intl = intl(&ctx);
        self.require_setup_finished(&intl).await?;

        let net_man = self.manager.network_manager();
        let wifi_data = require_wifi(&intl, net_man)?.status().await.map_err(|e| {
            warn!("Failed to get WiFi status: {}", e);
            failed_wifi_status(&intl)
        })?;

        let status = into_grpc_wifi_status(wifi_data.status);

        Response::ok(WifiStatusResponse {
            status: status.into(),
        })
    }

    async fn get_wifi_saved_networks(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WifiSavedNetworksResponse> {
        let intl = intl(&ctx);
        self.require_setup_finished(&intl).await?;

        let net_man = self.manager.network_manager();
        let wifi_data = require_wifi(&intl, net_man)?
            .saved_networks()
            .await
            .map_err(|e| {
                warn!("Failed to get WiFi status: {}", e);
                failed_wifi_status(&intl)
            })?
            .into_iter()
            .map(into_grpc_wifi_status)
            .collect();

        Response::ok(WifiSavedNetworksResponse { status: wifi_data })
    }

    async fn set_wifi(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetWifiRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        self.require_setup_finished(&intl).await?;

        let config = try_into_wifi_network_config(&intl, request.to_owned_message())?;
        require_wifi(&intl, self.manager.network_manager())?;

        let join = self.wifi_joins.join(config).or_status(&intl)?;
        join.await
            .map_err(|e| {
                warn!("Wi-Fi join task failed: {e}");
                failed_wifi_join(&intl)
            })?
            .map_err(WifiJoinFailure)
            .or_status(&intl)?;
        Response::ok(Empty::default())
    }

    async fn scan_wifi(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ScanWifiResponse> {
        let intl = intl(&ctx);
        self.require_setup_finished(&intl).await?;

        Response::ok(scan_wifi_response(&intl, self.manager.clone()).await?)
    }
}

fn failed_wifi_status(intl: &Intl) -> ConnectError {
    ConnectError::internal(format_message!(
        intl,
        default_message: "Failed to get Wi-Fi status!",
        description: "Error when the device could not read its Wi-Fi state or its saved Wi-Fi networks",
    ))
}

fn failed_wifi_join(intl: &Intl) -> ConnectError {
    ConnectError::internal(format_message!(
        intl,
        default_message: "Failed to join the Wi-Fi network!",
        description: "Error when connecting to a Wi-Fi network failed and the cause is neither a wrong password nor a missing network",
    ))
}

impl ToStatus for WifiSetupError {
    fn to_status(&self, intl: &Intl) -> ConnectError {
        match self {
            WifiSetupError::InProgress => ConnectError::failed_precondition(format_message!(
                intl,
                default_message: "The device is already connecting to a Wi-Fi network.",
                description: "Error when connecting to a Wi-Fi network while an earlier attempt has not finished",
            )),
        }
    }
}

/// A failed join, whose cause the Wi-Fi driver may have reported as a `WifiJoinError`.
pub(crate) struct WifiJoinFailure(pub(crate) anyhow::Error);

impl ToStatus for WifiJoinFailure {
    fn to_status(&self, intl: &Intl) -> ConnectError {
        let Self(error) = self;
        match error.downcast_ref::<WifiJoinError>() {
            Some(WifiJoinError::WrongKey(ssid)) => ConnectError::invalid_argument(format_message!(
                intl,
                default_message: "The password for {ssid} was rejected.",
                description: "Error when connecting to a Wi-Fi network with a wrong password; ssid is the network's name",
                values: { ssid: ssid.as_str() },
            )),
            Some(WifiJoinError::NetworkNotFound(ssid)) => {
                ConnectError::failed_precondition(format_message!(
                    intl,
                    default_message: "No Wi-Fi network named {ssid} was found.",
                    description: "Error when connecting to a Wi-Fi network that is not in range; ssid is the network's name",
                    values: { ssid: ssid.as_str() },
                ))
            }
            None => failed_wifi_join(intl),
        }
    }
}

/// Resolve the WiFi surface, mapping its absence to `unimplemented`: no WiFi
/// means the operation does not apply to this hardware, not a server fault.
fn require_wifi<'a>(
    intl: &Intl,
    net_man: &'a dyn NetworkManager,
) -> Result<&'a dyn WifiControl, ConnectError> {
    net_man.require_wifi().map_err(|_| {
        ConnectError::unimplemented(format_message!(
            intl,
            default_message: "This device does not support Wi-Fi.",
            description: "Error when a Wi-Fi operation is used on a device with no Wi-Fi hardware",
        ))
    })
}

pub(crate) fn into_network_config(config: &NetworkProtocolConfig) -> NetworkConfig {
    let protocol = match config {
        NetworkProtocolConfig::Dhcp => Protocol::Dhcp(Empty::default()),
        NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
            address,
            netmask,
            gateway,
            dns_servers,
        }) => Protocol::Static(NetworkConfigStatic {
            address: address.to_string(),
            gateway: gateway.to_string(),
            netmask: netmask.to_string(),
            dns_servers: dns_servers.iter().map(ToString::to_string).collect(),
        }),
    };
    NetworkConfig {
        protocol: Some(protocol),
    }
}

pub(crate) fn try_from_network_config(
    intl: &Intl,
    config: &NetworkConfig,
) -> Result<NetworkProtocolConfig, ConnectError> {
    parse_network_config(intl, config, "protocol", "").map_err(|v| bad_request(intl, v))
}

/// Parses a network config, naming a missing protocol `protocol_field` and
/// each violated static field `{static_prefix}<name>`, so a caller embedding
/// the config in a larger request reports paths the client can map back onto
/// its form.
pub(crate) fn parse_network_config(
    intl: &Intl,
    config: &NetworkConfig,
    protocol_field: &str,
    static_prefix: &str,
) -> Result<NetworkProtocolConfig, Vec<FieldViolation>> {
    fn parse_ipv4(intl: &Intl, field: &str, value: &str) -> Result<Ipv4Addr, FieldViolation> {
        if value.is_empty() {
            return Err(violation(field, missing_value(intl)));
        }

        value.parse::<Ipv4Addr>().map_err(|_| {
            violation(
                field,
                format_message!(
                    intl,
                    default_message: "{value} is not a valid IPv4.",
                    description: "Error under a network address field; value is what was typed",
                    values: { value: value },
                ),
            )
        })
    }

    let Some(protocol) = config.protocol.as_ref() else {
        let description = format_message!(
            intl,
            default_message: "Protocol must be specified.",
            description: "Error when neither automatic (DHCP) nor manual network settings are chosen",
        );
        return Err(vec![violation(protocol_field, description)]);
    };

    match protocol {
        Protocol::Dhcp(_) => Ok(NetworkProtocolConfig::Dhcp),
        Protocol::Static(static_config) => {
            let mut field_violations = vec![];

            macro_rules! parse_field {
                ($field:expr, $value:expr) => {
                    match parse_ipv4(intl, &format!("{static_prefix}{}", $field), $value) {
                        Ok(val) => Some(val),
                        Err(err) => {
                            field_violations.push(err);
                            None
                        }
                    }
                };
            }

            let address = parse_field!("address", &static_config.address);
            let netmask = parse_field!("netmask", &static_config.netmask);
            let gateway = parse_field!("gateway", &static_config.gateway);

            let dns_servers: Vec<Option<Ipv4Addr>> = static_config
                .dns_servers
                .iter()
                .enumerate()
                .map(|(i, dns)| parse_field!(format!("dns_servers[{i}]"), dns))
                .collect();

            if !field_violations.is_empty() {
                return Err(field_violations);
            }

            Ok(NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
                address: address.expect("BUG: address parsed without a recorded violation"),
                netmask: netmask.expect("BUG: netmask parsed without a recorded violation"),
                gateway: gateway.expect("BUG: gateway parsed without a recorded violation"),
                dns_servers: dns_servers
                    .into_iter()
                    .map(|dns| dns.expect("BUG: DNS server parsed without a recorded violation"))
                    .collect(),
            }))
        }
    }
}

pub(crate) async fn scan_wifi_response(
    intl: &Intl,
    manager: Arc<impl BmcManager>,
) -> Result<ScanWifiResponse, ConnectError> {
    let available_wifi = require_wifi(intl, manager.network_manager())?
        .scan()
        .await
        .map_err(|e| {
            warn!("Failed to scan WiFi networks: {}", e);
            ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to scan Wi-Fi networks!",
                description: "Error when the device could not list the Wi-Fi networks around it",
            ))
        })?;

    Ok(ScanWifiResponse {
        networks: available_wifi
            .into_iter()
            .filter(|wifi| wifi.signal_strength() != SignalStrength::Offline)
            .map(|wifi| WifiNetwork {
                signal_strength: into_grpc_signal_strength(wifi.signal_strength()).into(),
                ssid: wifi.ssid,
                encryption_type: into_encryption_type(wifi.encryption_type).into(),
            })
            .collect(),
    })
}

pub(crate) fn try_into_wifi_network_config(
    intl: &Intl,
    request: SetWifiRequest,
) -> Result<WifiNetworkConfig, ConnectError> {
    let encryption = try_into_encryption_type(intl, request.encryption_type)?;

    Ok(WifiNetworkConfig {
        ssid: request.ssid,
        password: request.password,
        encryption,
    })
}

pub(crate) fn try_into_encryption_type(
    intl: &Intl,
    value: EnumValue<GrpcEncryptionType>,
) -> Result<EncryptionType, ConnectError> {
    let unspecified = || {
        let description = format_message!(
            intl,
            default_message: "Choose an encryption type",
            description: "Error under the Wi-Fi encryption field when nothing is chosen",
        );
        bad_request(intl, vec![violation("encryption_type", description)])
    };
    let EnumValue::Known(value) = value else {
        return Err(unspecified());
    };
    match value {
        GrpcEncryptionType::Unspecified => Err(unspecified()),
        GrpcEncryptionType::None => Ok(EncryptionType::None),
        GrpcEncryptionType::Wep => Ok(EncryptionType::Wep),
        GrpcEncryptionType::WepShared => Ok(EncryptionType::WepShared),
        GrpcEncryptionType::Wpa => Ok(EncryptionType::Wpa),
        GrpcEncryptionType::Wpa12 => Ok(EncryptionType::Wpa1_2),
        GrpcEncryptionType::Wpa2 => Ok(EncryptionType::Wpa2),
        GrpcEncryptionType::Wpa23 => Ok(EncryptionType::Wpa2_3),
        GrpcEncryptionType::Wpa3 => Ok(EncryptionType::Wpa3),
    }
}

pub(crate) fn into_encryption_type(value: EncryptionType) -> GrpcEncryptionType {
    match value {
        EncryptionType::None => GrpcEncryptionType::None,
        EncryptionType::Wep => GrpcEncryptionType::Wep,
        EncryptionType::WepShared => GrpcEncryptionType::WepShared,
        EncryptionType::Wpa => GrpcEncryptionType::Wpa,
        EncryptionType::Wpa1_2 => GrpcEncryptionType::Wpa12,
        EncryptionType::Wpa2 => GrpcEncryptionType::Wpa2,
        EncryptionType::Wpa2_3 => GrpcEncryptionType::Wpa23,
        EncryptionType::Wpa3 => GrpcEncryptionType::Wpa3,
    }
}

pub(crate) fn into_grpc_signal_strength(value: SignalStrength) -> GrpcSignalStrength {
    match value {
        SignalStrength::Offline => GrpcSignalStrength::Unspecified,
        SignalStrength::Low => GrpcSignalStrength::Weak,
        SignalStrength::Fair => GrpcSignalStrength::Moderate,
        SignalStrength::Excellent => GrpcSignalStrength::Strong,
    }
}

pub(crate) fn into_grpc_wifi_status(value: bmc_net_types::wifi::WifiStatus) -> GrpcWifiStatus {
    GrpcWifiStatus {
        enabled: value.enabled,
        network: value
            .configuration
            .map(|network| GrpcWifiNetwork {
                ssid: network.ssid,
                encryption_type: into_encryption_type(network.encryption_type).into(),
                signal_strength: into_grpc_signal_strength(
                    value
                        .sta_link_state
                        .map_or(SignalStrength::Offline, |link| link.signal_strength()),
                )
                .into(),
            })
            .into(),
    }
}
