// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::session;
use futures::Future;
use http::{HeaderValue, Request, Response};
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

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let mut service = self.service.clone();
        let session_manager = self.session_manager.clone();

        Box::pin(async move {
            let cookies = extract_cookies(req.headers());
            let mut response_set_cookie = None;

            if session_manager.find(&cookies).await.is_err() {
                if let Ok(cookie) = session_manager.login(DEFAULT_PASSWORD).await {
                    if let Ok(parsed_cookie) = cookie.to_string().parse::<HeaderValue>() {
                        response_set_cookie = Some(parsed_cookie.clone());

                        req.headers_mut()
                            .append(http::header::COOKIE, parsed_cookie);
                    }
                }
            }

            let mut resp = service.call(req).await?;

            if let Some(cookie) = response_set_cookie {
                resp.headers_mut().append(http::header::SET_COOKIE, cookie);
            }
            Ok(resp)
        })
    }
}
