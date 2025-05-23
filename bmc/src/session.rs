// Copyright (C) 2025  Braiins Systems s.r.o.

use axum_extra::extract::cookie::Cookie;

pub trait Handle: Clone + Unpin + Send + Sync + 'static + std::fmt::Debug {
    fn is_valid(&self) -> bool;
    fn id(&self) -> String;
}

#[async_trait::async_trait]
pub trait Manager: Default + Sync + Send + 'static {
    type Error: std::error::Error + Send + Sync;
    type Session: Handle;

    // session timeout in seconds
    const SESSION_TIMEOUT: u32;

    async fn login(&self, password: &str) -> Result<Cookie<'static>, Self::Error>;
    async fn logout(&self, session: Self::Session) -> Result<Cookie<'static>, Self::Error>;
    /// Logout all related sessions of the user except current session
    async fn logout_all_related(&self, session: Self::Session) -> Result<(), Self::Error>;
    async fn extend(&self, session: Self::Session) -> Result<Cookie<'static>, Self::Error>;
    async fn find(&self, cookies: &[Cookie<'_>]) -> Result<Self::Session, Self::Error>;
}
