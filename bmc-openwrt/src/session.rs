// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::ROOT_USERNAME;
use axum_extra::extract::cookie::Cookie;
use bmc::session;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use tokio::process::Command;
use tracing::{debug, warn};

#[derive(Default, Clone, Debug)]
pub struct Handle {
    session_id: String,
    valid: bool,
}

impl Handle {
    #[must_use]
    pub fn new(session_id: String, valid: bool) -> Self {
        Self { session_id, valid }
    }
}

impl session::Handle for Handle {
    fn is_valid(&self) -> bool {
        self.valid
    }

    fn id(&self) -> String {
        self.session_id.clone()
    }

    fn username(&self) -> String {
        // TODO: remove
        ROOT_USERNAME.to_owned()
    }
}

/// Structure defined in https://openwrt.org/docs/guide-developer/ubus/session
#[derive(Deserialize, Debug)]
struct UbusSession {
    ubus_rpc_session: String,
    timeout: i64,
    expires: i64,
    #[serde(default)]
    data: HashMap<String, serde_json::Value>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("FromUtf8Error: {0}")]
    FromUtf8Error(#[from] std::string::FromUtf8Error),
    #[error("IO error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Ubus error: {0}")]
    UbusError(String),
    #[error("Session not found")]
    SessionNotFound,
    #[error("Session cookie not found")]
    SessionCookieNotFound,
    #[error("Bad credentials")]
    BadCredentials,
}

#[derive(Default, Debug, Clone)]
pub struct OpenwrtSessionManager;

impl OpenwrtSessionManager {
    const COOKIE_SESSION: &'static str = "session_id";
    const COOKIE_SESSION_PATH: &'static str = "/";

    // dumy token for LuCI access via ubus session
    const LUCI_DUMMY_TOKEN: &'static str = "0000";
    // dumy section for LuCI access via ubus session
    const LUCI_DUMMY_SECTION: &'static str = "0000";

    const UBUS_COMMAND: &'static str = "ubus";
    const UBUS_COMMAND_ARG_CALL: &'static str = "call";
    const UBUS_COMMAND_ARG_SESSION: &'static str = "session";
    const UBUS_COMMAND_ARG_LOGIN: &'static str = "login";
    const UBUS_COMMAND_ARG_SET: &'static str = "set";
    const UBUS_COMMAND_ARG_LIST: &'static str = "list";
    const UBUS_COMMAND_ARG_DESTROY: &'static str = "destroy";

    async fn call_ubus_session(command_name: &str, arg: Option<&str>) -> Result<String, Error> {
        let mut cmd = Command::new(Self::UBUS_COMMAND);
        cmd.arg(Self::UBUS_COMMAND_ARG_CALL);
        cmd.arg(Self::UBUS_COMMAND_ARG_SESSION);
        cmd.arg(command_name);

        if let Some(arg) = arg {
            cmd.arg(arg);
        }

        let output = cmd.output().await?;

        if !output.status.success() {
            return Err(Error::UbusError(String::from_utf8(output.stderr)?));
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    async fn call_ubus_session_deserialize(
        command_name: &str,
        arg: Option<&str>,
    ) -> Result<UbusSession, Error> {
        Ok(serde_json::from_str(
            &Self::call_ubus_session(command_name, arg).await?,
        )?)
    }

    async fn call_ubus_sessions_deserialize(
        command_name: &str,
        arg: Option<&str>,
    ) -> Result<Vec<UbusSession>, Error> {
        // We cannot directly deserialize output to Vec<UbusSession> because it is
        // not a properly formatted json array (missing [] and commas).
        // Instead, we can use `serde_json::StreamDeserializer`
        // to try parse it one by one

        let sessions =
            serde_json::Deserializer::from_str(&Self::call_ubus_session(command_name, arg).await?)
                .into_iter::<UbusSession>()
                .collect::<Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    async fn ubus_login(
        username: &str,
        password: &str,
        timeout: u32,
    ) -> Result<UbusSession, Error> {
        Self::call_ubus_session_deserialize(
            Self::UBUS_COMMAND_ARG_LOGIN,
            Some(
                &json!({
                    "username": username,
                    "password": password,
                    "timeout": timeout,
                })
                .to_string(),
            ),
        )
        .await
        .map_err(|_| Error::BadCredentials)
    }

    // Function for set ubus session user,dummy token and section
    // for compatible session access to LuCI
    async fn ubus_set_luci_compatible_values(
        ubus_rpc_session: &str,
        username: &str,
    ) -> Result<String, Error> {
        Self::call_ubus_session(
            Self::UBUS_COMMAND_ARG_SET,
            Some(
                &json!({
                    "ubus_rpc_session": ubus_rpc_session,
                    "values":{
                        "user": username,
                        "token": Self::LUCI_DUMMY_TOKEN,
                        "section": Self::LUCI_DUMMY_SECTION
                    }
                })
                .to_string(),
            ),
        )
        .await
    }

    async fn ubus_list() -> Result<Vec<UbusSession>, Error> {
        Self::call_ubus_sessions_deserialize(Self::UBUS_COMMAND_ARG_LIST, None).await
    }

    async fn ubus_find(ubus_rpc_session: &str) -> Result<UbusSession, Error> {
        Self::call_ubus_session_deserialize(
            Self::UBUS_COMMAND_ARG_LIST,
            Some(
                &json!({
                    "ubus_rpc_session": ubus_rpc_session,
                })
                .to_string(),
            ),
        )
        .await
    }

    async fn ubus_destroy(ubus_rpc_session: &str) -> Result<String, Error> {
        Self::call_ubus_session(
            Self::UBUS_COMMAND_ARG_DESTROY,
            Some(
                &json!({
                    "ubus_rpc_session": ubus_rpc_session,
                })
                .to_string(),
            ),
        )
        .await
    }
}

#[async_trait::async_trait]
impl session::Manager for OpenwrtSessionManager {
    type Error = Error;
    type Session = Handle;

    const SESSION_TIMEOUT: u32 = 3600;

    async fn login(&self, password: &str) -> Result<Cookie<'static>, Error> {
        let ubus_session = Self::ubus_login(ROOT_USERNAME, password, Self::SESSION_TIMEOUT).await?;

        debug!(
            "New ubus session {:?} created for user {}@{}",
            ubus_session, ROOT_USERNAME, password
        );

        Self::ubus_set_luci_compatible_values(&ubus_session.ubus_rpc_session, ROOT_USERNAME)
            .await?;

        let mut cookie = Cookie::new(Self::COOKIE_SESSION, ubus_session.ubus_rpc_session);
        cookie.set_path(Self::COOKIE_SESSION_PATH);
        cookie.set_max_age(time::Duration::seconds(Self::SESSION_TIMEOUT.into()));

        Ok(cookie)
    }

    async fn extend(&self, handle: Handle) -> Result<Cookie<'static>, Error> {
        let ubus_session = Self::ubus_find(&handle.session_id).await?;

        debug!(
            "Extend ubus session {:?} for session_id: {}",
            ubus_session, handle.session_id
        );

        let mut cookie = Cookie::new(Self::COOKIE_SESSION, handle.session_id);
        cookie.set_path(Self::COOKIE_SESSION_PATH);
        cookie.set_max_age(time::Duration::seconds(ubus_session.timeout));

        Ok(cookie)
    }

    async fn logout(&self, handle: Handle) -> Result<Cookie<'static>, Error> {
        debug!("Logout session {:?}", handle);

        if let Err(e) = Self::ubus_destroy(&handle.session_id).await {
            warn!(
                "Failed to destroy ubus session: {} for session_id: {}",
                e, handle.session_id
            );
        }

        let mut cookie = Cookie::new(Self::COOKIE_SESSION, "");
        cookie.set_path(Self::COOKIE_SESSION_PATH);
        cookie.set_max_age(time::Duration::default());

        Ok(cookie)
    }

    async fn logout_all_related(&self, handle: Handle) -> Result<(), Error> {
        const USERNAME_KEY: &str = "username";

        let sessions = Self::ubus_list().await?;

        let maybe_target_username = {
            let target_session = sessions
                .iter()
                .find(|session| session.ubus_rpc_session == handle.session_id)
                .ok_or(Error::SessionNotFound)?;

            target_session.data.get(USERNAME_KEY).cloned()
        };

        let sessions_to_destroy = sessions.into_iter().filter(|session| {
            // We don't want to destroy current session
            if session.ubus_rpc_session == handle.session_id {
                return false;
            }
            // We care only about sessions created via our login procedure with matching username.
            let maybe_username = session.data.get(USERNAME_KEY);

            match (maybe_username, maybe_target_username.as_ref()) {
                (Some(username), Some(target_username)) => username == target_username,
                _ => false,
            }
        });

        debug!("Logout all related sessions");
        for session in sessions_to_destroy {
            if let Err(e) = Self::ubus_destroy(&session.ubus_rpc_session).await {
                warn!(
                    "Failed to destroy ubus session: {} for session_id: {}",
                    e, session.ubus_rpc_session
                );
            }
        }

        Ok(())
    }

    async fn find(&self, cookies: &[Cookie<'_>]) -> Result<Handle, Error> {
        let cookie = cookies
            .iter()
            .find(|cookie| cookie.name() == Self::COOKIE_SESSION)
            .ok_or(Error::SessionCookieNotFound)?;

        debug!(
            "Found session cookie name:{} value:{}",
            cookie.name(),
            cookie.value()
        );

        Ok(Self::ubus_find(cookie.value())
            .await
            .map(|ubus_session| {
                Handle::new(ubus_session.ubus_rpc_session, ubus_session.expires > 0)
            })
            .unwrap_or_default())
    }
}
