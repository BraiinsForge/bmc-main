// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::ROOT_USERNAME;
use axum_extra::extract::cookie::Cookie;
use tracing::info;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Session not found")]
    SessionNotFound,
    #[error("Session cookie not found")]
    SessionCookieNotFound,
    #[error("Session cookie is invalid")]
    SessionCookieInvalid,
    #[error("Bad credentials")]
    BadCredentials,
}

#[derive(Default, Clone, Debug)]
pub struct OpenwrtSessionManager;

#[derive(Default, Clone, Debug)]
pub struct Handle {
    username: String,
    token: String,
    valid: bool,
}

impl Handle {
    #[must_use]
    pub fn new(username: String, token: String, valid: bool) -> Self {
        Self {
            username,
            token,
            valid,
        }
    }
}

impl bmc::session::Handle for Handle {
    fn is_valid(&self) -> bool {
        self.valid
    }

    fn id(&self) -> String {
        self.token.clone()
    }

    fn username(&self) -> String {
        self.username.clone()
    }
}

#[async_trait::async_trait]
impl bmc::session::Manager for OpenwrtSessionManager {
    type Error = Error;
    type Session = Handle;

    const SESSION_TIMEOUT: u32 = 3600;

    async fn login(&self, password: &str) -> Result<Cookie<'static>, Error> {
        info!(
            "Login with username: {} and password: {}",
            ROOT_USERNAME, password
        );
        unimplemented!()
    }

    async fn logout(&self, handle: Handle) -> Result<Cookie<'static>, Error> {
        info!("Logout with handle: {}", handle.token);
        unimplemented!()
    }

    async fn logout_all_related(&self, handle: Handle) -> Result<(), Error> {
        info!("Logout all related with handle: {}", handle.token);
        unimplemented!()
    }

    async fn extend(&self, handle: Handle) -> Result<Cookie<'static>, Error> {
        info!("Extend with handle: {}", handle.token);
        unimplemented!()
    }

    async fn find(&self, cookies: &[Cookie<'_>]) -> Result<Handle, Error> {
        info!("Find with cookies: {:?}", cookies);
        unimplemented!()
    }
}
