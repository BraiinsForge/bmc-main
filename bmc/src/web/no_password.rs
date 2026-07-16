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

use crate::session;
use futures::Future;
use http::{Request, Response};
use tower::{Layer, Service};

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use super::session::extract_cookies;

const DEFAULT_PASSWORD: &str = "";

#[derive(Debug)]
pub struct NoPassword<U, V>
where
    U: session::Manager,
{
    session_manager: Arc<U>,
    service: V,
}

impl<U, V> NoPassword<U, V>
where
    U: session::Manager,
{
    pub fn new(session_manager: Arc<U>, service: V) -> Self {
        Self {
            session_manager,
            service,
        }
    }
}

impl<U: session::Manager, V: Clone> Clone for NoPassword<U, V> {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
            service: self.service.clone(),
        }
    }
}

#[derive(Debug)]
pub struct NoPasswordLayer<T>
where
    T: session::Manager,
{
    session_manager: Arc<T>,
}

impl<T: session::Manager> Clone for NoPasswordLayer<T> {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
        }
    }
}

impl<T> NoPasswordLayer<T>
where
    T: session::Manager,
{
    pub fn new(session_manager: Arc<T>) -> Self {
        Self { session_manager }
    }
}

impl<U, V> Layer<V> for NoPasswordLayer<U>
where
    U: session::Manager,
{
    type Service = NoPassword<U, V>;

    fn layer(&self, inner: V) -> Self::Service {
        NoPassword::new(self.session_manager.clone(), inner)
    }
}

impl<T, S, ReqBody, ResBody> Service<Request<ReqBody>> for NoPassword<T, S>
where
    T: session::Manager + Send + Sync + 'static,
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + Sync + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let mut service = self.service.clone();
        let session_manager = self.session_manager.clone();

        Box::pin(async move {
            let cookies = extract_cookies(req.extensions());

            if session_manager.find(&cookies.list()).await.is_err()
                && let Ok(cookie) = session_manager.login(DEFAULT_PASSWORD).await
            {
                cookies.add(cookie);
            }

            service.call(req).await
        })
    }
}
