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

use super::http_server::{DEVICE_SETUP_URL_ENDPOINT, ROOT_URL_ENDPOINT, WIFI_SETUP_URL_ENDPOINT};

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

/// Whether `state` runs the setup AP and its captive portal.
///
/// Only there does dnsmasq hijack DNS, so only there can a `Host` be a name
/// the client never aimed at this device, and only there does anything hold
/// the AP's own address.
fn runs_captive_portal(state: BmcState) -> bool {
    match state {
        BmcState::FactoryDefault | BmcState::WifiReconfiguration => true,
        BmcState::SetupPending | BmcState::Operational => false,
    }
}

// NOTE: Original list of urls to return redirect is here: https://captivebehavior.wballiance.com/
// It is not needed to check individual url, it can be decided based on the top level domain
fn should_redirect(req: &Request<Body>, state: BmcState) -> bool {
    if state == BmcState::Operational {
        return false;
    }

    // This is covering a case when user displays the main page in browser. Initial setup needs to be displayed instead of the login page
    let uri_path = req.uri().path();

    match (state, uri_path) {
        (_, ROOT_URL_ENDPOINT)
        | (BmcState::FactoryDefault | BmcState::WifiReconfiguration, DEVICE_SETUP_URL_ENDPOINT)
        | (BmcState::SetupPending, WIFI_SETUP_URL_ENDPOINT) => {
            return true;
        }
        _ => (),
    }

    // Without a portal there is no hijack, so a suffix says nothing
    // about where the request was aimed.
    if !runs_captive_portal(state) {
        return false;
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

/// Where a redirected request is sent, absolute while the captive portal runs.
///
/// The hijack can put any name in `Host`, so only naming the device
/// points the client back at it.
/// Elsewhere the request arrived on an address the client picked,
/// and a relative `Location` resolves against it.
/// That is also the only answer left when the AP address is unknown.
fn redirect_location(state: BmcState, ap_address: Option<String>) -> String {
    let path = redirect_path(state);
    match ap_address {
        Some(host) if runs_captive_portal(state) => format!("http://{host}{path}"),
        Some(_) | None => path.to_owned(),
    }
}

fn redirect_path(state: BmcState) -> &'static str {
    match state {
        BmcState::FactoryDefault | BmcState::WifiReconfiguration => WIFI_SETUP_URL_ENDPOINT,
        BmcState::SetupPending => DEVICE_SETUP_URL_ENDPOINT,
        BmcState::Operational => ROOT_URL_ENDPOINT,
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
            let state = manager
                .network_manager()
                .provisioning()
                .device_state()
                .await;

            if should_redirect(&req, state) {
                let ap_address = match manager.network_manager().wifi() {
                    Some(wifi) => wifi.captive_portal_redirect_host().await,
                    None => None,
                };
                let location = redirect_location(state, ap_address);

                let response = Response::builder()
                    .status(302)
                    .header(LOCATION.to_string(), location)
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

#[cfg(test)]
mod tests {
    use super::{
        DEVICE_SETUP_URL_ENDPOINT, WIFI_SETUP_URL_ENDPOINT, redirect_location, should_redirect,
    };
    use crate::manager::BmcState;
    use axum::body::Body;
    use hyper::Request;

    fn request(host: &str, path: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .header("Host", host)
            .body(Body::empty())
            .expect("BUG: failed to build a test request")
    }

    #[test]
    fn a_probe_domain_is_redirected_whatever_the_path() {
        // A relative Location would resolve against the hijacked name.
        for path in ["/generate_204", "/"] {
            assert!(
                should_redirect(
                    &request("connectivitycheck.gstatic.com", path),
                    BmcState::FactoryDefault
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn the_setup_ap_names_the_device_in_its_location() {
        assert_eq!(
            redirect_location(BmcState::FactoryDefault, Some("10.0.0.21".to_owned())),
            "http://10.0.0.21/init_connect"
        );
    }

    #[test]
    fn an_unknown_ap_address_falls_back_to_the_path() {
        // Every other answer would name an address nothing holds.
        assert_eq!(
            redirect_location(BmcState::WifiReconfiguration, None),
            WIFI_SETUP_URL_ENDPOINT
        );
    }

    #[test]
    fn without_a_portal_the_client_keeps_the_address_it_used() {
        assert!(should_redirect(
            &request("10.0.0.21", WIFI_SETUP_URL_ENDPOINT),
            BmcState::SetupPending
        ));
        assert_eq!(
            redirect_location(BmcState::SetupPending, Some("10.0.0.21".to_owned())),
            DEVICE_SETUP_URL_ENDPOINT
        );
    }

    #[test]
    fn without_a_portal_a_suffix_means_nothing() {
        // Nothing hijacks DNS here.
        // A .com Host is then a name that genuinely points at this device,
        // and the AP address is on no interface.
        assert!(!should_redirect(
            &request("deck.company.com", "/asset.js"),
            BmcState::SetupPending
        ));
    }

    #[test]
    fn an_operational_device_hijacks_nothing() {
        assert!(!should_redirect(
            &request("connectivitycheck.gstatic.com", "/generate_204"),
            BmcState::Operational
        ));
    }

    #[test]
    fn a_setup_endpoint_the_state_serves_is_left_alone() {
        assert!(!should_redirect(
            &request("10.0.0.21", DEVICE_SETUP_URL_ENDPOINT),
            BmcState::SetupPending
        ));
    }
}
