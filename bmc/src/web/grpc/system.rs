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

use std::str::FromStr;

use bmc_grpc::web::{
    ChangePasswordRequest, CreatePasswordRequest, GetTimezoneListResponse, GetTimezoneResponse,
    RemovePasswordRequest, SetTimezoneRequest, SystemService as GrpcSystemService,
};
use bmc_shared_time::time::Timezone;
use buffa_types::google::protobuf::{BoolValue, Empty};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::sync::Arc;
use tracing::{error, warn};

use super::shared::invalid_field;
use crate::{BmcManager, session::Manager as SessionManager, web::session::extract_session};

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

impl<T, S> GrpcSystemService for SystemService<T, S>
where
    T: BmcManager,
    S: SessionManager,
{
    async fn has_password(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<BoolValue> {
        let has_password = self.manager.has_password().await.map_err(|err| {
            error!(?err, "Failed to check password presence");
            ConnectError::internal("Failed to check password presence")
        })?;

        Response::ok(BoolValue {
            value: has_password,
            ..Default::default()
        })
    }

    async fn create_password(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePasswordRequest>,
    ) -> ServiceResult<Empty> {
        let session = extract_session::<S>(ctx.extensions())?.clone();

        let has_password = self.manager.has_password().await.map_err(|err| {
            error!(?err, "Failed to check password presence");
            ConnectError::internal("Failed to check password presence")
        })?;

        if has_password {
            return Err(ConnectError::failed_precondition(
                "System already has password. You can change it using `change_password` call",
            ));
        }

        self.manager
            .set_password(Some(request.password.to_owned()))
            .await
            .map_err(|err| {
                error!(?err, "Failed to set password");
                ConnectError::internal("Failed to set password")
            })?;

        if let Err(err) = self.session_manager.logout_all_related(session).await {
            warn!("Failed to logout all related sessions: {err}");
        }

        Response::ok(Empty::default())
    }

    async fn change_password(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ChangePasswordRequest>,
    ) -> ServiceResult<Empty> {
        let session = extract_session::<S>(ctx.extensions())?.clone();

        let is_current_password_correct = self
            .manager
            .check_password(Some(request.current_password))
            .await
            .map_err(|err| {
                error!(?err, "Failed to check current password");
                ConnectError::internal("Failed to check current password")
            })?;

        if !is_current_password_correct {
            return Err(invalid_field(
                "current_password",
                "Incorrect current password",
            ));
        }

        self.manager
            .set_password(Some(request.new_password.to_owned()))
            .await
            .map_err(|err| {
                error!(?err, "Failed to set password");
                ConnectError::internal("Failed to set password")
            })?;

        if let Err(err) = self.session_manager.logout_all_related(session).await {
            warn!("Failed to logout all related sessions: {err}");
        }

        Response::ok(Empty::default())
    }

    async fn remove_password(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, RemovePasswordRequest>,
    ) -> ServiceResult<Empty> {
        let is_current_password_correct = self
            .manager
            .check_password(Some(request.password))
            .await
            .map_err(|err| {
                error!(?err, "Failed to check current password");
                ConnectError::internal("Failed to check current password")
            })?;

        if !is_current_password_correct {
            return Err(invalid_field("password", "Incorrect current password"));
        }

        self.manager.set_password(None).await.map_err(|err| {
            error!(?err, "Failed to set password");
            ConnectError::internal("Failed to set password")
        })?;

        Response::ok(Empty::default())
    }

    async fn get_timezone(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetTimezoneResponse> {
        Response::ok(GetTimezoneResponse {
            timezone: into_grpc_timezone(&self.manager.timezone()).into(),
        })
    }

    async fn get_timezone_list(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetTimezoneListResponse> {
        let timezones = Timezone::list().iter().map(into_grpc_timezone).collect();
        Response::ok(GetTimezoneListResponse { timezones })
    }

    async fn set_timezone(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetTimezoneRequest>,
    ) -> ServiceResult<Empty> {
        let timezone = Timezone::from_str(request.id)
            .map_err(|_| invalid_field("timezone", "invalid timezone variant"))?;

        self.manager.set_timezone(timezone).await.map_err(|e| {
            warn!("Failed to set timezone: {}", e);
            ConnectError::internal("Unexpected error occured when settign timezone")
        })?;

        Response::ok(Empty::default())
    }

    async fn factory_reset(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        // NOTE: this API for now supports only soft-reset
        self.manager.factory_reset(false).await.map_err(|err| {
            warn!(?err, "Failed to apply factory settings");
            ConnectError::internal("Failed to apply factory settings")
        })?;

        Response::ok(Empty::default())
    }

    async fn reboot(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.manager.reboot().await.map_err(|err| {
            warn!(?err, "Failed to reboot device");
            ConnectError::internal("Failed to reboot device")
        })?;

        Response::ok(Empty::default())
    }
}

pub(crate) fn into_grpc_timezone(timezone: &Timezone) -> bmc_grpc::web::Timezone {
    bmc_grpc::web::Timezone {
        id: timezone.iana().to_owned(),
        label: timezone.iana().to_owned(),
        offset: timezone.offset().to_string(),
    }
}
