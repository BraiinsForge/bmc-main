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
use hyper::{
    HeaderMap, StatusCode,
    header::{self, CONTENT_LENGTH},
};
use mime_guess::from_path;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use tower_http::compression::CompressionLayer;
use tracing::info;

use crate::BmcManager;

use super::ServerConfig;

const INDEX_PATH: &str = "index.html";
const ZERO: &str = "0";

pub(crate) struct HttpServer<T: BmcManager> {
    config: ServerConfig,
    manager: Arc<T>,
}

impl<T: BmcManager> HttpServer<T> {
    pub(crate) fn new(config: ServerConfig, manager: Arc<T>) -> Self {
        Self { config, manager }
    }

    pub(crate) fn build(&self) -> Router {
        Router::new()
            .merge(self.static_file_router())
            .layer(CompressionLayer::new())
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

        Router::new()
            .route("/", get(Self::index_handler))
            .route("/{*file_path}", get(Self::file_handler_with_index_fallback))
            .with_state(www_storage)
            .merge(var_router)
            .merge(assets_router)
    }

    async fn file_handler_with_index_fallback(
        State(storage): State<Storage>,
        Path(file_path): Path<String>,
    ) -> impl IntoResponse {
        let response = Self::file_handler(State(storage.clone()), Path(file_path))
            .await
            .into_response();

        if response.status() == StatusCode::NOT_FOUND {
            Self::index_handler(State(storage)).await.into_response()
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

    async fn index_handler(storage: State<Storage>) -> Response {
        let mut resp = Self::file_handler(storage, Path(INDEX_PATH.to_owned()))
            .await
            .into_response();

        // NOTE: Add cache-control headers to prevent storing the index file,
        // forcing the browser to always fetch the latest version from the server
        if let Ok(header) = HeaderValue::from_str("no-cache, no-store, must-revalidate") {
            resp.headers_mut().append(header::CACHE_CONTROL, header);
        }

        resp
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
