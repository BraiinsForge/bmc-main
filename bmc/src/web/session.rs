// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::session::{self};

use futures::Future;
use http::{Request, Response};
use tower::{Layer, Service};

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Debug)]
pub struct Session<U, V>
where
    U: session::Manager,
{
    session_manager: Arc<U>,
    service: V,
}

impl<U, V> Session<U, V>
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

impl<U: session::Manager, V: Clone> Clone for Session<U, V> {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
            service: self.service.clone(),
        }
    }
}

#[derive(Debug)]
pub struct SessionLayer<T>
where
    T: session::Manager,
{
    session_manager: Arc<T>,
}

impl<T: session::Manager> Clone for SessionLayer<T> {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
        }
    }
}

impl<T> SessionLayer<T>
where
    T: session::Manager,
{
    pub fn new(session_manager: Arc<T>) -> Self {
        Self { session_manager }
    }
}

impl<U, V> Layer<V> for SessionLayer<U>
where
    U: session::Manager,
{
    type Service = Session<U, V>;

    fn layer(&self, inner: V) -> Self::Service {
        Session::new(self.session_manager.clone(), inner)
    }
}

#[tonic::async_trait]
impl<T, S, ReqBody, ResBody> Service<Request<ReqBody>> for Session<T, S>
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

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let mut service = self.service.clone();
        let session_manager = self.session_manager.clone();

        Box::pin(async move {
            let cookies = extract_cookies(req.extensions());

            if let Ok(session) = session_manager.find(&cookies.list()).await
                && let Ok(cookie) = session_manager.extend(session).await
            {
                // NOTE: extend does not return updated session, only cookie.
                // Previous session would have incorrect expiration time.
                let session = session_manager
                    .find(std::slice::from_ref(&cookie))
                    .await
                    .expect("BUG: session must be available, because it was extended");

                cookies.add(cookie);
                req.extensions_mut().insert(session);
            }

            service.call(req).await
        })
    }
}

pub(crate) fn extract_cookies(extensions: &http::Extensions) -> &tower_cookies::Cookies {
    extensions
        .get::<tower_cookies::Cookies>()
        .expect("BUG: Missing cookies jar, check layers")
}

/// Retrieves authentication session and fails if it is not present
pub fn extract_session<S: session::Manager>(
    extensions: &http::Extensions,
) -> Result<&S::Session, tonic::Status> {
    extensions
        .get::<S::Session>()
        .ok_or_else(|| tonic::Status::unauthenticated("Missing or invalid session"))
}
