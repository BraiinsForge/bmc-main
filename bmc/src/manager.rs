// Copyright (C) 2025  Braiins Systems s.r.o.

#[async_trait::async_trait]
pub trait BmcManager: Sync + Send + 'static {
    type SessionManager: crate::session::Manager;
    type Error: std::error::Error + Send + Sync;

    fn version(&self) -> String;
    fn session_manager(&self) -> Self::SessionManager;
    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error>;
}
