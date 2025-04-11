// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::web::session;
use anyhow::Result;
use bmc_grpc::web::{self, LoginRequest, LoginResponse};
use hyper::http::header;
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub struct AuthenticationService<S>
where
    S: session::Manager,
{
    session_manager: Arc<S>,
}

impl<S> AuthenticationService<S>
where
    S: session::Manager,
{
    pub fn new(session_manager: Arc<S>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl<S> web::authentication_service_server::AuthenticationService for AuthenticationService<S>
where
    S: session::Manager,
{
    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, tonic::Status> {
        let username = request.get_ref().username.clone();
        let password = request.get_ref().password.clone();
        self.session_manager
            .login(username, password)
            .await
            .map(|cookie| {
                let mut response = Response::new(LoginResponse {
                    token: cookie.value().to_owned(),
                    timeout_s: S::SESSION_TIMEOUT,
                });

                #[cfg(debug_assertions)]
                tracing::debug!(
                    "Session {} has been started for user {}",
                    cookie.value(),
                    request.get_ref().username
                );

                response
            })
            .map_err(|e| Status::unauthenticated(e.to_string()))
    }
}
