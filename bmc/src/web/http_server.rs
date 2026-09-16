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

use std::{
    net::SocketAddr,
    path::{Component, PathBuf},
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};

use axum::{
    Router,
    body::{Body, HttpBody as _},
    extract::{ConnectInfo, Path, Request, State},
    http::HeaderValue,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use hyper::{
    HeaderMap, StatusCode,
    header::{self, CONTENT_LENGTH},
};
use mime_guess::from_path;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use tower_http::compression::CompressionLayer;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{BmcManager, manager::BmcState, widget::WidgetRegistry};

use super::{ServerConfig, captive_portal::CaptivePortalLayer};

const ZERO: &str = "0";
/// Original peer of a request forwarded to boser, the way boser's own
/// `LuciProxy` reports it.
const X_FORWARDED_FOR: &str = "x-forwarded-for";

const SUPPORT_ARCHIVE_FILENAME_PREFIX: &str = "support_archive_";
// NOTE: the suffix reflects the format the manager implementation produces —
// a standard zip whose entries are password-protected.
const SUPPORT_ARCHIVE_FILENAME_SUFFIX: &str = ".zip";
const SUPPORT_ARCHIVE_STREAM_CAPACITY: usize = 8 * 1024;

/// On-disk icon path for `/widgets/{uid}/icon`, or `None` (→ 404) for a bad uid,
/// unknown widget, or no icon. A missing file 404s later when the handler opens it.
fn widget_icon_path(registry: &WidgetRegistry, uid: &str) -> Option<PathBuf> {
    let uid = Uuid::parse_str(uid).ok()?;
    registry.get(&uid).and_then(|info| info.icon_path)
}

pub(crate) struct HttpServer<T: BmcManager> {
    config: ServerConfig,
    manager: Arc<T>,
    widget_registry: Arc<WidgetRegistry>,
}

pub(crate) const WIFI_SETUP_URL_ENDPOINT: &str = "/init_connect";
pub(crate) const DEVICE_SETUP_URL_ENDPOINT: &str = "/init_setup";
pub(crate) const ROOT_URL_ENDPOINT: &str = "/";

/// Roots of our own single-page-app routes: every route under `URLS.auth` and
/// `URLS.pages` in `frontend/src/constants.tsx`. Anything outside this set
/// belongs to boser once it is mounted. `spa_route_roots_match_the_frontend`
/// reads that file, so a page added there fails the build until it is listed.
const SPA_ROUTE_ROOTS: &[&str] = &[
    "accounts",
    "alarms",
    "display",
    "init_setup",
    "login",
    "network",
    "notifications",
    "price-alerts",
    "settings",
];

/// Headers that only ever apply to one connection (RFC 9110 section 7.6.1);
/// a proxy drops them on both legs, together with whatever `Connection`
/// nominates (see [`connection_nominated`]).
const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// The header names a `Connection` header nominates as hop-by-hop, lower-case.
fn connection_nominated<'a>(connection_values: impl Iterator<Item = &'a [u8]>) -> Vec<String> {
    connection_values
        .flat_map(|value| {
            String::from_utf8_lossy(value)
                .split(',')
                .map(|token| token.trim().to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

/// Whether `name` (lower-case) must not be forwarded across this hop.
fn is_hop_by_hop(name: &str, nominated: &[String]) -> bool {
    HOP_BY_HOP_HEADERS.contains(&name) || nominated.iter().any(|token| token == name)
}

/// A `Vary` value from boser without the `accept-encoding` token: the encoding
/// of a proxied answer is decided by our `CompressionLayer`, which adds its own.
fn vary_without_accept_encoding(value: &[u8]) -> Option<http::HeaderValue> {
    let kept: Vec<&str> = std::str::from_utf8(value)
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|token| !token.eq_ignore_ascii_case("accept-encoding"))
        .collect();
    if kept.is_empty() {
        return None;
    }
    http::HeaderValue::from_str(&kept.join(", ")).ok()
}

/// Whether a path that missed on disk is one of our own single-page-app
/// routes. Boser owns everything else, so this is what decides between
/// answering with index.html and forwarding the request on.
fn spa_owns_route(file_path: &str) -> bool {
    let root = file_path.split('/').next().unwrap_or_default();
    SPA_ROUTE_ROOTS.contains(&root)
}

/// boser behind this server: its address and the one client every forwarded
/// request shares, so connections are pooled and the policy lives in one place.
#[derive(Clone)]
pub(crate) struct BoserProxy {
    addr: SocketAddr,
    client: reqwest::Client,
}

impl BoserProxy {
    /// Bound on connecting to boser. It is on loopback, so anything longer is
    /// boser not accepting, not the network.
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    /// Bound on boser's response head. Deliberately not a total timeout: the
    /// gRPC-web subscription bodies boser streams stay open for their lifetime.
    /// Generous because boser answers some mutations only once they are done:
    /// stopping bosminer on a BMM with a hashboard took 45 s, joining a WiFi
    /// network with an ESP32 reflash about as long, and a system upgrade is
    /// acknowledged only after the image is staged.
    const HEADER_TIMEOUT: Duration = Duration::from_mins(5);

    fn new(addr: SocketAddr) -> Self {
        let client = reqwest::Client::builder()
            // boser's redirects are for the browser to follow, not for us: a
            // followed one would hand back the target's status and body and
            // turn a POST into a GET on the way.
            .redirect(reqwest::redirect::Policy::none())
            // With a decoder compiled in, the client negotiates gzip on its
            // own and decodes behind our back; the proxy strips the browser's
            // `Accept-Encoding` so boser answers identity and our layer
            // compresses once. Nothing here must re-add the negotiation.
            .no_gzip()
            .connect_timeout(Self::CONNECT_TIMEOUT)
            .build()
            .expect("BUG: the boser client configuration is static");
        Self { addr, client }
    }
}

impl<T: BmcManager> HttpServer<T> {
    const INDEX_PATH: &str = "index.html";
    /// Where boser's own frontend is mounted beside ours. Must match
    /// `BASE_PATH` in bos-main `frontend/src/lib/router/basename.ts`
    /// (bos-main!2504): its build detects the basename from the browser's
    /// path, so the browser keeps this prefix while boser is asked without it.
    const BOSER_FRONTEND_PREFIX: &str = "/bos";
    const INITIAL_SETUP_INDEX_FILENAME: &str = "index-connect.html";
    const SUPPORT_ARCHIVE: &str = "/api/get_support_archive";
    const WIDGET_ICON: &str = "/widgets/{uid}/icon";

    pub(crate) fn new(
        config: ServerConfig,
        manager: Arc<T>,
        widget_registry: Arc<WidgetRegistry>,
    ) -> Self {
        Self {
            config,
            manager,
            widget_registry,
        }
    }

    pub(crate) fn build(&self) -> Router {
        self.routes()
            .layer(CompressionLayer::new())
            .layer(CaptivePortalLayer::new(self.manager.clone()))
            .layer(middleware::from_fn(Self::log_request))
    }

    /// Every route, ours and the forwarding to boser, without the layers
    /// [`Self::build`] wraps them in; the router tests drive this directly.
    fn routes(&self) -> Router {
        let boser = self.config.boser.map(BoserProxy::new);
        let router = Router::new()
            .merge(self.static_file_router(boser.clone()))
            .merge(self.general_api_router())
            .merge(self.widget_icon_router());

        // Requests this binary has no route for belong to boser: it owns the
        // miner API and its own frontend, while :80 is ours on a display
        // device. Its frontend is also mounted whole under
        // `BOSER_FRONTEND_PREFIX`, so a user can reach the miner UI from ours.
        // Without boser the server stays standalone.
        match boser {
            Some(boser) => {
                let fallback_boser = boser.clone();
                let mount_boser = boser.clone();
                let mount_rest_boser = boser.clone();
                router
                    .route(
                        Self::BOSER_FRONTEND_PREFIX,
                        any(move |req| Self::proxy_stripped(mount_boser.clone(), req)),
                    )
                    .route(
                        &format!("{}/{{*rest}}", Self::BOSER_FRONTEND_PREFIX),
                        any(move |req| Self::proxy_stripped(mount_rest_boser.clone(), req)),
                    )
                    .fallback(move |req| Self::proxy_to_boser(fallback_boser.clone(), req))
                    // The catch-all dispatches non-GETs itself; this covers the
                    // remaining GET-only routes (index, `/var`, `/assets`), so a
                    // non-GET there still reaches boser instead of a 405.
                    .method_not_allowed_fallback(move |req| {
                        Self::proxy_to_boser(boser.clone(), req)
                    })
            }
            None => router,
        }
    }

    fn static_file_router(&self, boser: Option<BoserProxy>) -> Router {
        let www_storage = Storage::new(self.config.www_root_path.clone());
        let var_storage = Storage::new(self.config.www_var_path.clone());
        let assets_storage = Storage::new(self.config.www_assets_path.clone());

        // /var and /assets exist in both frontends. Ours answers first, so a
        // file only boser has (its branding, favicon) would 404 here instead of
        // reaching the proxy. Fall through on a miss so both are reachable.
        let var_boser = boser.clone();
        let var_router = Router::new()
            .route(
                "/var/{*file_path}",
                get(move |state, path, request| {
                    Self::static_or_boser(var_boser.clone(), state, path, request)
                }),
            )
            .with_state(var_storage);

        let assets_boser = boser.clone();
        let assets_router = Router::new()
            .route(
                "/assets/{*file_path}",
                get(move |state, path, request| {
                    Self::static_or_boser(assets_boser.clone(), state, path, request)
                }),
            )
            .with_state(assets_storage);

        // A non-GET to a path we do not route is boser's: its API is mostly
        // POST. Dispatched here rather than through the router's
        // method-not-allowed fallback, which would stamp axum's
        // `Allow: GET,HEAD` on boser's answer.
        let catch_all = match boser.clone() {
            Some(proxy) => any(
                move |state: State<IndexState<T>>, path: Path<String>, request: Request| {
                    let proxy = proxy.clone();
                    async move {
                        if request.method() == http::Method::GET
                            || request.method() == http::Method::HEAD
                        {
                            Self::file_handler_with_index_fallback(state, path, request)
                                .await
                                .into_response()
                        } else {
                            Self::proxy_to_boser(proxy, request).await
                        }
                    }
                },
            ),
            None => get(Self::file_handler_with_index_fallback),
        };
        let index_state = IndexState::new(www_storage, self.manager.clone(), boser);

        Router::new()
            .route(ROOT_URL_ENDPOINT, get(Self::index_handler))
            .route(WIFI_SETUP_URL_ENDPOINT, get(Self::wifi_setup_index_handler))
            .route(DEVICE_SETUP_URL_ENDPOINT, get(Self::device_setup_handler))
            .route("/{*file_path}", catch_all)
            .with_state(index_state)
            .merge(var_router)
            .merge(assets_router)
    }

    fn general_api_router(&self) -> Router {
        Router::new()
            .route(Self::SUPPORT_ARCHIVE, get(Self::handle_support_archive))
            .with_state(self.manager.clone())
    }

    fn widget_icon_router(&self) -> Router {
        Router::new()
            .route(Self::WIDGET_ICON, get(Self::handle_widget_icon))
            .with_state(self.widget_registry.clone())
    }

    /// Serve a widget's manifest icon. The path is from the trusted install-time
    /// manifest (the uid only selects the widget), so the URL adds no traversal.
    async fn handle_widget_icon(
        State(registry): State<Arc<WidgetRegistry>>,
        Path(uid): Path<String>,
    ) -> impl IntoResponse {
        let Some(icon_path) = widget_icon_path(&registry, &uid) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let Ok(file) = File::open(&icon_path).await else {
            return StatusCode::NOT_FOUND.into_response();
        };

        let filename = icon_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("icon");
        let headers = Storage::get_file_headers(filename, &file).await;
        let body = Body::from_stream(ReaderStream::new(file));
        (headers, body).into_response()
    }

    async fn file_handler_with_index_fallback(
        State(IndexState {
            storage,
            manager,
            boser,
        }): State<IndexState<T>>,
        Path(file_path): Path<String>,
        request: Request,
    ) -> impl IntoResponse {
        let spa_owned = spa_owns_route(&file_path);

        let response = Storage::file_handler(State(storage.clone()), Path(file_path))
            .await
            .into_response();

        if response.status() != StatusCode::NOT_FOUND {
            return response;
        }

        // A miss is either one of our client-side routes, which resolve only
        // once index.html runs, or a path we do not own at all. Route root is
        // the discriminator rather than protocol: boser's API and subscriptions
        // are ordinary GETs, so index.html would shadow them.
        if let Some(boser) = &boser
            && !spa_owned
        {
            return Self::proxy_to_boser(boser.clone(), request).await;
        }

        Self::index_handler(State(IndexState {
            storage,
            manager,
            boser,
        }))
        .await
        .into_response()
    }

    /// Serve a static file, handing the request to boser when we do not have
    /// it. Keeps our own assets authoritative without hiding boser's.
    async fn static_or_boser(
        boser: Option<BoserProxy>,
        State(storage): State<Storage>,
        Path(file_path): Path<String>,
        request: Request,
    ) -> Response {
        let response = Storage::file_handler(State(storage), Path(file_path))
            .await
            .into_response();

        match (response.status(), boser) {
            (StatusCode::NOT_FOUND, Some(boser)) => Self::proxy_to_boser(boser, request).await,
            _ => response,
        }
    }

    /// Relay a WebSocket to boser. The GraphQL subscriptions the frontend
    /// opens are upgrades, which the pooled client cannot forward, so the
    /// handshake goes over a dedicated HTTP/1.1 connection to boser and, once
    /// both sides have switched protocols, the two are joined byte for byte.
    /// boser's answer is returned as it came: the browser needs boser's own
    /// `Sec-WebSocket-Accept` and `Upgrade` headers, and rejects the connection
    /// outright if the 101 arrives without them. A non-101 answer (boser sends
    /// `400` when `Sec-WebSocket-Protocol` is missing) is relayed with its body.
    /// The request goes up the way the plain path sends one: no body framing,
    /// `Host` rewritten to boser and the client's address in `x-forwarded-for`.
    async fn proxy_websocket(boser: SocketAddr, mut request: Request) -> Response {
        let connect = tokio::net::TcpStream::connect(boser);
        let stream = match tokio::time::timeout(BoserProxy::CONNECT_TIMEOUT, connect).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
                warn!(%err, "WebSocket connect to boser failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
            Err(_) => {
                warn!("WebSocket connect to boser timed out");
                return StatusCode::GATEWAY_TIMEOUT.into_response();
            }
        };
        let handshake = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream));
        let (mut sender, connection) = match handshake.await {
            Ok(connection) => connection,
            Err(err) => {
                warn!(%err, "HTTP handshake with boser failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        // The connection task must outlive the response: it carries the
        // upgraded stream once boser answers 101.
        tokio::spawn(async move {
            if let Err(err) = connection.with_upgrades().await {
                debug!(%err, "WebSocket connection to boser ended");
            }
        });

        // The browser's side of the upgrade, taken before the request is
        // consumed; it resolves once our 101 has gone out.
        let client_upgrade = hyper::upgrade::on(&mut request);
        // A handshake is a GET whose body is not part of the protocol, so the
        // body is dropped here and its framing headers go with it below.
        let (parts, _body) = request.into_parts();
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(peer)| peer.ip());
        let path_and_query = parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path().to_owned(), ToString::to_string);
        let mut upstream = http::Request::builder()
            .method(parts.method)
            .uri(path_and_query)
            .version(http::Version::HTTP_11);
        if let Some(headers) = upstream.headers_mut() {
            // Replayed with `Connection`/`Upgrade` kept: they are what makes
            // this a WebSocket handshake rather than a GET. `Host` names boser,
            // as the plain path's client does for its own connection, since
            // this low-level client does not fill it in.
            for (name, value) in &parts.headers {
                if name == http::header::HOST
                    || name == http::header::CONTENT_LENGTH
                    || name == http::header::TRANSFER_ENCODING
                {
                    continue;
                }
                headers.append(name.clone(), value.clone());
            }
            if let Ok(host) = http::HeaderValue::from_str(&boser.to_string()) {
                headers.insert(http::header::HOST, host);
            }
            // Same reason as on the plain path: boser would otherwise see
            // every subscription coming from loopback.
            if let Some(peer) = peer
                && let Ok(value) = http::HeaderValue::from_str(&peer.to_string())
            {
                headers.insert(http::HeaderName::from_static(X_FORWARDED_FOR), value);
            }
        }
        let upstream = match upstream.body(Body::empty()) {
            Ok(upstream) => upstream,
            Err(err) => {
                warn!(%err, "Rebuilding the WebSocket handshake for boser failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };

        let send = sender.send_request(upstream);
        let mut boser_response = match tokio::time::timeout(BoserProxy::HEADER_TIMEOUT, send).await
        {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                warn!(%err, "WebSocket handshake with boser failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
            Err(_) => {
                warn!("boser did not answer the WebSocket handshake in time");
                return StatusCode::GATEWAY_TIMEOUT.into_response();
            }
        };

        if boser_response.status() == StatusCode::SWITCHING_PROTOCOLS {
            let server_upgrade = hyper::upgrade::on(&mut boser_response);
            tokio::spawn(async move {
                match tokio::try_join!(client_upgrade, server_upgrade) {
                    Ok((client, server)) => {
                        let mut client = hyper_util::rt::TokioIo::new(client);
                        let mut server = hyper_util::rt::TokioIo::new(server);
                        // Runs until either side closes; errors here are just
                        // the connection ending, so they are logged at debug.
                        if let Err(err) =
                            tokio::io::copy_bidirectional(&mut client, &mut server).await
                        {
                            debug!(%err, "WebSocket relay finished");
                        }
                    }
                    Err(err) => warn!(%err, "WebSocket upgrade failed"),
                }
            });
        }

        // The status and headers are boser's; the body (empty on a 101, the
        // error page otherwise) is streamed through so the framing headers
        // stay true.
        let (parts, body) = boser_response.into_parts();
        Response::from_parts(parts, Body::new(body))
    }

    /// Forward a request for boser's frontend mount with the prefix removed, so
    /// boser sees root-relative paths, and put it back on any redirect boser
    /// answers with, so the browser stays under the mount.
    async fn proxy_stripped(proxy: BoserProxy, mut request: Request) -> Response {
        let uri = request.uri();
        let rest = uri
            .path()
            .strip_prefix(Self::BOSER_FRONTEND_PREFIX)
            .unwrap_or(uri.path());
        let rest = if rest.is_empty() { "/" } else { rest };
        let query = uri.query().map_or_else(String::new, |q| format!("?{q}"));
        match format!("{rest}{query}").parse() {
            Ok(rewritten) => *request.uri_mut() = rewritten,
            Err(err) => {
                warn!(%err, "Rewriting the boser frontend path failed");
                return StatusCode::BAD_REQUEST.into_response();
            }
        }

        let mut response = Self::proxy_to_boser(proxy, request).await;
        let mounted = response
            .headers()
            .get(header::LOCATION)
            .and_then(|location| location.to_str().ok())
            .filter(|location| {
                location.starts_with('/') && !location.starts_with(Self::BOSER_FRONTEND_PREFIX)
            })
            .and_then(|location| {
                HeaderValue::from_str(&format!("{}{location}", Self::BOSER_FRONTEND_PREFIX)).ok()
            });
        if let Some(mounted) = mounted {
            response.headers_mut().insert(header::LOCATION, mounted);
        }
        response
    }

    /// Forward a request to boser verbatim and return its response.
    async fn proxy_to_boser(proxy: BoserProxy, request: Request) -> Response {
        if request
            .headers()
            .get(http::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        {
            return Self::proxy_websocket(proxy.addr, request).await;
        }

        let (parts, body) = request.into_parts();
        let path_and_query = parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path().to_owned(), ToString::to_string);
        let url = format!("http://{}{path_and_query}", proxy.addr);

        // reqwest carries its own `http` major, so the method and headers are
        // rebuilt from bytes rather than moved across.
        let Ok(method) = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()) else {
            return StatusCode::BAD_REQUEST.into_response();
        };

        // The body is streamed with backpressure, not collected: a client that
        // sends a first message and waits for boser's answer before the next
        // must not deadlock against a proxy waiting for the end of the body,
        // and boser's own limits govern upload sizes. `content-length`, when
        // the client sent one, is forwarded below and frames the stream; a
        // chunked upload is re-chunked by the client. A request that carries
        // no body is sent without one, so a GET does not turn into a chunked
        // GET on the way.
        let mut outgoing = proxy.client.request(method, &url);
        if !body.is_end_stream() {
            outgoing = outgoing.body(reqwest::Body::wrap_stream(body.into_data_stream()));
        }
        let nominated = connection_nominated(
            parts
                .headers
                .get_all(http::header::CONNECTION)
                .iter()
                .map(http::HeaderValue::as_bytes),
        );
        for (name, value) in &parts.headers {
            // Hop-by-hop headers belong to the browser's connection, not
            // boser's (a forwarded `connection: close` would close boser's
            // pooled connection after every request). `Host` is rewritten by
            // the client for the new connection. `Accept-Encoding` stays here
            // on purpose: boser then answers identity and our outer
            // `CompressionLayer` compresses once for the browser, instead of
            // boser compressing, the client decoding and the layer re-encoding
            // on the way out (boser's LuciProxy does the same). It also keeps
            // the answer independent of which decoders happen to be compiled
            // into the client.
            if is_hop_by_hop(name.as_str(), &nominated)
                || name == http::header::HOST
                || name == http::header::ACCEPT_ENCODING
            {
                continue;
            }
            outgoing = outgoing.header(name.as_str(), value.as_bytes());
        }
        // boser's API log and any per-client logic would otherwise see every
        // request coming from loopback.
        if let Some(ConnectInfo(peer)) = parts.extensions.get::<ConnectInfo<SocketAddr>>() {
            outgoing = outgoing.header(X_FORWARDED_FOR, peer.ip().to_string());
        }

        // Bounded on the response head only; see `BoserProxy::HEADER_TIMEOUT`.
        match tokio::time::timeout(BoserProxy::HEADER_TIMEOUT, outgoing.send()).await {
            Err(_) => {
                warn!(
                    url,
                    "boser did not answer within {:?}",
                    BoserProxy::HEADER_TIMEOUT
                );
                StatusCode::GATEWAY_TIMEOUT.into_response()
            }
            Ok(Ok(boser_response)) => {
                let status = boser_response.status();
                let headers = boser_response.headers().clone();
                let nominated = connection_nominated(
                    headers
                        .get_all(reqwest::header::CONNECTION)
                        .iter()
                        .map(reqwest::header::HeaderValue::as_bytes),
                );
                // Streamed, not collected: boser answers gRPC-web
                // subscriptions with a body that stays open for the
                // life of the subscription, so waiting for the end of
                // it would hang the request until the client gave up.
                let mut response =
                    Response::new(axum::body::Body::from_stream(boser_response.bytes_stream()));
                *response.status_mut() =
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                for (name, value) in &headers {
                    // The framing is ours, so boser's length header would
                    // describe bytes that no longer exist, and hop-by-hop
                    // headers (with what `Connection` nominates) do not survive
                    // a hop. Everything else passes through, `content-encoding`
                    // included: with `Accept-Encoding` stripped above boser
                    // answers identity, and if it ever does not, the header
                    // still matches the bytes it labels.
                    if name == reqwest::header::CONTENT_LENGTH
                        || is_hop_by_hop(name.as_str(), &nominated)
                    {
                        continue;
                    }
                    if name == reqwest::header::VARY {
                        if let Some(value) = vary_without_accept_encoding(value.as_bytes()) {
                            response.headers_mut().append(http::header::VARY, value);
                        }
                        continue;
                    }
                    if let (Ok(name), Ok(value)) = (
                        http::HeaderName::from_bytes(name.as_str().as_bytes()),
                        http::HeaderValue::from_bytes(value.as_bytes()),
                    ) {
                        // `append`, not `insert`: boser answers a login with
                        // several `Set-Cookie` fields and only the last would
                        // survive an insert.
                        response.headers_mut().append(name, value);
                    }
                }
                response
            }
            Ok(Err(err)) => {
                warn!(%err, url, "Forwarding to boser failed");
                StatusCode::BAD_GATEWAY.into_response()
            }
        }
    }

    async fn index_handler(State(IndexState { storage, .. }): State<IndexState<T>>) -> Response {
        let mut resp = Storage::file_handler(State(storage), Path(Self::INDEX_PATH.to_owned()))
            .await
            .into_response();

        // NOTE: Add cache-control headers to prevent storing the index file,
        // forcing the browser to always fetch the latest version from the server
        if let Ok(header) = HeaderValue::from_str("no-cache, no-store, must-revalidate") {
            resp.headers_mut().append(header::CACHE_CONTROL, header);
        }

        resp
    }

    async fn wifi_setup_index_handler(
        State(IndexState {
            storage, manager, ..
        }): State<IndexState<T>>,
    ) -> Response {
        let state = manager
            .network_manager()
            .provisioning()
            .device_state()
            .await;
        if !matches!(
            state,
            BmcState::FactoryDefault | BmcState::WifiReconfiguration
        ) {
            return (
                StatusCode::PERMANENT_REDIRECT,
                [(http::header::LOCATION.as_str(), "/")],
            )
                .into_response();
        }

        Storage::file_handler(
            State(storage),
            Path(Self::INITIAL_SETUP_INDEX_FILENAME.to_owned()),
        )
        .await
        .into_response()
    }

    async fn device_setup_handler(
        State(IndexState {
            storage, manager, ..
        }): State<IndexState<T>>,
    ) -> Response {
        if manager
            .network_manager()
            .provisioning()
            .device_state()
            .await
            != BmcState::SetupPending
        {
            return (
                StatusCode::PERMANENT_REDIRECT,
                [(http::header::LOCATION.as_str(), ROOT_URL_ENDPOINT)],
            )
                .into_response();
        }

        Storage::file_handler(State(storage), Path(Self::INDEX_PATH.to_owned()))
            .await
            .into_response()
    }

    async fn log_request(request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let uri = request.uri().to_string();
        let version = format!("{:?}", request.version());

        let client_ip = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.to_string())
            .unwrap_or_default();

        let instant = Instant::now();
        let response = next.run(request).await;
        let latency = instant.elapsed().as_secs_f64();

        let status_code = response.status().as_u16();
        let response_size = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(ZERO)
            .to_owned();

        let formatted_latency = format!("{latency:.6}");

        info!(
            "{} {} {} {} {} {} - {}",
            client_ip, method, uri, version, status_code, response_size, formatted_latency,
        );

        response
    }

    #[expect(clippy::unused_async, reason = "axum handlers must be async")]
    async fn handle_support_archive(State(manager): State<Arc<T>>) -> impl IntoResponse {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%z").to_string();
        let filename = format!(
            "{}_{}{}",
            SUPPORT_ARCHIVE_FILENAME_PREFIX,
            timestamp.as_str(),
            SUPPORT_ARCHIVE_FILENAME_SUFFIX
        );

        let reader = manager.support_archive();
        let body = Body::from_stream(ReaderStream::with_capacity(
            reader,
            SUPPORT_ARCHIVE_STREAM_CAPACITY,
        ));
        let content_disposition = format!("attachment; filename=\"{filename}\"");
        let headers = [
            (CONTENT_TYPE, "application/octet-stream"),
            (CONTENT_DISPOSITION, content_disposition.as_str()),
        ];
        (StatusCode::OK, headers, body).into_response()
    }
}

#[derive(Clone)]
struct Storage {
    mount_path: PathBuf,
}

impl Storage {
    fn new(mount_path: PathBuf) -> Self {
        Self { mount_path }
    }

    fn error_status(error: &std::io::Error) -> StatusCode {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::NOT_FOUND
        }
    }

    async fn get_asset(&self, file_name: &str) -> std::io::Result<File> {
        let file_path = std::path::Path::new(file_name);
        if !file_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "asset path must contain only normal components",
            ));
        }

        let path = self.mount_path.join(file_path);
        let file = File::open(path).await?;
        if !file.metadata().await?.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "asset path must be a regular file",
            ));
        }
        Ok(file)
    }

    async fn file_handler(
        State(storage): State<Storage>,
        Path(file_path): Path<String>,
    ) -> impl IntoResponse {
        let file = match storage.get_asset(&file_path).await {
            Ok(file) => file,
            Err(error) => return Self::error_status(&error).into_response(),
        };

        let headers = Self::get_file_headers(&file_path, &file).await;

        let stream = ReaderStream::new(file);
        let body = Body::from_stream(stream);
        (headers, body).into_response()
    }

    async fn get_file_headers(filename: &str, file: &File) -> HeaderMap {
        let mut headers = HeaderMap::new();

        if let Ok(metadata) = file.metadata().await {
            // Add Last-Modified header
            metadata
                .modified()
                .ok()
                .map(chrono::DateTime::<chrono::Utc>::from)
                .map(|datetime| datetime.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
                .and_then(|formatted| HeaderValue::from_str(&formatted).ok())
                .map(|header| headers.append(header::LAST_MODIFIED, header));

            // Add ETag header
            Self::etag(&metadata)
                .and_then(|etag| HeaderValue::from_str(&etag).ok())
                .map(|header| headers.append(header::ETAG, header));
        }

        // Add Content-Type header
        let mime_type = from_path(filename).first_or_text_plain();

        if let Ok(header) = HeaderValue::from_str(mime_type.as_ref()) {
            headers.append(header::CONTENT_TYPE, header);
        }

        // Add Content-Disposition header
        if let Ok(header) = HeaderValue::from_str(&format!("inline; filename=\"{filename}\"")) {
            headers.append(header::CONTENT_DISPOSITION, header);
        }

        headers
    }

    // NOTE: Taken from https://github.com/actix/actix-web/blob/0ef246a846f478e8d85ad441ab979e13c010d152/actix-files/src/named.rs#L384
    // Copyright (c) the actix-web authors; licensed under MIT OR Apache-2.0,
    // used here under the MIT license.
    fn etag(metadata: &std::fs::Metadata) -> Option<String> {
        let modified = metadata.modified().ok();

        modified.as_ref().map(|mtime| {
            let ino = {
                #[cfg(unix)]
                {
                    #[cfg(unix)]
                    use std::os::unix::fs::MetadataExt as _;

                    metadata.ino()
                }

                #[cfg(not(unix))]
                {
                    0
                }
            };

            let dur = mtime.duration_since(UNIX_EPOCH).unwrap_or_default();

            format!(
                "{:x}:{:x}:{:x}:{:x}",
                ino,
                metadata.len(),
                dur.as_secs(),
                dur.subsec_nanos()
            )
        })
    }
}

struct IndexState<T: BmcManager> {
    storage: Storage,
    manager: Arc<T>,
    boser: Option<BoserProxy>,
}

impl<T: BmcManager> IndexState<T> {
    fn new(storage: Storage, manager: Arc<T>, boser: Option<BoserProxy>) -> Self {
        Self {
            storage,
            manager,
            boser,
        }
    }
}

impl<T: BmcManager> Clone for IndexState<T> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            manager: self.manager.clone(),
            boser: self.boser.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;
    use crate::widget::{WidgetInfo, WidgetRegistry};
    use tower::ServiceExt as _;

    /// boser's REST API must not be shadowed by the SPA fallback: answering
    /// it with index.html returns HTML under status 200, which the fleet widget
    /// reads as a miner doing 0 TH/s.
    #[test]
    fn boser_paths_are_not_claimed_by_the_spa_fallback() {
        for path in [
            "api/v1/miner/stats",
            "api/v1/miner/hw/hashboards",
            "api/v1/miner/details",
            "api/v1/version",
            "graphql",
        ] {
            assert!(
                !spa_owns_route(path),
                "boser owns /{path}, it must be forwarded"
            );
        }

        for path in [
            "display",
            "display/scene-1",
            "settings",
            "login",
            "network",
            "alarms",
            "price-alerts",
            "notifications",
            "accounts",
            "init_setup",
        ] {
            assert!(
                spa_owns_route(path),
                "/{path} is our own route and only resolves once index.html runs"
            );
        }
    }

    #[tokio::test]
    async fn file_router_rejects_traversal_as_bad_request() {
        let temp = tempfile::tempdir().expect("BUG: create temporary directory");
        let router = Router::new()
            .route("/{*file_path}", get(Storage::file_handler))
            .with_state(Storage::new(temp.path().to_path_buf()));

        for uri in [
            "/../../secret",
            "/%2e%2e/%2e%2e/secret",
            "/%2e%2e%2f%2e%2e%2fsecret",
        ] {
            let request = Request::get(uri)
                .body(Body::empty())
                .expect("BUG: build traversal request");
            let response = router
                .clone()
                .oneshot(request)
                .await
                .expect("BUG: router should respond");

            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "URI: {uri}");
        }
    }

    #[tokio::test]
    async fn storage_rejects_paths_outside_mount() {
        let temp = tempfile::tempdir().expect("BUG: create temporary directory");
        let mount = temp.path().join("www");
        let nested = mount.join("nested");
        std::fs::create_dir_all(&nested).expect("BUG: create nested asset directory");
        std::fs::write(nested.join("asset"), "asset").expect("BUG: write nested asset");

        let outside = temp.path().join("secret");
        std::fs::write(&outside, "secret").expect("BUG: write file outside web root");

        let storage = Storage::new(mount);
        storage
            .get_asset("nested/asset")
            .await
            .expect("nested asset should open");

        for path in [
            "../secret".to_owned(),
            "nested/../../secret".to_owned(),
            outside.to_string_lossy().into_owned(),
        ] {
            let error = storage
                .get_asset(&path)
                .await
                .expect_err("path outside web root must be rejected");
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    #[tokio::test]
    async fn storage_rejects_directory_requests_as_not_found() {
        let temp = tempfile::tempdir().expect("BUG: create temporary directory");
        let mount = temp.path().join("www");
        let nested = mount.join("nested");
        std::fs::create_dir_all(&nested).expect("BUG: create nested asset directory");
        std::fs::write(nested.join("asset"), "asset").expect("BUG: write nested asset");

        let storage = Storage::new(mount);
        for path in ["nested", "nested/"] {
            let error = storage
                .get_asset(path)
                .await
                .expect_err("directory must not be served");
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "path: {path}");
        }
    }

    fn registry_with_icon(uid: Uuid, icon_path: Option<PathBuf>) -> WidgetRegistry {
        let json = format!(
            r#"{{
                "uid": "{uid}",
                "version": "1.0.0",
                "name": "T",
                "description": "T",
                "binary": "bin/test",
                "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}]
            }}"#
        );
        let manifest = bmc_widget_manifest::Manifest::from_str(&json).expect("BUG: valid manifest");
        WidgetRegistry::new(vec![WidgetInfo::for_test(
            manifest,
            PathBuf::from("/widgets/t"),
            PathBuf::from("/widgets/t/bin/test"),
            icon_path,
        )])
    }

    #[test]
    fn widget_icon_path_rejects_malformed_uid() {
        let registry = registry_with_icon(Uuid::new_v4(), Some(PathBuf::from("/icon.svg")));
        assert!(widget_icon_path(&registry, "not-a-uuid").is_none());
    }

    #[test]
    fn widget_icon_path_unknown_uid_is_none() {
        let registry = registry_with_icon(Uuid::new_v4(), Some(PathBuf::from("/icon.svg")));
        assert!(widget_icon_path(&registry, &Uuid::new_v4().to_string()).is_none());
    }

    #[test]
    fn widget_icon_path_returns_icon_for_known_widget() {
        let uid = Uuid::new_v4();
        let registry = registry_with_icon(uid, Some(PathBuf::from("/widgets/t/icon.svg")));
        assert_eq!(
            widget_icon_path(&registry, &uid.to_string()),
            Some(PathBuf::from("/widgets/t/icon.svg"))
        );
    }

    #[test]
    fn widget_icon_path_none_when_widget_has_no_icon() {
        let uid = Uuid::new_v4();
        let registry = registry_with_icon(uid, None);
        assert!(widget_icon_path(&registry, &uid.to_string()).is_none());
    }
    /// The roots of every route literal in the `auth` and `pages` blocks of
    /// `constants.tsx`: the first path segment of each `'/...'` string. A plain
    /// scan, precise enough to fail when someone adds a page.
    fn frontend_route_roots(constants: &str) -> Vec<String> {
        let mut roots = Vec::new();
        for block in ["auth: {", "pages: {"] {
            let start = constants
                .find(block)
                .unwrap_or_else(|| panic!("BUG: constants.tsx has no `{block}` block"));
            let body = constants
                .get(start..)
                .expect("BUG: `find` returned an offset inside the file");
            let end = body
                .find("\n    },")
                .expect("BUG: the block never closes at the URLS indentation");
            let block = body
                .get(..end)
                .expect("BUG: `find` returned an offset inside the block");
            for literal in block.split('\'').skip(1).step_by(2) {
                let Some(path) = literal.strip_prefix('/') else {
                    continue;
                };
                let root = path.split(['/', ':']).next().unwrap_or_default();
                if !root.is_empty() {
                    roots.push(root.to_owned());
                }
            }
        }
        roots
    }

    #[test]
    fn spa_route_roots_match_the_frontend() {
        let roots = frontend_route_roots(include_str!("../../../frontend/src/constants.tsx"));
        assert!(
            roots.iter().any(|root| root == "login") && roots.iter().any(|root| root == "settings"),
            "the scan found no routes: {roots:?}"
        );
        for root in &roots {
            assert!(
                SPA_ROUTE_ROOTS.contains(&root.as_str()),
                "/{root} is in constants.tsx but not in SPA_ROUTE_ROOTS"
            );
        }
        for root in SPA_ROUTE_ROOTS {
            assert!(
                roots.iter().any(|found| found == root),
                "/{root} is in SPA_ROUTE_ROOTS but no longer in constants.tsx"
            );
        }
    }

    /// The forwarding path driven end to end against a stub boser.
    mod boser_proxy {
        use std::net::Ipv4Addr;

        use axum::routing::post;
        use http::header::{
            ACCEPT_ENCODING, CONNECTION, CONTENT_ENCODING, LOCATION, SET_COOKIE, UPGRADE,
        };
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::{TcpListener, TcpStream};

        use super::*;
        use crate::test_support::StubManager;

        /// `hello from boser`, gzip-compressed.
        const GZIP_HELLO: &[u8] = &[
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xcb, 0x48, 0xcd, 0xc9,
            0xc9, 0x57, 0x48, 0x2b, 0xca, 0xcf, 0x55, 0x48, 0xca, 0x2f, 0x4e, 0x2d, 0x02, 0x00,
            0x19, 0x3c, 0x40, 0xaf, 0x10, 0x00, 0x00, 0x00,
        ];

        /// A stand-in boser on loopback answering the shapes the frontend and
        /// the fleet widget use, plus the edge cases the relay has to preserve.
        async fn stub_boser() -> SocketAddr {
            let app = Router::new()
                .route(
                    "/graphql",
                    post(|body: String| async move { format!("graphql:{body}") }),
                )
                .route(
                    "/api/v1/miner/stats",
                    get(|| async { ([(CONTENT_TYPE, "application/json")], r#"{"hashrate":1}"#) }),
                )
                .route(
                    "/api/v1/login",
                    // Built by hand: the array form of `IntoResponse` inserts
                    // headers, so only one cookie would survive the stub itself.
                    post(|| async {
                        Response::builder()
                            .header(SET_COOKIE, "session=1; Path=/")
                            .header(SET_COOKIE, "refresh=2; Path=/")
                            .body(Body::from("logged in"))
                            .expect("BUG: build the login answer")
                    }),
                )
                .route(
                    "/redirect",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/elsewhere")]) }),
                )
                .route(
                    "/gzip",
                    get(|| async { ([(CONTENT_ENCODING, "gzip")], GZIP_HELLO) }),
                )
                .route(
                    "/vary",
                    get(|| async {
                        Response::builder()
                            .header(http::header::VARY, "accept-encoding")
                            .header(http::header::VARY, "Origin, Accept-Encoding")
                            .body(Body::from("varied"))
                            .expect("BUG: build the vary answer")
                    }),
                )
                .route(
                    "/ws",
                    get(|| async { (StatusCode::BAD_REQUEST, "missing Sec-WebSocket-Protocol") }),
                )
                .route(
                    "/headers",
                    get(|headers: HeaderMap| async move {
                        let value = |name: &str| {
                            headers
                                .get(name)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or("none")
                                .to_owned()
                        };
                        format!(
                            "accept-encoding={} x-forwarded-for={} connection={}",
                            value("accept-encoding"),
                            value("x-forwarded-for"),
                            value("connection"),
                        )
                    }),
                )
                .route("/assets/{*path}", get(|| async { "boser asset" }))
                .route(
                    "/",
                    get(|request: Request| async move { format!("root:{}", request.uri()) }),
                )
                .route(
                    "/echo-path",
                    get(|request: Request| async move { format!("path:{}", request.uri()) }),
                )
                .route(
                    "/upgrade",
                    get(|request: Request| async move {
                        // Echo on the upgraded stream, no WebSocket framing:
                        // the relay only moves bytes.
                        let on_upgrade = hyper::upgrade::on(request);
                        tokio::spawn(async move {
                            let Ok(upgraded) = on_upgrade.await else {
                                return;
                            };
                            let mut io = hyper_util::rt::TokioIo::new(upgraded);
                            let mut buf = [0_u8; 64];
                            if let Ok(read) = io.read(&mut buf).await {
                                let _ = io.write_all(&buf[..read]).await;
                            }
                        });
                        (
                            StatusCode::SWITCHING_PROTOCOLS,
                            [(UPGRADE, "websocket"), (CONNECTION, "Upgrade")],
                        )
                    }),
                );
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("BUG: bind the stub boser");
            let addr = listener.local_addr().expect("BUG: stub boser address");
            tokio::spawn(async move {
                axum::serve(listener, app)
                    .await
                    .expect("BUG: the stub boser stopped serving");
            });
            addr
        }

        /// Our routes with the stub boser behind them, over a `www` root that
        /// holds `index.html` and one asset of ours. The directory is returned
        /// so it lives as long as the router.
        async fn proxy_router() -> (Router, tempfile::TempDir) {
            let www = tempfile::tempdir().expect("BUG: create the www root");
            std::fs::write(www.path().join("index.html"), "<html>bmc</html>")
                .expect("BUG: write index.html");
            std::fs::create_dir_all(www.path().join("assets")).expect("BUG: create assets");
            std::fs::write(www.path().join("assets/ours.css"), "body{}")
                .expect("BUG: write our asset");
            std::fs::create_dir_all(www.path().join("var")).expect("BUG: create var");
            let config = ServerConfig {
                www_root_path: www.path().to_path_buf(),
                www_assets_path: www.path().join("assets"),
                www_var_path: www.path().join("var"),
                boser: Some(stub_boser().await),
            };
            let server = HttpServer::new(
                config,
                Arc::new(StubManager),
                Arc::new(WidgetRegistry::new(Vec::new())),
            );
            (server.routes(), www)
        }

        async fn send(router: &Router, request: Request) -> (StatusCode, HeaderMap, Vec<u8>) {
            let response = router
                .clone()
                .oneshot(request)
                .await
                .expect("BUG: the router always answers");
            let (parts, body) = response.into_parts();
            let body = axum::body::to_bytes(body, usize::MAX)
                .await
                .expect("BUG: read the response body");
            (parts.status, parts.headers, body.to_vec())
        }

        fn request(method: &str, uri: &str, body: &str) -> Request {
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::from(body.to_owned()))
                .expect("BUG: build the request")
        }

        #[tokio::test]
        async fn a_post_to_an_unrouted_path_reaches_boser_without_an_allow_header() {
            let (router, _www) = proxy_router().await;
            let (status, headers, body) = send(&router, request("POST", "/graphql", "{q}")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, b"graphql:{q}");
            // axum stamps `Allow` on answers from a method-not-allowed
            // fallback; boser's answer to a POST must not carry one.
            assert!(headers.get(http::header::ALLOW).is_none());
        }

        #[tokio::test]
        async fn boser_s_vary_on_the_encoding_is_left_to_the_compression_layer() {
            let (router, _www) = proxy_router().await;
            let (status, headers, _) = send(&router, request("GET", "/vary", "")).await;
            assert_eq!(status, StatusCode::OK);
            let vary: Vec<&[u8]> = headers
                .get_all(http::header::VARY)
                .iter()
                .map(HeaderValue::as_bytes)
                .collect();
            assert_eq!(vary, vec![b"Origin".as_slice()]);
        }

        #[tokio::test]
        async fn a_get_miss_reaches_boser_while_our_routes_stay_ours() {
            let (router, _www) = proxy_router().await;
            let (status, headers, body) =
                send(&router, request("GET", "/api/v1/miner/stats", "")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, br#"{"hashrate":1}"#);
            assert_eq!(
                headers.get(CONTENT_TYPE).map(HeaderValue::as_bytes),
                Some(b"application/json".as_slice())
            );

            let (status, _, body) = send(&router, request("GET", "/settings", "")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, b"<html>bmc</html>");

            let (_, _, body) = send(&router, request("GET", "/assets/ours.css", "")).await;
            assert_eq!(body, b"body{}");
            let (_, _, body) = send(&router, request("GET", "/assets/boser.png", "")).await;
            assert_eq!(body, b"boser asset");
        }

        #[tokio::test]
        async fn the_boser_frontend_mount_strips_the_prefix_and_keeps_redirects_under_it() {
            let (router, _www) = proxy_router().await;
            let (status, _, body) = send(&router, request("GET", "/bos", "")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, b"root:/");

            let (_, _, body) = send(&router, request("GET", "/bos/echo-path?x=1", "")).await;
            assert_eq!(body, b"path:/echo-path?x=1");

            // boser's own SPA fallback answers the rest, so a miss under the
            // mount must reach boser rather than our index.html.
            let (_, _, body) = send(&router, request("GET", "/bos/assets/logo.png", "")).await;
            assert_eq!(body, b"boser asset");

            let (status, headers, _) = send(&router, request("GET", "/bos/redirect", "")).await;
            assert_eq!(status, StatusCode::FOUND);
            assert_eq!(
                headers.get(LOCATION).map(HeaderValue::as_bytes),
                Some(b"/bos/elsewhere".as_slice())
            );
        }

        #[tokio::test]
        async fn every_set_cookie_survives_and_redirects_pass_through() {
            let (router, _www) = proxy_router().await;
            let (status, headers, _) = send(&router, request("POST", "/api/v1/login", "")).await;
            assert_eq!(status, StatusCode::OK);
            let cookies: Vec<_> = headers
                .get_all(SET_COOKIE)
                .iter()
                .map(|value| value.to_str().unwrap_or_default().to_owned())
                .collect();
            assert_eq!(cookies, ["session=1; Path=/", "refresh=2; Path=/"]);

            let (status, headers, _) = send(&router, request("GET", "/redirect", "")).await;
            assert_eq!(status, StatusCode::FOUND);
            assert_eq!(
                headers.get(LOCATION).map(HeaderValue::as_bytes),
                Some(b"/elsewhere".as_slice())
            );
        }

        #[tokio::test]
        async fn a_compressed_answer_stays_consistent_with_its_header() {
            let (router, _www) = proxy_router().await;
            let (status, headers, body) = send(&router, request("GET", "/gzip", "")).await;
            assert_eq!(status, StatusCode::OK);
            // Either the client decoded it (and dropped the header) or it did
            // not touch it; a decoded body labelled gzip is the bug.
            match headers.get(CONTENT_ENCODING).map(HeaderValue::as_bytes) {
                None => assert_eq!(body, b"hello from boser"),
                Some(b"gzip") => assert_eq!(body, GZIP_HELLO),
                Some(other) => panic!("unexpected content-encoding {other:?}"),
            }
        }

        #[tokio::test]
        async fn hop_by_hop_and_accept_encoding_stop_here_and_the_peer_is_forwarded() {
            let (router, _www) = proxy_router().await;
            let mut request = request("GET", "/headers", "");
            request
                .headers_mut()
                .insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, br"));
            request
                .headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("close"));
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from((
                    Ipv4Addr::new(10, 0, 0, 9),
                    51_000,
                ))));
            let (_, _, body) = send(&router, request).await;
            assert_eq!(
                body,
                b"accept-encoding=none x-forwarded-for=10.0.0.9 connection=none"
            );
        }

        #[tokio::test]
        async fn a_refused_websocket_handshake_is_relayed_with_its_body() {
            let (router, _www) = proxy_router().await;
            let mut request = request("GET", "/ws", "");
            request
                .headers_mut()
                .insert(UPGRADE, HeaderValue::from_static("websocket"));
            request
                .headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("Upgrade"));
            let (status, headers, body) = send(&router, request).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body, b"missing Sec-WebSocket-Protocol");
            assert_eq!(
                headers.get(CONTENT_LENGTH).map(HeaderValue::as_bytes),
                Some(body.len().to_string().as_bytes())
            );
        }

        #[tokio::test]
        async fn an_accepted_upgrade_joins_both_ends() {
            let (router, _www) = proxy_router().await;
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("BUG: bind the router");
            let addr = listener.local_addr().expect("BUG: router address");
            tokio::spawn(async move {
                axum::serve(listener, router)
                    .await
                    .expect("BUG: the router stopped serving");
            });

            let mut client = TcpStream::connect(addr)
                .await
                .expect("BUG: connect to the router");
            client
                .write_all(
                    b"GET /upgrade HTTP/1.1\r\nHost: bmc\r\nConnection: Upgrade\r\n\
                      Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
                      Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
                )
                .await
                .expect("BUG: send the handshake");
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0_u8; 1];
                assert_eq!(
                    client
                        .read(&mut byte)
                        .await
                        .expect("BUG: read the handshake"),
                    1,
                    "the router closed before answering the handshake"
                );
                head.push(byte[0]);
            }
            assert!(
                head.starts_with(b"HTTP/1.1 101"),
                "{}",
                String::from_utf8_lossy(&head)
            );

            client
                .write_all(b"ping")
                .await
                .expect("BUG: send on the relay");
            let mut echoed = [0_u8; 4];
            client
                .read_exact(&mut echoed)
                .await
                .expect("BUG: read from the relay");
            assert_eq!(&echoed, b"ping");
        }
    }
}
