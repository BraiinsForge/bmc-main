// Copyright (C) 2025  Braiins Systems s.r.o.
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
