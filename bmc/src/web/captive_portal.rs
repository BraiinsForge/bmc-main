// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{BmcManager, manager::BmcState};
use axum::body::Body;
use http::header::LOCATION;
use hyper::{Request, Response};
use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};

use super::http_server::HttpServer;

const SUFFIX_COM: &str = ".com";
const SUFFIX_NET: &str = ".net";
const SUFFIX_INFO: &str = ".info";
const SUFFIX_US: &str = ".us";
const SUFFIX_NETWORK: &str = ".network";

pub struct CaptivePortalLayer<T>
where
    T: BmcManager,
{
    manager: Arc<T>,
}

impl<T> CaptivePortalLayer<T>
where
    T: BmcManager,
{
    pub fn new(manager: Arc<T>) -> Self {
        Self { manager }
    }
}

impl<T> Clone for CaptivePortalLayer<T>
where
    T: BmcManager,
{
    fn clone(&self) -> Self {
        Self {
            manager: self.manager.clone(),
        }
    }
}

impl<S, T> Layer<S> for CaptivePortalLayer<T>
where
    T: BmcManager,
{
    type Service = CaptivePortalMiddleware<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        CaptivePortalMiddleware {
            inner,
            manager: self.manager.clone(),
        }
    }
}

pub struct CaptivePortalMiddleware<S, T>
where
    T: BmcManager,
{
    inner: S,
    manager: Arc<T>,
}

impl<S, T: BmcManager> CaptivePortalMiddleware<S, T> {
    // NOTE: Original list of urls to return redirect is here: https://captivebehavior.wballiance.com/
    // It is not needed to check individual url, it can be decided based on the top level domain
    fn should_redirect(req: &Request<Body>, state: &BmcState) -> bool {
        if *state == BmcState::Operational {
            return false;
        }

        // This is covering a case when user displays the main page in browser. Initial setup needs to be displayed instead of the login page
        let uri_path = req.uri().path();

        match (state, uri_path) {
            (_, HttpServer::<T>::ROOT_URL_ENDPOINT)
            | (
                &BmcState::FactoryDefault | &BmcState::WifiReconfiguration,
                HttpServer::<T>::DEVICE_SETUP_URL_ENDPOINT,
            )
            | (&BmcState::SetupPending, HttpServer::<T>::WIFI_SETUP_URL_ENDPOINT) => {
                return true;
            }
            _ => (),
        }

        if let Some(host) = req.headers().get("Host")
            && let Ok(host_str) = host.to_str()
        {
            return host_str.ends_with(SUFFIX_COM)
                || host_str.ends_with(SUFFIX_NET)
                || host_str.ends_with(SUFFIX_INFO)
                || host_str.ends_with(SUFFIX_US)
                || host_str.ends_with(SUFFIX_NETWORK);
        }

        false
    }

    fn redirect_path(state: &BmcState) -> &str {
        match *state {
            BmcState::FactoryDefault | BmcState::WifiReconfiguration => {
                HttpServer::<T>::WIFI_SETUP_URL_ENDPOINT
            }
            BmcState::SetupPending => HttpServer::<T>::DEVICE_SETUP_URL_ENDPOINT,
            BmcState::Operational => HttpServer::<T>::ROOT_URL_ENDPOINT,
        }
    }
}

impl<T: BmcManager, S: Clone> Clone for CaptivePortalMiddleware<S, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            manager: self.manager.clone(),
        }
    }
}

impl<S, T> Service<Request<Body>> for CaptivePortalMiddleware<S, T>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Send
        + Clone
        + 'static,
    S::Future: Send + 'static,
    T: BmcManager,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let manager = self.manager.clone();

        Box::pin(async move {
            // If device is in factory default or device setup state and host ends with a given suffix, then return 302 redirect
            let state = manager.device_state().await;

            if Self::should_redirect(&req, &state) {
                let redirect_path = Self::redirect_path(&state);

                let host = manager
                    .captive_portal_redirect_host()
                    .await
                    .unwrap_or_else(|| req.uri().host().unwrap_or_default().to_owned());

                let redirect_uri = format!("http://{host}{redirect_path}");

                let response = Response::builder()
                    .status(302)
                    .header(LOCATION.to_string(), redirect_uri)
                    .body(Body::empty())
                    .expect("BUG: Failed to create response");
                return Ok(response);
            }

            // If no match, forward the request to the next service
            let response = inner.call(req).await?;

            Ok(response)
        })
    }
}
