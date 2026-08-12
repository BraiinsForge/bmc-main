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

use bmc_grpc::web::{
    EncryptionType as GrpcEncryptionType, NetworkConfig, NetworkConfigStatic, NetworkInfoResponse,
    ScanWifiResponse, SetWifiRequest, SignalStrength as GrpcSignalStrength,
    WifiNetwork as GrpcWifiNetwork, WifiNetwork, WifiSavedNetworksResponse,
    WifiStatus as GrpcWifiStatus, WifiStatusResponse,
    network_service_server::NetworkService as GrpcNetworkService,
};
use bmc_net::{NetworkManager, WifiControl};
use bmc_net_types::wifi::{EncryptionType, SignalStrength};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, FieldViolation, StatusExt};
use tracing::warn;

use super::GrpcError;
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
}

impl<T> NetworkService<T>
where
    T: BmcManager,
{
    pub(crate) fn new(manager: Arc<T>) -> Self {
        Self { manager }
    }

    async fn check_precondition(&self, state: BmcState) -> Result<(), Status> {
        let current_state = self
            .manager
            .network_manager()
            .provisioning()
            .device_state()
            .await;
        if current_state != state {
            return Err(Status::failed_precondition(format!(
                "Function is only available when the device is in '{state}' state. Current state is '{current_state}'.",
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<T> GrpcNetworkService for NetworkService<T>
where
    T: BmcManager,
{
    async fn get_network_info(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<NetworkInfoResponse>, tonic::Status> {
        let net_man = self.manager.network_manager();
        let hostname = net_man
            .hostname()
            .await
            .ok_or(tonic::Status::internal("Failed to get hostname"))?;

        let ip_address = net_man
            .ip_address()
            .await
            .ok_or(Status::internal("Failed to get ip address"))?
            .to_string();

        let mac_address = net_man
            .mac_address()
            .ok_or(Status::internal("Failed to get mac address"))?;

        Ok(Response::new(NetworkInfoResponse {
            hostname,
            mac_address,
            ip_address,
        }))
    }

    async fn get_network_config(
        &self,
        _request: Request<()>,
    ) -> Result<Response<NetworkConfig>, Status> {
        let network_config = self
            .manager
            .network_manager()
            .network_config()
            .await
            .ok_or_else(|| {
                warn!("Failed to get network config");
                Status::internal("Failed to get network config")
            })?;

        Ok(Response::new(into_network_config(&network_config)))
    }

    async fn set_network_config(
        &self,
        request: Request<NetworkConfig>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();

        let config = try_from_network_config(&request)?;

        self.manager
            .network_manager()
            .set_network_config(config)
            .await
            .map_err(|e| {
                warn!("Failed to set network config: {e}");
                Status::internal("Failed to set network config")
            })?;

        Ok(Response::new(()))
    }
    async fn get_wifi_status(
        &self,
        _request: Request<()>,
    ) -> Result<Response<WifiStatusResponse>, Status> {
        self.check_precondition(BmcState::Operational).await?;

        let net_man = self.manager.network_manager();
        let wifi_data = require_wifi(net_man)?.status().await.map_err(|e| {
            warn!("Failed to get WiFi status: {}", e);
            Status::internal("Failed to get WiFi status")
        })?;

        let status = into_grpc_wifi_status(wifi_data.status);

        Ok(Response::new(WifiStatusResponse {
            status: Some(status),
        }))
    }

    async fn get_wifi_saved_networks(
        &self,
        _request: Request<()>,
    ) -> Result<Response<WifiSavedNetworksResponse>, Status> {
        self.check_precondition(BmcState::Operational).await?;

        let net_man = self.manager.network_manager();
        let wifi_data = require_wifi(net_man)?
            .saved_networks()
            .await
            .map_err(|e| {
                warn!("Failed to get WiFi status: {}", e);
                Status::internal("Failed to get WiFi status")
            })?
            .into_iter()
            .map(into_grpc_wifi_status)
            .collect();

        Ok(Response::new(WifiSavedNetworksResponse {
            status: wifi_data,
        }))
    }

    async fn set_wifi(&self, request: Request<SetWifiRequest>) -> Result<Response<()>, Status> {
        self.check_precondition(BmcState::Operational).await?;

        let request = request.into_inner();

        let config = try_into_wifi_network_config(request)?;

        let net_man = self.manager.network_manager();
        match require_wifi(net_man)?
            .wifi_save_and_connect(config.ssid, config.password, config.encryption)
            .await
        {
            Ok(()) => Ok(Response::new(())),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn scan_wifi(&self, _request: Request<()>) -> Result<Response<ScanWifiResponse>, Status> {
        self.check_precondition(BmcState::Operational).await?;

        Ok(Response::new(
            scan_wifi_response(self.manager.clone()).await?,
        ))
    }
}

/// Resolve the WiFi surface, mapping its absence to `unimplemented`: no WiFi
/// means the operation does not apply to this hardware, not a server fault.
fn require_wifi(net_man: &dyn NetworkManager) -> Result<&dyn WifiControl, Status> {
    net_man
        .require_wifi()
        .map_err(|e| Status::unimplemented(e.to_string()))
}

fn into_network_config(config: &NetworkProtocolConfig) -> NetworkConfig {
    match config {
        NetworkProtocolConfig::Dhcp => NetworkConfig {
            protocol: Some(bmc_grpc::web::network_config::Protocol::Dhcp(())),
        },
        NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
            address,
            netmask,
            gateway,
            dns_servers,
        }) => NetworkConfig {
            protocol: Some(bmc_grpc::web::network_config::Protocol::Static(
                NetworkConfigStatic {
                    address: address.to_string(),
                    gateway: gateway.to_string(),
                    netmask: netmask.to_string(),
                    dns_servers: dns_servers.iter().map(ToString::to_string).collect(),
                },
            )),
        },
    }
}

fn try_from_network_config(config: &NetworkConfig) -> Result<NetworkProtocolConfig, Status> {
    fn parse_ipv4(field: &str, value: &str) -> Result<Ipv4Addr, FieldViolation> {
        if value.is_empty() {
            return Err(FieldViolation::new(field, "Missing value!"));
        }

        value
            .parse::<Ipv4Addr>()
            .map_err(|_| FieldViolation::new(field, format!("'{value}' is not a valid IPv4!")))
    }

    let protocol = config
        .protocol
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("Protocol must be specified!"))?;

    let protocol = match protocol {
        bmc_grpc::web::network_config::Protocol::Dhcp(()) => NetworkProtocolConfig::Dhcp,
        bmc_grpc::web::network_config::Protocol::Static(static_config) => {
            let mut field_violations = vec![];

            macro_rules! parse_field {
                ($field:expr, $value:expr) => {
                    match parse_ipv4($field, $value) {
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
                .map(|(i, dns)| parse_field!(&format!("dns_servers[{i}]"), dns))
                .collect();

            if !field_violations.is_empty() {
                return Err(Status::with_error_details(
                    Code::InvalidArgument,
                    GrpcError::BadRequest.to_string(),
                    ErrorDetails::with_bad_request(field_violations),
                ));
            }

            NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
                address: address.ok_or_else(|| Status::invalid_argument("Invalid address!"))?,
                netmask: netmask.ok_or_else(|| Status::invalid_argument("Invalid netmask!"))?,
                gateway: gateway.ok_or_else(|| Status::invalid_argument("Invalid gateway!"))?,
                dns_servers: dns_servers
                    .iter()
                    .map(|dns| dns.ok_or_else(|| Status::invalid_argument("Invalid DNS!")))
                    .collect::<Result<Vec<_>, _>>()?,
            })
        }
    };

    Ok(protocol)
}

pub(crate) async fn scan_wifi_response(
    manager: Arc<impl BmcManager>,
) -> Result<ScanWifiResponse, Status> {
    let available_wifi = require_wifi(manager.network_manager())?
        .scan()
        .await
        .map_err(|e| {
            warn!("Failed to scan WiFi networks: {}", e);
            Status::internal("Failed to scan WiFi networks")
        })?;

    Ok(ScanWifiResponse {
        networks: available_wifi
            .into_iter()
            .filter(|wifi| wifi.signal_strength() != SignalStrength::Offline)
            .map(|wifi| WifiNetwork {
                signal_strength: into_grpc_signal_strength(wifi.signal_strength()) as i32,
                ssid: wifi.ssid,
                encryption_type: into_encryption_type(wifi.encryption_type) as i32,
            })
            .collect(),
    })
}

pub(crate) fn try_into_wifi_network_config(
    request: SetWifiRequest,
) -> Result<WifiNetworkConfig, Status> {
    let encryption = try_into_encryption_type(request.encryption_type())?;

    Ok(WifiNetworkConfig {
        ssid: request.ssid,
        password: request.password,
        encryption,
    })
}

pub(crate) fn try_into_encryption_type(
    value: GrpcEncryptionType,
) -> Result<EncryptionType, Status> {
    match value {
        GrpcEncryptionType::Unspecified => Err(Status::with_error_details(
            Code::InvalidArgument,
            GrpcError::BadRequest.to_string(),
            ErrorDetails::with_bad_request_violation(
                "encryption_type",
                "Encryption type must be specified and cannot be UNSPECIFIED.",
            ),
        )),
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
        network: Some(GrpcWifiNetwork {
            ssid: network.ssid,
            encryption_type: network.encryption_type as i32,
            signal_strength: value.sta_link_state.unwrap_or_default().signal_level,
        }),
    }
}
