// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::{
    ChangePasswordRequest, CreatePasswordRequest, GetTimezoneListResponse, GetTimezoneResponse,
    RemovePasswordRequest, SetTimezoneRequest,
    system_service_server::SystemService as GrpcSystemService,
};
use std::str::FromStr;
use std::sync::Arc;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use tracing::{error, warn};

use super::GrpcError;
use crate::{
    BmcManager, session::Manager as SessionManager, time::Timezone, web::session::extract_session,
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
    ) -> Result<tonic::Response<GetTimezoneListResponse>, tonic::Status> {
        let timezones = self
            .manager
            .timezone_list()
            .map(|tz| into_grpc_timezone(&tz))
            .collect();
        Ok(tonic::Response::new(GetTimezoneListResponse { timezones }))
    }

    async fn set_timezone(
        &self,
        request: Request<SetTimezoneRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let value = request.into_inner().id;
        let timezone = Timezone::from_str(&value).map_err(|_| {
            tonic::Status::with_error_details(
                Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request_violation("timezone", "invalid timezone variant"),
            )
        })?;

        self.manager.set_timezone(timezone).await.map_err(|e| {
            warn!("Failed to set timezone: {}", e);
            tonic::Status::internal("Unexpected error occured when settign timezone")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn factory_reset(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        // NOTE: this API for now supports only soft-reset
        self.manager.factory_reset(false).await.map_err(|err| {
            warn!(?err, "Failed to apply factory settings");
            Status::internal("Failed to apply factory settings")
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
