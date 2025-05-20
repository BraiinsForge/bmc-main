// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::anyhow;
use axum_extra::extract::cookie::Cookie;
use bmc::BmcManager;
use bmc_platform::BmcPlatform;
use std::path::Path;
use tokio::{fs, process::Command};
use tracing::info;

#[derive(Debug)]
pub struct Manager {
    pub session_manager: OpenwrtSessionManager,
}

impl Manager {
    const SYSUPGRADE_BIN: &'static str = "/sbin/sysupgrade";
    const SYSUPGRADE_ARG_NO_SAVE: &'static str = "-n";
    const UPGRADE_RESULT_FILE_PATH: &str = "/etc/upgrade_result";

    #[must_use]
    pub fn new(session_manager: OpenwrtSessionManager) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl BmcManager for Manager {
    type SessionManager = OpenwrtSessionManager;
    type Error = Error;

    fn version(&self) -> String {
        todo!()
    }

    fn platform(&self) -> BmcPlatform {
        BmcPlatform::BraiinsBmc
    }

    async fn upgrade(&self, keep_settings: bool, upgrade_image_path: &Path) -> anyhow::Result<()> {
        let mut sysupgrade = Command::new(Self::SYSUPGRADE_BIN);
        if !keep_settings {
            sysupgrade.arg(Self::SYSUPGRADE_ARG_NO_SAVE);
        }
        sysupgrade.arg(upgrade_image_path.as_os_str());

        let mut handle = sysupgrade.spawn()?;

        let status = handle
            .wait()
            .await
            .map_err(|_| anyhow!("Invalid firmware image"))?;

        if let Some(code) = status.code() {
            match code {
                // Error code "1" is returned on BCB when using incompatible image, unsigned image or wrong signature keys
                1 => Err(anyhow!("Invalid firmware image")),
                _ => Ok(()),
            }
        } else {
            Err(anyhow!("Upgrade failed"))
        }
    }

    async fn check_and_remove_upgrade_marker(&self) -> bool {
        let is_after_upgrade = Path::new(Self::UPGRADE_RESULT_FILE_PATH).exists();

        if is_after_upgrade {
            _ = fs::remove_file(Self::UPGRADE_RESULT_FILE_PATH).await;
        }

        is_after_upgrade
    }

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        info!("Setting password to {:?}", password);
        Ok(())
    }
}

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

impl OpenwrtSessionManager {
    const IMPLICIT_USERNAME: &'static str = "root";
}

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
            Self::IMPLICIT_USERNAME,
            password
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
