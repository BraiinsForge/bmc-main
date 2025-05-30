// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{net::Ipv4Addr, str::FromStr};

use bmc_grpc::web::{
    ChangePasswordRequest, CreatePasswordRequest, GetTimezoneListResponse, GetTimezoneResponse,
    NetworkConfig, NetworkConfigStatic, NetworkInfoResponse, RemovePasswordRequest,
    SetTimezoneRequest, system_service_server::SystemService as GrpcSystemService,
};
use std::sync::Arc;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, FieldViolation, StatusExt};
use tracing::{error, warn};

use super::GrpcError;
use crate::{
    BmcManager,
    manager::{NetworkProtocolConfig, NetworkProtocolConfigStatic},
    session::Manager as SessionManager,
    time::Timezone,
    web::session::extract_session,
};

#[derive(Clone)]
pub(crate) struct SystemService<T, S>
where
    T: BmcManager,
    S: SessionManager,
{
    manager: Arc<T>,
    session_manager: Arc<S>,
}

impl<T, S> SystemService<T, S>
where
    T: BmcManager,
    S: SessionManager,
{
    pub(crate) fn new(manager: Arc<T>, session_manager: Arc<S>) -> Self {
        Self {
            manager,
            session_manager,
        }
    }
}

#[async_trait::async_trait]
impl<T, S> GrpcSystemService for SystemService<T, S>
where
    T: BmcManager,
    S: SessionManager,
{
    async fn has_password(&self, _request: Request<()>) -> Result<Response<bool>, Status> {
        let has_password = self.manager.has_password().await.map_err(|err| {
            error!(?err, "Failed to check password presence");
            Status::internal("Failed to check password presence")
        })?;

        Ok(Response::new(has_password))
    }

    async fn create_password(
        &self,
        request: Request<CreatePasswordRequest>,
    ) -> Result<Response<()>, Status> {
        let session = extract_session::<S>(request.extensions())?.clone();
        let request = request.into_inner();

        let has_password = self.manager.has_password().await.map_err(|err| {
            error!(?err, "Failed to check password presence");
            Status::internal("Failed to check password presence")
        })?;

        if has_password {
            return Err(Status::failed_precondition(
                "System already has password. You can change it using `change_password` call",
            ));
        }

        self.manager
            .set_password(Some(request.password))
            .await
            .map_err(|err| {
                error!(?err, "Failed to set password");
                Status::internal("Failed to set password")
            })?;

        if let Err(err) = self.session_manager.logout_all_related(session).await {
            warn!("Failed to logout all related sessions: {err}");
        }

        Ok(Response::new(()))
    }

    async fn change_password(
        &self,
        request: Request<ChangePasswordRequest>,
    ) -> Result<Response<()>, Status> {
        let session = extract_session::<S>(request.extensions())?.clone();
        let request = request.into_inner();

        let is_current_password_correct = self
            .manager
            .check_password(Some(&request.current_password))
            .await
            .map_err(|err| {
                error!(?err, "Failed to check current password");
                Status::internal("Failed to check current password")
            })?;

        if !is_current_password_correct {
            return Err(Status::with_error_details(
                Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request_violation(
                    "current_password",
                    "Incorrect current password",
                ),
            ));
        }

        self.manager
            .set_password(Some(request.new_password))
            .await
            .map_err(|err| {
                error!(?err, "Failed to set password");
                Status::internal("Failed to set password")
            })?;

        if let Err(err) = self.session_manager.logout_all_related(session).await {
            warn!("Failed to logout all related sessions: {err}");
        }

        Ok(Response::new(()))
    }

    async fn remove_password(
        &self,
        request: Request<RemovePasswordRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();

        let is_current_password_correct = self
            .manager
            .check_password(Some(&request.password))
            .await
            .map_err(|err| {
                error!(?err, "Failed to check current password");
                Status::internal("Failed to check current password")
            })?;

        if !is_current_password_correct {
            return Err(Status::with_error_details(
                Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request_violation("password", "Incorrect current password"),
            ));
        }

        self.manager.set_password(None).await.map_err(|err| {
            error!(?err, "Failed to set password");
            Status::internal("Failed to set password")
        })?;

        Ok(Response::new(()))
    }

    async fn get_timezone(
        &self,
        _request: Request<()>,
    ) -> Result<tonic::Response<GetTimezoneResponse>, tonic::Status> {
        Ok(tonic::Response::new(GetTimezoneResponse {
            timezone: Some(into_grpc_timezone(&self.manager.timezone())),
        }))
    }
    async fn get_timezone_list(
        &self,
        _request: Request<()>,
    ) -> Result<Response<GetTimezoneListResponse>, Status> {
        let timezones = self
            .manager
            .timezone_list()
            .map(|tz| into_grpc_timezone(&tz))
            .collect();
        Ok(Response::new(GetTimezoneListResponse { timezones }))
    }

    async fn set_timezone(
        &self,
        request: Request<SetTimezoneRequest>,
    ) -> Result<Response<()>, Status> {
        let value = request.into_inner().id;
        let timezone = Timezone::from_str(&value).map_err(|_| {
            Status::with_error_details(
                Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request_violation("timezone", "invalid timezone variant"),
            )
        })?;

        self.manager.set_timezone(timezone).await.map_err(|e| {
            warn!("Failed to set timezone: {}", e);
            Status::internal("Unexpected error occured when settign timezone")
        })?;

        Ok(Response::new(()))
    }

    async fn factory_reset(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        // NOTE: this API for now supports only soft-reset
        self.manager.factory_reset(false).await.map_err(|err| {
            warn!(?err, "Failed to apply factory settings");
            Status::internal("Failed to apply factory settings")
        })?;

        Ok(Response::new(()))
    }

    async fn get_network_info(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<NetworkInfoResponse>, tonic::Status> {
        let hostname = self
            .manager
            .hostname()
            .await
            .ok_or(tonic::Status::internal("Failed to get hostname"))?;

        let ip_address = self
            .manager
            .ip_address()
            .ok_or(Status::internal("Failed to get ip address"))?
            .to_string();

        let mac_address = self
            .manager
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
        let network_config = self.manager.network_config().await.ok_or_else(|| {
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

        self.manager.set_network_config(config).await.map_err(|e| {
            warn!("Failed to set network config: {e}");
            Status::internal("Failed to set network config")
        })?;

        Ok(Response::new(()))
    }
}

fn into_grpc_timezone(timezone: &Timezone) -> bmc_grpc::web::Timezone {
    bmc_grpc::web::Timezone {
        id: timezone.normalize_iana(),
        label: timezone.iana.to_owned(),
        offset: timezone
            .current_timezone_offset()
            .map(|offset| offset.to_string())
            .unwrap_or_default(),
    }
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
