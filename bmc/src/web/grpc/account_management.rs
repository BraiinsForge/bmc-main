// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use crate::web::grpc::GrpcError;
use crate::web::grpc::shared::{FieldViolations, ParseOutput, unchecked_field_violations_status};
use bmc_display::data::{Account, AccountId, AccountType, AuthenticationType, WidgetKind};
use bmc_grpc::web;
use prost_types::Timestamp;
use reqwest::StatusCode;
use std::panic;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tap::TapOptional;
use thiserror::Error;
use tokio::sync::RwLock;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use tracing::{error, warn};

const POOL_API_URL: &str = "https://api.braiins.com/pool/v2";
const POOL_API_ENDPOINT: &str = "/user/hashrate/current";
const API_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct AccountManagementService {
    config_handle: Arc<RwLock<ConfigHandle>>,
}

impl AccountManagementService {
    pub(crate) fn new(config_handle: Arc<RwLock<ConfigHandle>>) -> Self {
        Self { config_handle }
    }
}

#[async_trait::async_trait]
impl web::account_management_service_server::AccountManagementService for AccountManagementService {
    async fn add_account(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::AddAccountResponse>, Status> {
        let default_account_type = web::AccountType::Braiinspool;

        Ok(Response::new(web::AddAccountResponse {
            default_account_type: default_account_type.into(),
        }))
    }

    async fn remove_account(&self, request: Request<String>) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        let (id, field_violations) = parse_account_id("account_id", &request);

        if !field_violations.is_empty() {
            return Err(Status::with_error_details(
                Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            async move {
                let mut config = config_handle.write().await;
                let mut temp_config = config.clone();

                temp_config
                    .accounts
                    .remove(&id)
                    .ok_or_else(|| Status::not_found("Account not found"))?;

                if let Err(err) = temp_config.save().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn edit_account(
        &self,
        request: Request<web::EditAccountRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();

        let mut all_field_violations = FieldViolations::new();

        let (id, field_violations) = parse_account_id("account_id", &request.id);
        all_field_violations.extend(field_violations);

        let (account_name, field_violations) =
            parse_account_name("account_name", &request.account_name);
        all_field_violations.extend(field_violations);

        let (authentication_key, field_violations) =
            parse_authentication("authentication", request.authentication);
        all_field_violations.extend(field_violations);

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;
        let account_name = account_name.ok_or_else(unchecked_field_violations_status)?;
        let authentication_key =
            authentication_key.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();

            async move {
                let mut config = config_handle.write().await;
                let mut temp_config = config.clone();

                let account = temp_config
                    .accounts
                    .get_mut(&id)
                    .ok_or_else(|| Status::not_found("Account not found"))?;
                let authentication_validation = match account.r#type {
                    AccountType::BraiinsPool => {
                        validate_api_key(account.r#type, &authentication_key).await
                    }
                };
                if let Err(connect_app_error) = authentication_validation {
                    let description = match connect_app_error {
                        ConnectAppError::ServiceNotAvailable => "Service or internet unavailable",
                        ConnectAppError::ClientInitFailed => "Failed to initialize HTTP client",
                        ConnectAppError::InvalidApiKey => "Invalid API key",
                        #[expect(unused_variables)]
                        ConnectAppError::UnexpectedResponse(status) => {
                            "Unexpected response status: {status}"
                        }
                    };
                    return Err(Status::with_error_details(
                        Code::InvalidArgument,
                        GrpcError::BadRequest.to_string(),
                        ErrorDetails::with_bad_request_violation("authentication", description),
                    ));
                }

                let authentication_type = match account.r#type {
                    AccountType::BraiinsPool => AuthenticationType::ApiKey(authentication_key),
                };

                account.name = account_name;
                account.authentication = authentication_type;

                if let Err(err) = temp_config.save().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn get_all_accounts(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetAllAccountsResponse>, Status> {
        let config = self.config_handle.read().await;

        let accounts: Vec<web::Account> = config
            .accounts
            .clone()
            .into_values()
            .map(map_account_to_proto)
            .map(|mut account| {
                account.connected_widgets = config
                    .scenes
                    .values()
                    .flat_map(|scene| scene.widgets.values())
                    .filter_map(|w| match &w.kind {
                        WidgetKind::BraiinsPool(pool_widget)
                            if pool_widget
                                .account_id
                                .as_ref()
                                .is_some_and(|id| id.to_string() == account.id) =>
                        {
                            Some(w.id.to_string())
                        }
                        WidgetKind::BraiinsPool(_)
                        | WidgetKind::Clock(_)
                        | WidgetKind::TickerBtc(_)
                        | WidgetKind::BlockHeight(_)
                        | WidgetKind::RemoteImage(_)
                        | WidgetKind::BlockchainData
                        | WidgetKind::RemoteWidget(_)
                        | WidgetKind::HalvingCountdown => None,
                    })
                    .collect();
                account
            })
            .collect();

        Ok(Response::new(web::GetAllAccountsResponse { accounts }))
    }

    async fn connect_app(
        &self,
        request: Request<web::ConnectAppRequest>,
    ) -> Result<Response<String>, Status> {
        let request = request.into_inner();
        let mut all_field_violations = FieldViolations::new();

        let (account_type, field_violations) =
            parse_account_type("account_type", request.account_type());
        all_field_violations.extend(field_violations);

        let (account_name, field_violations) =
            parse_account_name("account_name", &request.account_name);
        all_field_violations.extend(field_violations);

        let (authentication_key, field_violations) =
            parse_authentication("authentication", request.authentication);
        all_field_violations.extend(field_violations);

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let account_type = account_type.ok_or_else(unchecked_field_violations_status)?;
        let account_name = account_name.ok_or_else(unchecked_field_violations_status)?;
        let authentication_key =
            authentication_key.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();

            async move {
                let authentication_validation = match account_type {
                    AccountType::BraiinsPool => {
                        validate_api_key(account_type, &authentication_key).await
                    }
                };
                if let Err(connect_app_error) = authentication_validation {
                    let description = match connect_app_error {
                        ConnectAppError::ServiceNotAvailable => "Service or internet unavailable",
                        ConnectAppError::ClientInitFailed => "Failed to initialize HTTP client",
                        ConnectAppError::InvalidApiKey => "Invalid API key",
                        #[expect(unused_variables)]
                        ConnectAppError::UnexpectedResponse(status) => {
                            "Unexpected response status: {status}"
                        }
                    };
                    return Err(Status::with_error_details(
                        Code::InvalidArgument,
                        GrpcError::BadRequest.to_string(),
                        ErrorDetails::with_bad_request_violation("authentication", description),
                    ));
                }

                let authentication_type = match account_type {
                    AccountType::BraiinsPool => AuthenticationType::ApiKey(authentication_key),
                };

                let new_account = Account::new(account_type, &account_name, authentication_type);

                let mut config = config_handle.write().await;
                let mut temp_config = config.clone();
                let new_account_id = new_account.id.clone();

                temp_config
                    .accounts
                    .insert(new_account_id.clone(), new_account);

                if let Err(err) = temp_config.save().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

                Ok(Response::new(new_account_id.to_string()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }
}

async fn validate_api_key(account_type: AccountType, api_key: &str) -> Result<(), ConnectAppError> {
    let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
        warn!("Cannot build reqwest::ClientBuilder");
        return Err(ConnectAppError::ClientInitFailed);
    };

    let status = match account_type {
        AccountType::BraiinsPool => {
            let endpoint = format!("{POOL_API_URL}{POOL_API_ENDPOINT}");
            match client
                .get(endpoint)
                .header("X-API-Key", api_key)
                .send()
                .await
            {
                Ok(response) => response.status(),
                Err(e) => {
                    warn!("Failed to get response from API endpoint: {e}");
                    return Err(ConnectAppError::ServiceNotAvailable);
                }
            }
        }
    };

    match status {
        StatusCode::OK => Ok(()),
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => Err(ConnectAppError::InvalidApiKey),
        status if status.is_server_error() => {
            warn!("API server error: {status}");
            Err(ConnectAppError::ServiceNotAvailable)
        }
        status => {
            warn!("Failed to verify API key with upstream service, status: {status}");
            Err(ConnectAppError::UnexpectedResponse(status))
        }
    }
}

fn map_account_to_proto(account: Account) -> web::Account {
    use web::AccountType as AccountTypeProto;

    let account_type = match account.r#type {
        AccountType::BraiinsPool => AccountTypeProto::Braiinspool,
    };

    let authentication = match account.authentication {
        AuthenticationType::ApiKey(api_key) => web::authentication::Value::ApiKey(api_key),
    };

    web::Account {
        id: account.id.to_string(),
        account_type: account_type.into(),
        account_name: account.name,
        authentication: Some(web::Authentication {
            value: Some(authentication),
        }),
        created_at: Some(Timestamp {
            seconds: account.created_at.timestamp(),
            nanos: 0,
        }),
        ..Default::default()
    }
}

fn parse_account_type(field: &str, input: web::AccountType) -> ParseOutput<AccountType> {
    use web::AccountType as AccountTypeProto;

    let mut field_violations = FieldViolations::new();

    let maybe_type = match input {
        AccountTypeProto::Unspecified => {
            field_violations.push(field, "Missing value!");
            None
        }
        AccountTypeProto::Braiinspool => Some(AccountType::BraiinsPool),
    };

    (maybe_type, field_violations)
}

fn parse_account_name(field: &str, input: &str) -> ParseOutput<String> {
    let mut field_violations = FieldViolations::new();

    let maybe_name = match input.len() {
        0 => {
            field_violations.push(field, "Missing value!");
            None
        }
        1..=50 => Some(input.to_owned()),
        _ => {
            field_violations.push(field, "Input too long!");
            None
        }
    };

    (maybe_name, field_violations)
}

fn parse_authentication(field: &str, input: Option<web::Authentication>) -> ParseOutput<String> {
    let mut field_violations = FieldViolations::new();

    let Some(input) = input else {
        field_violations.push(field, "Missing value!");
        return (None, field_violations);
    };

    let Some(value) = input.value else {
        field_violations.push(format!("{field}.value"), "Missing value!");
        return (None, field_violations);
    };

    let maybe_authentication = match value {
        web::authentication::Value::ApiKey(api_key) => {
            if api_key.is_empty() {
                field_violations.push("api_key", "Missing value!");
                None
            } else {
                Some(api_key.clone())
            }
        }
    };

    (maybe_authentication, field_violations)
}

fn parse_account_id(field: &str, input: &str) -> ParseOutput<AccountId> {
    let mut field_violations = FieldViolations::new();

    let maybe_id = AccountId::from_str(input).ok().tap_none(|| {
        field_violations.push(field, "Invalid account ID");
    });

    (maybe_id, field_violations)
}

#[derive(Debug, Clone, Error)]
pub enum ConnectAppError {
    #[error("Service unavailable")]
    ServiceNotAvailable,
    #[error("Invalid API key")]
    InvalidApiKey,
    #[error("Unexpected API response: {0}")]
    UnexpectedResponse(StatusCode),
    #[error("Failed to initialize HTTP client")]
    ClientInitFailed,
}
