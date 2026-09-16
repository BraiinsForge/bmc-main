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
    time::{Instant, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Path, Request, State},
    http::HeaderValue,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
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

/// Cap on a request body forwarded to boser; the upgrade image goes over
/// its own endpoint, so anything larger here is a bug or an attack.
const PROXY_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
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

/// Roots of our own single-page-app routes, mirroring `URLS.pages` in
/// `frontend/src/constants.tsx` - keep the two in step when adding a page
/// there. Anything outside this set belongs to boser once it is mounted.
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

/// Whether a path that missed on disk is one of our own single-page-app
/// routes. Boser owns everything else, so this is what decides between
/// answering with index.html and forwarding the request on.
fn spa_owns_route(file_path: &str) -> bool {
    let root = file_path.split('/').next().unwrap_or_default();
    SPA_ROUTE_ROOTS.contains(&root)
}

impl<T: BmcManager> HttpServer<T> {
    const INDEX_PATH: &str = "index.html";
    /// Upper bound on boser's handshake response head.
    const WS_HANDSHAKE_MAX_BYTES: usize = 64 * 1024;
    const PROXY_MAX_BODY_BYTES: usize = PROXY_MAX_BODY_BYTES;
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
        let router = Router::new()
            .merge(self.static_file_router())
            .merge(self.general_api_router())
            .merge(self.widget_icon_router());

        // Requests this binary has no route for belong to boser: it owns the
        // miner API and its own frontend, while :80 is ours on a display
        // device. Without boser the server stays standalone.
        let router = match self.config.boser {
            Some(boser) => router
                .fallback(move |req| Self::proxy_to_boser(boser, req))
                // The static catch-all is GET-only, so a non-GET to any path
                // matches it and axum answers 405 before reaching the
                // fallback. boser's API is mostly POST, so route those too.
                .method_not_allowed_fallback(move |req| Self::proxy_to_boser(boser, req)),
            None => router,
        };

        router
            .layer(CompressionLayer::new())
            .layer(CaptivePortalLayer::new(self.manager.clone()))
            .layer(middleware::from_fn(Self::log_request))
    }

    fn static_file_router(&self) -> Router {
        let www_storage = Storage::new(self.config.www_root_path.clone());
        let var_storage = Storage::new(self.config.www_var_path.clone());
        let assets_storage = Storage::new(self.config.www_assets_path.clone());

        // /var and /assets exist in both frontends. Ours answers first, so a
        // file only boser has (its branding, favicon) would 404 here instead of
        // reaching the proxy. Fall through on a miss so both are reachable.
        let boser = self.config.boser;
        let var_router = Router::new()
            .route(
                "/var/{*file_path}",
                get(move |state, path, request| Self::static_or_boser(boser, state, path, request)),
            )
            .with_state(var_storage);

        let assets_router = Router::new()
            .route(
                "/assets/{*file_path}",
                get(move |state, path, request| Self::static_or_boser(boser, state, path, request)),
            )
            .with_state(assets_storage);

        let index_state = IndexState::new(www_storage, self.manager.clone(), self.config.boser);

        Router::new()
            .route(ROOT_URL_ENDPOINT, get(Self::index_handler))
            .route(WIFI_SETUP_URL_ENDPOINT, get(Self::wifi_setup_index_handler))
            .route(DEVICE_SETUP_URL_ENDPOINT, get(Self::device_setup_handler))
            .route("/{*file_path}", get(Self::file_handler_with_index_fallback))
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
        if let Some(boser) = boser
            && !spa_owned
        {
            return Self::proxy_to_boser(boser, request).await;
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
        boser: Option<std::net::SocketAddr>,
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
    /// opens are upgrades, which an HTTP client cannot forward, so the
    /// handshake is replayed on a raw socket and the two ends are then joined
    /// byte for byte.
    /// Relay a WebSocket to boser. The client's handshake is replayed onto a
    /// raw socket and boser's answer is returned verbatim: the browser needs
    /// boser's own `Sec-WebSocket-Accept` and `Upgrade` headers, and rejects
    /// the connection outright if the 101 arrives without them.
    async fn proxy_websocket(boser: std::net::SocketAddr, request: Request) -> Response {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let (parts, body) = request.into_parts();
        let path_and_query = parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path().to_owned(), ToString::to_string);

        let mut handshake = format!("GET {path_and_query} HTTP/1.1\r\n");
        for (name, value) in &parts.headers {
            if let Ok(value) = value.to_str() {
                use std::fmt::Write as _;
                // Ignore: writing to a String cannot fail.
                let _ = write!(handshake, "{name}: {value}\r\n");
            }
        }
        handshake.push_str("\r\n");

        let mut server = match tokio::net::TcpStream::connect(boser).await {
            Ok(server) => server,
            Err(err) => {
                warn!(%err, "WebSocket connect to boser failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        if let Err(err) = server.write_all(handshake.as_bytes()).await {
            warn!(%err, "WebSocket handshake to boser failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }

        let mut buffered = Vec::new();
        let header_end = loop {
            if let Some(at) = buffered.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            if buffered.len() > Self::WS_HANDSHAKE_MAX_BYTES {
                warn!("boser's WebSocket handshake exceeded the size limit");
                return StatusCode::BAD_GATEWAY.into_response();
            }
            let mut chunk = [0_u8; 1024];
            match server.read(&mut chunk).await {
                Ok(0) => {
                    warn!("boser closed the connection during the WebSocket handshake");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
                Ok(read) => buffered.extend_from_slice(&chunk[..read]),
                Err(err) => {
                    warn!(%err, "Reading boser's WebSocket handshake failed");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            }
        };

        let Ok(head) = std::str::from_utf8(&buffered[..header_end]) else {
            warn!("boser's WebSocket handshake was not valid UTF-8");
            return StatusCode::BAD_GATEWAY.into_response();
        };
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .and_then(|code| StatusCode::from_u16(code).ok());
        let Some(status) = status else {
            warn!("boser's WebSocket handshake had no usable status line");
            return StatusCode::BAD_GATEWAY.into_response();
        };

        let mut response = Response::builder().status(status);
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                response = response.header(name.trim(), value.trim());
            }
        }
        let Ok(response) = response.body(axum::body::Body::empty()) else {
            warn!("Rebuilding boser's WebSocket handshake failed");
            return StatusCode::BAD_GATEWAY.into_response();
        };

        // Anything boser already sent past its handshake belongs to the client.
        let leftover = buffered.split_off(header_end);

        if status == StatusCode::SWITCHING_PROTOCOLS {
            let upgraded = hyper::upgrade::on(Request::from_parts(parts, body));
            tokio::spawn(async move {
                match upgraded.await {
                    Ok(client) => {
                        let mut client = hyper_util::rt::TokioIo::new(client);
                        if !leftover.is_empty()
                            && let Err(err) = client.write_all(&leftover).await
                        {
                            debug!(%err, "Forwarding buffered WebSocket bytes failed");
                            return;
                        }
                        // Runs until either side closes; errors here are just
                        // the connection ending, so they are logged at debug.
                        if let Err(err) =
                            tokio::io::copy_bidirectional(&mut client, &mut server).await
                        {
                            debug!(%err, "WebSocket relay finished");
                        }
                    }
                    Err(err) => warn!(%err, "Client WebSocket upgrade failed"),
                }
            });
        }

        response
    }

    /// Forward a request to boser verbatim and return its response.
    async fn proxy_to_boser(boser: std::net::SocketAddr, request: Request) -> Response {
        if request
            .headers()
            .get(http::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        {
            return Self::proxy_websocket(boser, request).await;
        }

        let (parts, body) = request.into_parts();
        let path_and_query = parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path().to_owned(), ToString::to_string);
        let url = format!("http://{boser}{path_and_query}");

        let body = match axum::body::to_bytes(body, Self::PROXY_MAX_BODY_BYTES).await {
            Ok(body) => body,
            Err(err) => {
                warn!(%err, "Rejecting oversized request for boser");
                return StatusCode::PAYLOAD_TOO_LARGE.into_response();
            }
        };

        // reqwest carries its own `http` major, so the method and headers are
        // rebuilt from bytes rather than moved across.
        let Ok(method) = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()) else {
            return StatusCode::BAD_REQUEST.into_response();
        };

        let client = reqwest::Client::new();
        let mut outgoing = client.request(method, &url).body(body.to_vec());
        for (name, value) in &parts.headers {
            // Rewritten by the client for the new connection.
            if name != http::header::HOST {
                outgoing = outgoing.header(name.as_str(), value.as_bytes());
            }
        }

        match outgoing.send().await {
            Ok(boser_response) => {
                let status = boser_response.status();
                let headers = boser_response.headers().clone();
                // Streamed, not collected: boser answers gRPC-web
                // subscriptions with a body that stays open for the
                // life of the subscription, so waiting for the end of
                // it would hang the request until the client gave up.
                let mut response =
                    Response::new(axum::body::Body::from_stream(boser_response.bytes_stream()));
                *response.status_mut() =
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                for (name, value) in &headers {
                    // The client already decoded the body and the new
                    // framing is ours, so boser's encoding and length
                    // headers would describe bytes that no longer
                    // exist. Hop-by-hop headers do not survive a hop
                    // either. Everything else is passed through.
                    if matches!(
                        name.as_str(),
                        "content-encoding" | "content-length" | "transfer-encoding" | "connection"
                    ) {
                        continue;
                    }
                    if let (Ok(name), Ok(value)) = (
                        http::HeaderName::from_bytes(name.as_str().as_bytes()),
                        http::HeaderValue::from_bytes(value.as_bytes()),
                    ) {
                        response.headers_mut().insert(name, value);
                    }
                }
                response
            }
            Err(err) => {
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
    boser: Option<std::net::SocketAddr>,
}

impl<T: BmcManager> IndexState<T> {
    fn new(storage: Storage, manager: Arc<T>, boser: Option<std::net::SocketAddr>) -> Self {
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
            boser: self.boser,
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
}
