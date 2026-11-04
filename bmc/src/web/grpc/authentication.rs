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

use super::intl::intl;
use crate::session::{LoginError, Manager as SessionManager};
use crate::web::session::{extract_cookies, extract_session};
use bmc_grpc::web::{self, LoginRequest, LoginResponse};
use bmc_intl::{Intl, format_message};
use buffa_types::google::protobuf::{BoolValue, Empty};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::fmt::Display;
use std::sync::Arc;

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

impl<S> web::AuthenticationService for AuthenticationService<S>
where
    S: SessionManager,
{
    async fn is_authenticated(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<BoolValue> {
        Response::ok(BoolValue {
            value: extract_session::<S>(ctx.extensions()).is_ok(),
            ..Default::default()
        })
    }

    async fn login(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LoginRequest>,
    ) -> ServiceResult<LoginResponse> {
        let cookies = extract_cookies(ctx.extensions());
        let cookie = self
            .session_manager
            .login(request.password)
            .await
            .map_err(|error| login_error(&intl(&ctx), &error))?;

        #[cfg(debug_assertions)]
        tracing::debug!("Session {} has been started", cookie.value());

        cookies.add(cookie.clone());

        Response::ok(LoginResponse {
            token: cookie.value().to_owned(),
            timeout_s: S::SESSION_TIMEOUT,
        })
    }

    async fn logout(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        let cookies = extract_cookies(ctx.extensions());
        let session = extract_session::<S>(ctx.extensions())?;

        let cookie = self
            .session_manager
            .logout(session.clone())
            .await
            .map_err(|e| ConnectError::internal(e.to_string()))?;

        cookies.remove(cookie);

        Response::ok(Empty::default())
    }
}

fn login_error(intl: &Intl, error: &LoginError<impl Display>) -> ConnectError {
    match error {
        LoginError::BadCredentials => ConnectError::unauthenticated(format_message!(
            intl,
            default_message: "Incorrect username or password!",
            description: "Error on the login page when the password is wrong",
        )),
        LoginError::Failed(error) => ConnectError::internal(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_intl::Language;
    use connectrpc::ErrorCode;

    #[test]
    fn a_wrong_password_is_unauthenticated_and_worded() {
        let intl = bmc_intl::shared_formatter(Language::EN);
        let error = login_error(&intl, &LoginError::<&str>::BadCredentials);
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert_eq!(
            error.message.as_deref(),
            Some("Incorrect username or password!")
        );
    }

    #[test]
    fn a_device_that_failed_is_internal_and_keeps_its_own_text() {
        let intl = bmc_intl::shared_formatter(Language::EN);
        let error = login_error(&intl, &LoginError::Failed("no reply from ubus"));
        assert_eq!(error.code, ErrorCode::Internal);
        assert_eq!(error.message.as_deref(), Some("no reply from ubus"));
    }

    #[test]
    fn a_request_asking_for_chinese_is_refused_in_it() {
        let intl = crate::web::grpc::intl::asking_for("zh");
        let error = login_error(&intl, &LoginError::<&str>::BadCredentials);
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        let message = error.message.expect("BUG: a refusal is always worded");
        assert_ne!(
            message, "Incorrect username or password!",
            "worded in English"
        );
    }
}
