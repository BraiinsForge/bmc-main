// Copyright (C) 2025  Braiins Systems s.r.o.
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
