// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::session::{self};

use axum_extra::extract::cookie::Cookie;
use futures::Future;
use http::{HeaderMap, HeaderValue, Request, Response, header};
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
            let token = extract_token(&req);
            let cookies = if let Some(token) = token.as_ref() {
                // NOTE: this is not an elegant integration of gRPC and existing session manager.
                // Session manager provides cookie interface, not a token interface. More of that
                // the name of cookie is defined by specific boser implementation, not a library.
                // this part has to be changed in future
                vec![Cookie::new("session_id", token)]
            } else {
                extract_cookies(req.headers()).collect::<Vec<Cookie<'_>>>()
            };
            session_manager
                .find(&cookies)
                .await
                .ok()
                .and_then(|session| req.extensions_mut().insert(session));

            service.call(req).await
        })
    }
}

fn strip_bearer(token: &str) -> String {
    token.split_whitespace().last().unwrap_or(token).to_owned()
}

pub(crate) fn extract_cookies(
    headers: &HeaderMap<HeaderValue>,
) -> impl Iterator<Item = Cookie<'_>> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .flat_map(|hdr| {
            let s = String::from_utf8_lossy(hdr.as_bytes());
            Cookie::split_parse_encoded(s)
        })
        .filter_map(Result::ok)
}

pub(crate) fn extract_token<T>(request: &Request<T>) -> Option<String> {
    request
        .headers()
        .get(header::AUTHORIZATION.as_str())
        .and_then(|token_header| token_header.to_str().ok())
        .map(String::from)
        .map(|s| strip_bearer(&s))
}

/// Retrieves authentication session and fails if it is not present
pub fn extract_session<S: session::Manager, R>(
    request: &tonic::Request<R>,
) -> Result<&S::Session, tonic::Status> {
    request
        .extensions()
        .get::<S::Session>()
        .ok_or_else(|| tonic::Status::unauthenticated("Missing or invalid session"))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use header::{ACCEPT_ENCODING, CONTENT_LENGTH, COOKIE};

    use super::*;
    #[test]
    fn test_extract_token() {
        let req = Request::builder()
            .header(header::AUTHORIZATION, "Bearer gVZIvHtgCYYfbxXa")
            .body(Body::empty())
            .expect("BUG: failed to build request");
        assert_eq!(extract_token(&req), Some("gVZIvHtgCYYfbxXa".to_owned()));
    }

    #[test]
    fn test_extract_token_no_bearer() {
        let req = Request::builder()
            .header(header::AUTHORIZATION, "gVZIvHtgCYYfbxXa")
            .body(Body::empty())
            .expect("BUG: failed to build request");
        assert_eq!(extract_token(&req), Some("gVZIvHtgCYYfbxXa".to_owned()));
    }

    #[test]
    fn test_extract_cookies() {
        let mut header_map = HeaderMap::new();

        let headers = vec![
            (COOKIE, "session_id=gVZIvHtgCYYfbxXa"),
            (CONTENT_LENGTH, "320"),
            (ACCEPT_ENCODING, "gzip"),
            (COOKIE, "test=kjfdsQFKSowowFFW; test2=fdsdfQgWHd"),
        ];

        for (key, value) in headers {
            header_map.append(
                key,
                value.parse().expect("BUG: failed to parse header value"),
            );
        }

        let cookies = extract_cookies(&header_map).collect::<Vec<Cookie<'_>>>();

        let expected_cookies = vec![
            Cookie::new("session_id", "gVZIvHtgCYYfbxXa"),
            Cookie::new("test", "kjfdsQFKSowowFFW"),
            Cookie::new("test2", "fdsdfQgWHd"),
        ];

        assert_eq!(cookies, expected_cookies);
    }
}
