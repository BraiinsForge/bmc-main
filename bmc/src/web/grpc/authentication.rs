// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::session::Manager as SessionManager;
use crate::web::session::{extract_cookies, extract_session};
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
    async fn is_authenticated(&self, request: Request<()>) -> Result<Response<bool>, Status> {
        Ok(Response::new(
            extract_session::<S>(request.extensions()).is_ok(),
        ))
    }

    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let cookies = extract_cookies(request.extensions());
        let request = request.get_ref();
        let cookie = self
            .session_manager
            .login(&request.password)
            .await
            .map_err(|e| Status::unauthenticated(e.to_string()))?;

        #[cfg(debug_assertions)]
        tracing::debug!("Session {} has been started", cookie.value());

        cookies.add(cookie.clone());

        Ok(Response::new(LoginResponse {
            token: cookie.value().to_owned(),
            timeout_s: S::SESSION_TIMEOUT,
        }))
    }

    async fn logout(&self, request: Request<()>) -> Result<Response<()>, Status> {
        let cookies = extract_cookies(request.extensions());
        let session = extract_session::<S>(request.extensions())?;

        let cookie = self
            .session_manager
            .logout(session.clone())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        cookies.remove(cookie);

        Ok(Response::new(()))
    }
}
