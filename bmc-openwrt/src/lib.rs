// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod generic_backlight_driver;
pub mod linux_framebuffer_platform;

use anyhow::Result;
use axum_extra::extract::cookie::Cookie;
use bmc::BmcManager;
use tokio as _;
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

#[derive(Debug, Clone)]
pub struct OpenwrtManager {
    pub session_manager: OpenwrtSessionManager,
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

    async fn login(&self, username: String, password: String) -> Result<Cookie<'_>, Error> {
        info!(
            "Login with username: {} and password: {}",
            username, password
        );
        unimplemented!()
    }

    async fn logout(&self, handle: Handle) -> Result<Cookie<'_>, Error> {
        info!("Logout with handle: {}", handle.token);
        unimplemented!()
    }

    async fn logout_all_related(&self, handle: Handle) -> Result<Cookie<'_>, Error> {
        info!("Logout all related with handle: {}", handle.token);
        unimplemented!()
    }

    async fn extend(&self, handle: Handle) -> Result<Cookie<'_>, Error> {
        info!("Extend with handle: {}", handle.token);
        unimplemented!()
    }

    async fn find(&self, cookies: &[Cookie<'_>]) -> Result<Handle, Error> {
        info!("Find with cookies: {:?}", cookies);
        unimplemented!()
    }
}

#[async_trait::async_trait]
impl BmcManager for OpenwrtManager {
    type SessionManager = OpenwrtSessionManager;
    type Error = Error;

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    fn version(&self) -> String {
        "Hello from Openwrt".to_owned()
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        info!("Setting password to {:?}", password);
        Ok(())
    }
}
