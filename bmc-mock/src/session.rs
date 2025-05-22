// Copyright (C) 2025  Braiins Systems s.r.o.

// TODO: clean expired tokens from sessions

use axum_extra::extract::cookie::{Cookie, SameSite};
use bmc::session::{self, Handle as _};
use rand::{Rng, distr::Alphanumeric};
use time::OffsetDateTime;
use tracing::debug;

use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::sync::{Arc, Mutex, MutexGuard};

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
pub struct Handle {
    username: String,
    token: String,
    valid: bool,
}

impl Handle {
    pub fn new(username: String, token: String, valid: bool) -> Self {
        Self {
            username,
            token,
            valid,
        }
    }
}

impl session::Handle for Handle {
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

#[derive(Clone, Debug)]
struct Session {
    username: String,
    expiration_time: i64,
}

impl Session {
    pub fn new(username: String, expiration_time: i64) -> Self {
        Self {
            username,
            expiration_time,
        }
    }
}

/// key is a session token
type Sessions = HashMap<String, Session>;

#[derive(Default, Clone)]
pub struct MockSessionManager {
    sessions: Arc<Mutex<Sessions>>,
    password: Arc<Mutex<Option<String>>>,
}

impl Debug for MockSessionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MockSessionManager {{ sessions: {:?}, password: {:?} }}",
            self.sessions, self.password
        )
    }
}

impl MockSessionManager {
    const COOKIE_SESSION: &'static str = "session_id";
    const COOKIE_SESSION_PATH: &'static str = "/";
    const COOKIE_HTTP_ONLY: bool = true;
    const COOKIE_SAME_SITE: SameSite = SameSite::Strict;
    const DEFAULT_RANDOM_SESSION_LENGTH: usize = 16;
    const IMPLICIT_USERNAME: &'static str = "root";

    #[must_use]
    pub fn new(password: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(Sessions::new())),
            password,
        }
    }

    fn get_now() -> i64 {
        OffsetDateTime::unix_timestamp(OffsetDateTime::now_utc())
    }

    fn get_expiration_time(timeout: u32) -> i64 {
        Self::get_now() + i64::from(timeout)
    }

    fn sessions_lock(&self) -> MutexGuard<'_, Sessions> {
        self.sessions
            .lock()
            .expect("BUG: cannot lock session buffer")
    }
}

#[async_trait::async_trait]
impl session::Manager for MockSessionManager {
    type Error = Error;
    type Session = Handle;

    const SESSION_TIMEOUT: u32 = 3600;

    async fn login(&self, password: &str) -> Result<Cookie<'static>, Error> {
        let password_is_correct = {
            let guard = self.password.lock().expect("BUG: cannot lock password");
            guard.as_ref().is_none_or(|p| *p == password)
        };

        if !password_is_correct {
            return Err(Error::BadCredentials);
        }

        let random_session: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(Self::DEFAULT_RANDOM_SESSION_LENGTH)
            .map(char::from)
            .collect();

        debug!(
            "New random session {} created for user {}@{}",
            random_session,
            Self::IMPLICIT_USERNAME,
            password
        );

        let mut sessions = self.sessions_lock();

        sessions.insert(
            random_session.clone(),
            Session::new(
                Self::IMPLICIT_USERNAME.to_owned(),
                Self::get_expiration_time(Self::SESSION_TIMEOUT),
            ),
        );

        let mut cookie = Cookie::new(Self::COOKIE_SESSION, random_session);
        cookie.set_path(Self::COOKIE_SESSION_PATH);
        cookie.set_max_age(time::Duration::seconds(Self::SESSION_TIMEOUT.into()));
        cookie.set_http_only(Self::COOKIE_HTTP_ONLY);
        cookie.set_same_site(Self::COOKIE_SAME_SITE);

        Ok(cookie)
    }

    async fn logout(&self, handle: Handle) -> Result<Cookie<'static>, Error> {
        debug!("Logout session {:?}", handle);

        let mut sessions = self.sessions_lock();

        sessions.remove(&handle.token);

        let mut cookie = Cookie::new(Self::COOKIE_SESSION, "");
        cookie.set_path(Self::COOKIE_SESSION_PATH);
        cookie.set_max_age(time::Duration::default());

        Ok(cookie)
    }

    async fn logout_all_related(&self, handle: Handle) -> Result<(), Error> {
        let mut sessions = self.sessions_lock();

        let target_username = {
            let target_session = sessions.get(&handle.token).ok_or(Error::SessionNotFound)?;
            target_session.username.clone()
        };

        debug!("Logout all related sessions");
        sessions.retain(|token, session| {
            // We don't want to destroy current session
            if *token == handle.token {
                return true;
            }

            // We don't want to destroy sessions for different user
            session.username != target_username
        });

        Ok(())
    }

    async fn extend(&self, handle: Handle) -> Result<Cookie<'static>, Error> {
        if handle.is_valid() {
            debug!("Extend session {:?}", handle);

            let mut sessions = self.sessions_lock();

            let session = sessions
                .get_mut(&handle.token)
                .ok_or(Error::SessionNotFound)?;

            session.expiration_time = Self::get_expiration_time(Self::SESSION_TIMEOUT);

            let mut cookie = Cookie::new(Self::COOKIE_SESSION, handle.token);
            cookie.set_path(Self::COOKIE_SESSION_PATH);
            cookie.set_max_age(time::Duration::seconds(Self::SESSION_TIMEOUT.into()));
            cookie.set_http_only(Self::COOKIE_HTTP_ONLY);
            cookie.set_same_site(Self::COOKIE_SAME_SITE);

            Ok(cookie)
        } else {
            debug!("Invalid session {:?} can not be extended", handle);
            Err(Error::SessionCookieInvalid)
        }
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

        self.sessions_lock()
            .get(cookie.value())
            .map(|session| {
                Handle::new(
                    session.username.clone(),
                    cookie.value().into(),
                    session.expiration_time > Self::get_now(),
                )
            })
            .filter(Handle::is_valid)
            .ok_or(Error::SessionNotFound)
    }
}
