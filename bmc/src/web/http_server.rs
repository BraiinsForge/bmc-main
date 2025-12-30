// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    net::SocketAddr,
    path::PathBuf,
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
use bmc_support::SupportArchiveFormat;
use http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use hyper::{
    HeaderMap, StatusCode,
    header::{self, CONTENT_LENGTH},
};
use mime_guess::from_path;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use tower_http::compression::CompressionLayer;
use tracing::info;

use crate::{BmcManager, manager::BmcState};

use super::{ServerConfig, captive_portal::CaptivePortalLayer};

const ZERO: &str = "0";
const SUPPORT_ARCHIVE_FILENAME_PREFIX: &str = "support_archive_";
const SUPPORT_ARCHIVE_FILENAME_SUFFIX: &str = ".zip.enc";
const SUPPORT_ARCHIVE_FORMAT: SupportArchiveFormat = SupportArchiveFormat::ZipEncrypted;

pub(crate) struct HttpServer<T: BmcManager> {
    config: ServerConfig,
    manager: Arc<T>,
}

impl<T: BmcManager> HttpServer<T> {
    const INDEX_PATH: &str = "index.html";
    const INITIAL_SETUP_INDEX_FILENAME: &str = "index-connect.html";
    pub(crate) const WIFI_SETUP_URL_ENDPOINT: &str = "/init_connect";
    pub(crate) const DEVICE_SETUP_URL_ENDPOINT: &str = "/init_setup";
    pub(crate) const ROOT_URL_ENDPOINT: &str = "/";
    const SUPPORT_ARCHIVE: &str = "/api/get_support_archive";

    pub(crate) fn new(config: ServerConfig, manager: Arc<T>) -> Self {
        Self { config, manager }
    }

    pub(crate) fn build(&self) -> Router {
        Router::new()
            .merge(self.static_file_router())
            .merge(self.general_api_router())
            .layer(CompressionLayer::new())
            .layer(CaptivePortalLayer::new(self.manager.clone()))
            .layer(middleware::from_fn(Self::log_request))
    }

    fn static_file_router(&self) -> Router {
        let www_storage = Storage::new(self.config.www_root_path.clone());
        let var_storage = Storage::new(self.config.www_var_path.clone());
        let assets_storage = Storage::new(self.config.www_assets_path.clone());

        let var_router = Router::new()
            .route("/var/{*file_path}", get(Self::file_handler))
            .with_state(var_storage);

        let assets_router = Router::new()
            .route("/assets/{*file_path}", get(Self::file_handler))
            .with_state(assets_storage);

        let index_state = IndexState::new(www_storage, self.manager.clone());

        Router::new()
            .route(Self::ROOT_URL_ENDPOINT, get(Self::index_handler))
            .route(
                Self::WIFI_SETUP_URL_ENDPOINT,
                get(Self::wifi_setup_index_handler),
            )
            .route(
                Self::DEVICE_SETUP_URL_ENDPOINT,
                get(Self::device_setup_handler),
            )
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

    async fn file_handler_with_index_fallback(
        State(IndexState { storage, manager }): State<IndexState<T>>,
        Path(file_path): Path<String>,
    ) -> impl IntoResponse {
        let response = Self::file_handler(State(storage.clone()), Path(file_path))
            .await
            .into_response();

        if response.status() == StatusCode::NOT_FOUND {
            Self::index_handler(State(IndexState { storage, manager }))
                .await
                .into_response()
        } else {
            response
        }
    }

    async fn file_handler(
        State(storage): State<Storage>,
        Path(file_path): Path<String>,
    ) -> impl IntoResponse {
        let Ok(file) = storage.get_asset(&file_path).await else {
            return StatusCode::NOT_FOUND.into_response();
        };

        let headers = Self::get_file_headers(&file_path, &file).await;

        let stream = ReaderStream::new(file);
        let body = Body::from_stream(stream);
        (headers, body).into_response()
    }

    async fn index_handler(State(IndexState { storage, .. }): State<IndexState<T>>) -> Response {
        let mut resp = Self::file_handler(State(storage), Path(Self::INDEX_PATH.to_owned()))
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
        State(IndexState { storage, manager }): State<IndexState<T>>,
    ) -> Response {
        let state = manager.device_state().await;
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

        Self::file_handler(
            State(storage),
            Path(Self::INITIAL_SETUP_INDEX_FILENAME.to_owned()),
        )
        .await
        .into_response()
    }

    async fn device_setup_handler(
        State(IndexState { storage, manager }): State<IndexState<T>>,
    ) -> Response {
        if manager.device_state().await != BmcState::SetupPending {
            return (
                StatusCode::PERMANENT_REDIRECT,
                [(http::header::LOCATION.as_str(), Self::ROOT_URL_ENDPOINT)],
            )
                .into_response();
        }

        Self::file_handler(State(storage), Path(Self::INDEX_PATH.to_owned()))
            .await
            .into_response()
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

    async fn handle_support_archive(State(manager): State<Arc<T>>) -> impl IntoResponse {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%z").to_string();
        let filename = format!(
            "{}_{}{}",
            SUPPORT_ARCHIVE_FILENAME_PREFIX,
            timestamp.as_str(),
            SUPPORT_ARCHIVE_FILENAME_SUFFIX
        );

        match manager.support_archive(SUPPORT_ARCHIVE_FORMAT).await {
            Ok(data) => {
                let content_disposition = format!("attachment; filename=\"{filename}\"");
                let headers = [
                    (CONTENT_TYPE, "application/octet-stream"),
                    (CONTENT_DISPOSITION, content_disposition.as_str()),
                ];
                (StatusCode::OK, headers, data).into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
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

    async fn get_asset(&self, file_name: &str) -> std::io::Result<File> {
        let path = self.mount_path.join(file_name);
        File::open(path).await
    }
}

struct IndexState<T: BmcManager> {
    storage: Storage,
    manager: Arc<T>,
}

impl<T: BmcManager> IndexState<T> {
    fn new(storage: Storage, manager: Arc<T>) -> Self {
        Self { storage, manager }
    }
}

impl<T: BmcManager> Clone for IndexState<T> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            manager: self.manager.clone(),
        }
    }
}
