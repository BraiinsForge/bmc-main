// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::pwd::{PasswordHashType, SHADOW_PATH, ShadowFile};
use crate::{ROOT_USERNAME, pwd};
use anyhow::anyhow;
use axum_extra::extract::cookie::Cookie;
use bmc::{BmcManager, time::Timezone};
use bmc_platform::BmcPlatform;
use std::io;
use std::path::Path;
use tokio::{fs, process::Command};
use tracing::info;

use crate::unix::call_command;

#[derive(Debug)]
pub struct Manager {
    pub session_manager: OpenwrtSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
}

impl Manager {
    const SYSUPGRADE_BIN: &'static str = "/sbin/sysupgrade";
    const SYSUPGRADE_ARG_NO_SAVE: &'static str = "-n";
    const UPGRADE_RESULT_FILE_PATH: &str = "/etc/upgrade_result";
    const UCI_SYSTEM_ZONENAME: &str = "system.@system[0].zonename";
    const UCI_SYSTEM_TIMEZONE: &str = "system.@system[0].timezone";

    #[must_use]
    pub fn new(session_manager: OpenwrtSessionManager, timezone: Timezone) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(timezone);
        Self {
            session_manager,
            timezone_sender,
        }
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

    async fn check_password(&self, password: Option<&str>) -> Result<bool, Self::Error> {
        let shadow_file = ShadowFile::from_file(SHADOW_PATH)?;
        let matches = shadow_file.check_credentials(ROOT_USERNAME, password);

        Ok(matches)
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        info!("Changing `{ROOT_USERNAME}` password");

        let mut shadow_file = ShadowFile::from_file(SHADOW_PATH)?;
        shadow_file.set_password(ROOT_USERNAME, password, PasswordHashType::Md5)?;

        let temp_shadow_file_path = format!("{SHADOW_PATH}.tmp");

        fs::write(&temp_shadow_file_path, shadow_file.to_string()).await?;
        fs::rename(&temp_shadow_file_path, SHADOW_PATH).await?;

        Ok(())
    }

    fn timezone(&self) -> Timezone {
        self.timezone_sender.borrow().clone()
    }

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()> {
        let zonename_cmd = format!("{}={}", Self::UCI_SYSTEM_ZONENAME, timezone.iana);
        call_command("uci", &["set", &zonename_cmd]).await?;

        let timezone_cmd = format!("{}={}", Self::UCI_SYSTEM_TIMEZONE, timezone.posix);
        call_command("uci", &["set", &timezone_cmd]).await?;

        call_command("uci", &["commit", "system"]).await?;
        call_command("/etc/init.d/system", &["restart"]).await?;

        self.timezone_sender.send_if_modified(|current| {
            if *current != timezone {
                *current = timezone;
                return true;
            }
            false
        });

        Ok(())
    }

    fn watch_timezone_updates(&self) -> tokio::sync::watch::Receiver<Timezone> {
        self.timezone_sender.subscribe()
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
    #[error(transparent)]
    ShadowFile(#[from] pwd::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
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
