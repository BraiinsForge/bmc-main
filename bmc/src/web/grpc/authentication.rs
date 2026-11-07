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
use super::shared::{StatusExt as _, ToStatus};
use crate::session::{LoginError, Manager as SessionManager};
use crate::web::session::{extract_cookies, extract_session};
use bmc_grpc::web::{self, LoginRequest, LoginResponse};
use bmc_intl::{Intl, format_message};
use buffa_types::google::protobuf::{BoolValue, Empty};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::fmt::Display;
use std::sync::Arc;
use tracing::warn;

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
            value: extract_session::<S>(ctx.headers(), ctx.extensions()).is_ok(),
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
            .or_status(&intl(&ctx))?;

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
        let session = extract_session::<S>(ctx.headers(), ctx.extensions())?;

        let cookie = self
            .session_manager
            .logout(session.clone())
            .await
            .map_err(|e| {
                warn!("Failed to log out: {e}");
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to log out!",
                    description: "Error when the device could not end the session after the user chose to log out",
                ))
            })?;

        cookies.remove(cookie);

        Response::ok(Empty::default())
    }
}

impl<E: Display> ToStatus for LoginError<E> {
    fn to_status(&self, intl: &Intl) -> ConnectError {
        match self {
            LoginError::BadCredentials => ConnectError::unauthenticated(format_message!(
                intl,
                default_message: "Incorrect username or password!",
                description: "Error on the login page when the password is wrong",
            )),
            LoginError::Failed(error) => {
                warn!("Failed to check the password: {error}");
                ConnectError::internal(format_message!(
                    intl,
                    default_message: "The device could not check the password.",
                    description: "Error on the login page when the check itself failed, so the password may well be right",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::grpc::tests::{assert_worded, status_text};
    use bmc_intl::Language;
    use connectrpc::ErrorCode;

    #[test]
    fn every_login_refusal_is_worded() {
        assert_worded(|intl| {
            vec![
                status_text(&LoginError::<&str>::BadCredentials.to_status(intl)),
                status_text(&LoginError::Failed("1").to_status(intl)),
            ]
        });
    }

    #[test]
    fn a_wrong_password_is_unauthenticated_and_worded() {
        let intl = bmc_intl::shared_formatter(Language::EN);
        let error = LoginError::<&str>::BadCredentials.to_status(&intl);
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert_eq!(
            error.message.as_deref(),
            Some("Incorrect username or password!")
        );
    }

    #[test]
    fn a_device_that_failed_is_internal_and_worded_without_its_own_text() {
        let intl = bmc_intl::shared_formatter(Language::EN);
        let error = LoginError::Failed("no reply from ubus").to_status(&intl);
        assert_eq!(error.code, ErrorCode::Internal);
        assert_eq!(
            error.message.as_deref(),
            Some("The device could not check the password.")
        );
    }

    #[test]
    fn a_request_asking_for_chinese_is_refused_in_it() {
        let intl = crate::web::grpc::intl::asking_for("zh");
        let error = LoginError::<&str>::BadCredentials.to_status(&intl);
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        let message = error.message.expect("BUG: a refusal is always worded");
        assert_ne!(
            message, "Incorrect username or password!",
            "worded in English"
        );
    }
}
