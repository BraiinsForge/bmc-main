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
use bmc_net::{NetworkManager, WifiControl};
use bmc_net_types::wifi::{EncryptionType, SignalStrength};
use buffa::EnumValue;
use buffa_types::google::protobuf::Empty;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tracing::warn;

use super::GrpcError;
use super::shared::{invalid_field, violation, with_violations};
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

    async fn check_precondition(&self, state: BmcState) -> Result<(), ConnectError> {
        let current_state = self
            .manager
            .network_manager()
            .provisioning()
            .device_state()
            .await;
        // A device with no setup flow at all is as settled as a provisioned one
        // for the network calls, so `Unsupported` satisfies `Operational`.
        let settled = state == BmcState::Operational && current_state == BmcState::Unsupported;
        if current_state != state && !settled {
            return Err(ConnectError::failed_precondition(format!(
                "Function is only available when the device is in '{state}' state. Current state is '{current_state}'.",
            )));
        }
        Ok(())
    }
}

impl<T> GrpcNetworkService for NetworkService<T>
where
    T: BmcManager,
{
    async fn get_network_info(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<NetworkInfoResponse> {
        let net_man = self.manager.network_manager();
        let hostname = net_man
            .hostname()
            .await
            .ok_or(ConnectError::internal("Failed to get hostname"))?;

        let ip_address = net_man
            .ip_address()
            .await
            .ok_or(ConnectError::internal("Failed to get ip address"))?
            .to_string();

        let mac_address = net_man
            .mac_address()
            .ok_or(ConnectError::internal("Failed to get mac address"))?;

        Response::ok(NetworkInfoResponse {
            hostname,
            mac_address,
            ip_address,
        })
    }

    async fn get_network_config(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<NetworkConfig> {
        let network_config = self
            .manager
            .network_manager()
            .network_config()
            .await
            .ok_or_else(|| {
                warn!("Failed to get network config");
                ConnectError::internal("Failed to get network config")
            })?;

        Response::ok(into_network_config(&network_config))
    }

    async fn set_network_config(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, NetworkConfig>,
    ) -> ServiceResult<Empty> {
        let request = request.to_owned_message();

        let config = try_from_network_config(&request)?;

        self.manager
            .network_manager()
            .set_network_config(config)
            .await
            .map_err(|e| {
                warn!("Failed to set network config: {e}");
                ConnectError::internal("Failed to set network config")
            })?;

        Response::ok(Empty::default())
    }

    async fn get_wifi_status(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WifiStatusResponse> {
        self.check_precondition(BmcState::Operational).await?;

        let net_man = self.manager.network_manager();
        let wifi_data = require_wifi(net_man)?.status().await.map_err(|e| {
            warn!("Failed to get WiFi status: {}", e);
            ConnectError::internal("Failed to get Wi-Fi status")
        })?;

        let status = into_grpc_wifi_status(wifi_data.status);

        Response::ok(WifiStatusResponse {
            status: status.into(),
        })
    }

    async fn get_wifi_saved_networks(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WifiSavedNetworksResponse> {
        self.check_precondition(BmcState::Operational).await?;

        let net_man = self.manager.network_manager();
        let wifi_data = require_wifi(net_man)?
            .saved_networks()
            .await
            .map_err(|e| {
                warn!("Failed to get WiFi status: {}", e);
                ConnectError::internal("Failed to get Wi-Fi status")
            })?
            .into_iter()
            .map(into_grpc_wifi_status)
            .collect();

        Response::ok(WifiSavedNetworksResponse { status: wifi_data })
    }

    async fn set_wifi(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetWifiRequest>,
    ) -> ServiceResult<Empty> {
        self.check_precondition(BmcState::Operational).await?;

        let config = try_into_wifi_network_config(request.to_owned_message())?;
        require_wifi(self.manager.network_manager())?;

        let join = self.wifi_joins.join(config).map_err(|e| match e {
            WifiSetupError::InProgress => {
                ConnectError::failed_precondition("A Wi-Fi join is already in progress")
            }
        })?;
        join.await
            .map_err(|e| ConnectError::internal(format!("Wi-Fi join task failed: {e}")))?
            .map_err(|e| ConnectError::internal(e.to_string()))?;
        Response::ok(Empty::default())
    }

    async fn scan_wifi(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ScanWifiResponse> {
        self.check_precondition(BmcState::Operational).await?;

        Response::ok(scan_wifi_response(self.manager.clone()).await?)
    }
}

/// Resolve the WiFi surface, mapping its absence to `unimplemented`: no WiFi
/// means the operation does not apply to this hardware, not a server fault.
fn require_wifi(net_man: &dyn NetworkManager) -> Result<&dyn WifiControl, ConnectError> {
    net_man
        .require_wifi()
        .map_err(|e| ConnectError::unimplemented(e.to_string()))
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
    config: &NetworkConfig,
) -> Result<NetworkProtocolConfig, ConnectError> {
    parse_network_config(config, "protocol", "").map_err(bad_request)
}

/// One `InvalidArgument` carrying every field violation collected so far.
pub(crate) fn bad_request(field_violations: Vec<FieldViolation>) -> ConnectError {
    with_violations(
        ConnectError::invalid_argument(GrpcError::BadRequest.to_string()),
        field_violations,
    )
}

/// Parses a network config, naming a missing protocol `protocol_field` and
/// each violated static field `{static_prefix}<name>`, so a caller embedding
/// the config in a larger request reports paths the client can map back onto
/// its form.
pub(crate) fn parse_network_config(
    config: &NetworkConfig,
    protocol_field: &str,
    static_prefix: &str,
) -> Result<NetworkProtocolConfig, Vec<FieldViolation>> {
    fn parse_ipv4(field: &str, value: &str) -> Result<Ipv4Addr, FieldViolation> {
        if value.is_empty() {
            return Err(violation(field, "Missing value!"));
        }

        value
            .parse::<Ipv4Addr>()
            .map_err(|_| violation(field, format!("'{value}' is not a valid IPv4!")))
    }

    let Some(protocol) = config.protocol.as_ref() else {
        return Err(vec![violation(
            protocol_field,
            "Protocol must be specified!",
        )]);
    };

    match protocol {
        Protocol::Dhcp(_) => Ok(NetworkProtocolConfig::Dhcp),
        Protocol::Static(static_config) => {
            let mut field_violations = vec![];

            macro_rules! parse_field {
                ($field:expr, $value:expr) => {
                    match parse_ipv4(&format!("{static_prefix}{}", $field), $value) {
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
    manager: Arc<impl BmcManager>,
) -> Result<ScanWifiResponse, ConnectError> {
    let available_wifi = require_wifi(manager.network_manager())?
        .scan()
        .await
        .map_err(|e| {
            warn!("Failed to scan WiFi networks: {}", e);
            ConnectError::internal("Failed to scan Wi-Fi networks")
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
    request: SetWifiRequest,
) -> Result<WifiNetworkConfig, ConnectError> {
    let encryption = try_into_encryption_type(request.encryption_type)?;

    Ok(WifiNetworkConfig {
        ssid: request.ssid,
        password: request.password,
        encryption,
    })
}

pub(crate) fn try_into_encryption_type(
    value: EnumValue<GrpcEncryptionType>,
) -> Result<EncryptionType, ConnectError> {
    let unspecified = || {
        invalid_field(
            "encryption_type",
            "Encryption type must be specified and cannot be UNSPECIFIED.",
        )
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
        SignalStrength::Offline => GrpcSignalStrength::Unspecified, // Offline WiFi is filtered out
        SignalStrength::Low => GrpcSignalStrength::Weak,
        SignalStrength::Fair => GrpcSignalStrength::Moderate,
        SignalStrength::Excellent => GrpcSignalStrength::Strong,
    }
}

pub(crate) fn into_grpc_wifi_status(value: bmc_net_types::wifi::WifiStatus) -> GrpcWifiStatus {
    let network = value.configuration.unwrap_or_default();
    GrpcWifiStatus {
        enabled: value.enabled,
        network: GrpcWifiNetwork {
            ssid: network.ssid,
            encryption_type: (network.encryption_type as i32).into(),
            signal_strength: value.sta_link_state.unwrap_or_default().signal_level.into(),
        }
        .into(),
    }
}
