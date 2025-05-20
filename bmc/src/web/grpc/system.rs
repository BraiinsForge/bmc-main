// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_grpc::web::{
    SetPasswordRequest, SetPasswordResponse,
    system_service_server::SystemService as GrpcSystemService,
};
use tracing::warn;

use crate::{BmcManager, session::Manager as SessionManager, web::session::extract_session};

#[derive(Clone)]
pub(crate) struct SystemService<T, S>
where
    T: BmcManager,
    S: SessionManager,
{
    bmc_manager: Arc<T>,
    session_manager: Arc<S>,
}

impl<T, S> SystemService<T, S>
where
    T: BmcManager,
    S: SessionManager,
{
    pub(crate) fn new(bmc_manager: Arc<T>, session_manager: Arc<S>) -> Self {
        Self {
            bmc_manager,
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
        let session = extract_session::<S, _>(&request)?.clone();
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

        self.bmc_manager
            .set_password(request.new_password)
            .await
            .map_err(|err| tonic::Status::internal(format!("Failed to set password: {err}")))?;

        if let Err(err) = self.session_manager.logout_all_related(session).await {
            warn!("failed to logout all related sessions: {err}");
        }

        Ok(tonic::Response::new(SetPasswordResponse {}))
    }
}
