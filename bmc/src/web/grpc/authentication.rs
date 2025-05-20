// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::session::Manager as SessionManager;
use bmc_grpc::web::{self, LoginRequest, LoginResponse};
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub struct AuthenticationService<S>
where
    S: SessionManager,
{
    session_manager: Arc<S>,
}

impl<S> AuthenticationService<S>
where
    S: SessionManager,
{
    pub fn new(session_manager: Arc<S>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl<S> web::authentication_service_server::AuthenticationService for AuthenticationService<S>
where
    S: SessionManager,
{
    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let request = request.into_inner();
        self.session_manager
            .login(&request.password)
            .await
            .map(|cookie| {
                #[cfg(debug_assertions)]
                tracing::debug!("Session {} has been started", cookie.value());

                Response::new(LoginResponse {
                    token: cookie.value().to_owned(),
                    timeout_s: S::SESSION_TIMEOUT,
                })
            })
            .map_err(|e| Status::unauthenticated(e.to_string()))
    }
}
