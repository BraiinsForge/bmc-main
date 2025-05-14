// Copyright (C) 2025  Braiins Systems s.r.o.

use std::str::FromStr;
use std::sync::Arc;

use bmc_grpc::web::{
    GetTimezoneListResponse, GetTimezoneResponse, SetPasswordRequest, SetPasswordResponse,
    SetTimezoneRequest, system_service_server::SystemService as GrpcSystemService,
};
use tonic::{Code, Request};
use tonic_types::{ErrorDetails, StatusExt};
use tracing::warn;

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
    async fn set_password(
        &self,
        request: tonic::Request<SetPasswordRequest>,
    ) -> Result<tonic::Response<SetPasswordResponse>, tonic::Status> {
        let session = extract_session::<S>(request.extensions())?.clone();
        let request = request.into_inner();

        // Validate current password
        let _ = self
            .session_manager
            .login(&request.current_password)
            .await
            .map_err(|err| {
                tonic::Status::invalid_argument(format!(
                    "Cannot reset password due to invalid current password: {err}"
                ))
            })?;

        self.manager
            .set_password(request.new_password)
            .await
            .map_err(|err| tonic::Status::internal(format!("Failed to set password: {err}")))?;

        if let Err(err) = self.session_manager.logout_all_related(session).await {
            warn!("failed to logout all related sessions: {err}");
        }

        Ok(tonic::Response::new(SetPasswordResponse {}))
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
