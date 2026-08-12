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
use uuid::Uuid;

use crate::{BmcManager, manager::BmcState, widget::WidgetRegistry};

use super::{ServerConfig, captive_portal::CaptivePortalLayer};

const ZERO: &str = "0";
const SUPPORT_ARCHIVE_FILENAME_PREFIX: &str = "support_archive_";
const SUPPORT_ARCHIVE_FILENAME_SUFFIX: &str = ".zip.enc";
const SUPPORT_ARCHIVE_FORMAT: SupportArchiveFormat = SupportArchiveFormat::ZipEncrypted;

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

impl<T: BmcManager> HttpServer<T> {
    const INDEX_PATH: &str = "index.html";
    const INITIAL_SETUP_INDEX_FILENAME: &str = "index-connect.html";
    pub(crate) const WIFI_SETUP_URL_ENDPOINT: &str = "/init_connect";
    pub(crate) const DEVICE_SETUP_URL_ENDPOINT: &str = "/init_setup";
    pub(crate) const ROOT_URL_ENDPOINT: &str = "/";
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
        Router::new()
            .merge(self.static_file_router())
            .merge(self.general_api_router())
            .merge(self.widget_icon_router())
            .layer(CompressionLayer::new())
            .layer(CaptivePortalLayer::new(self.manager.clone()))
            .layer(middleware::from_fn(Self::log_request))
    }

    fn static_file_router(&self) -> Router {
        let www_storage = Storage::new(self.config.www_root_path.clone());
        let var_storage = Storage::new(self.config.www_var_path.clone());
        let assets_storage = Storage::new(self.config.www_assets_path.clone());

        let var_router = Router::new()
            .route("/var/{*file_path}", get(Storage::file_handler))
            .with_state(var_storage);

        let assets_router = Router::new()
            .route("/assets/{*file_path}", get(Storage::file_handler))
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
        State(IndexState { storage, manager }): State<IndexState<T>>,
        Path(file_path): Path<String>,
    ) -> impl IntoResponse {
        let response = Storage::file_handler(State(storage.clone()), Path(file_path))
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
        State(IndexState { storage, manager }): State<IndexState<T>>,
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
        State(IndexState { storage, manager }): State<IndexState<T>>,
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
                [(http::header::LOCATION.as_str(), Self::ROOT_URL_ENDPOINT)],
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

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;
    use crate::widget::{WidgetInfo, WidgetRegistry};
    use tower::ServiceExt as _;

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
