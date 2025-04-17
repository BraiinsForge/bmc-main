// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::session::{Handle, Manager as SessionManager};
use axum_extra::extract::cookie::Cookie;
use http::{Extensions, header};
use tonic::{Status, body::Body, codegen::http::Request};
use tonic_middleware::RequestInterceptor;
use tracing::debug;

#[derive(Clone)]
pub struct AuthInterceptor<S: SessionManager + Clone> {
    pub session_manager: std::sync::Arc<S>,
}

#[async_trait::async_trait]
impl<S: SessionManager + Clone> RequestInterceptor for AuthInterceptor<S> {
    async fn intercept(&self, mut req: Request<Body>) -> Result<Request<Body>, Status> {
        debug!("Intercepting request: {:?}", req);
        let token = req
            .headers()
            .get(header::AUTHORIZATION.as_str())
            .and_then(|token_header| token_header.to_str().ok())
            .map(String::from);

        let session_manager = self.session_manager.clone();

        let mut authenticated = false;
        if let Some(token) = token.as_ref() {
            // NOTE: this is not an elegant integration of gRPC and existing session manager.
            // Session manager provides cookie interface, not a token interface. More of that
            // the name of cookie is defined by specific boser implementation, not a library.
            // this part has to be changed in future
            let cookies = [Cookie::new("session_id", token)];

            // find the session by its ID from token
            if let Ok(session) = session_manager.find(&cookies).await {
                // extend the session
                let cookie = session_manager.extend(session.clone()).await;
                if cookie.is_ok() {
                    req.extensions_mut().insert(session);
                    authenticated = true;
                }
            }
        }

        if !authenticated {
            // make sure, there is no authentication header anymore
            req.headers_mut().remove(header::AUTHORIZATION.as_str());
        }
        // The actual check
        let _ = get_session::<S>(req.extensions()).ok_or_else(|| {
            tonic::Status::unauthenticated("Missing or invalid authentication token")
        })?;
        Ok(req)
    }
}

fn get_session<S: SessionManager>(extensions: &Extensions) -> Option<&S::Session> {
    extensions.get::<S::Session>().and_then(|session| {
        debug!("Session: {:?}", session);
        debug!("Session is valid: {}", session.is_valid());
        session.is_valid().then_some(session)
    })
}

pub fn check<S: SessionManager, R>(
    request: &tonic::Request<R>,
) -> Result<&S::Session, tonic::Status> {
    get_session::<S>(request.extensions())
        .ok_or_else(|| tonic::Status::unauthenticated("Missing or invalid authentication token"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use header::{ACCEPT_ENCODING, CONTENT_LENGTH, COOKIE};
    use http::{HeaderMap, HeaderValue};

    fn extract_cookies(headers: &HeaderMap<HeaderValue>) -> impl Iterator<Item = Cookie<'_>> {
        headers
            .get_all(header::COOKIE)
            .iter()
            .flat_map(|hdr| {
                let s = String::from_utf8_lossy(hdr.as_bytes());
                Cookie::split_parse_encoded(s)
            })
            .filter_map(Result::ok)
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

        headers.into_iter().for_each(|(key, value)| {
            header_map.append(
                key,
                value.parse().expect("BUG: failed to parse header value"),
            );
        });

        let cookies = extract_cookies(&header_map).collect::<Vec<Cookie<'_>>>();

        let expected_cookies = vec![
            Cookie::new("session_id", "gVZIvHtgCYYfbxXa"),
            Cookie::new("test", "kjfdsQFKSowowFFW"),
            Cookie::new("test2", "fdsdfQgWHd"),
        ];

        assert_eq!(cookies, expected_cookies);
    }
}
