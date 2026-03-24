// Copyright (C) 2026  Braiins Systems s.r.o.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Router, ServiceExt};
use bmc_grpc::web::initial_setup_service_server::{InitialSetupService, InitialSetupServiceServer};
use bmc_grpc::web::{
    EncryptionType as GrpcEncryptionType, ScanWifiResponse, SetWifiRequest, SettingsDataResponse,
    SettingsRequest, SignalStrength as GrpcSignalStrength, WifiNetwork,
};
use bmc_shared_ii_net::wifi::{EncryptionType, SignalStrength};
use http::StatusCode;
use http::header::LOCATION;
use tokio::fs::File;
use tokio::net::TcpListener;
use tokio_util::io::ReaderStream;
use tonic::{Response as TonicResponse, Status};
use tonic_web::GrpcWebLayer;
use tower::Layer as _;
use tower::steer::Steer;

use crate::init::{InitError, InitPlatform};
use crate::state::InitState;
use crate::utils::AP_IP;

/// RAII guard that resets the `connect_in_progress` flag on drop.
/// Ensures the flag is cleared even if the spawned task panics.
struct ConnectGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ConnectGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn into_grpc_encryption_type(value: EncryptionType) -> GrpcEncryptionType {
    match value {
        EncryptionType::None => GrpcEncryptionType::None,
        EncryptionType::Wep => GrpcEncryptionType::Wep,
        EncryptionType::WepShared => GrpcEncryptionType::WepShared,
        EncryptionType::Wpa => GrpcEncryptionType::Wpa,
        EncryptionType::Wpa1_2 => GrpcEncryptionType::Wpa12,
        EncryptionType::Wpa2 => GrpcEncryptionType::Wpa2,
        EncryptionType::Wpa2_3 => GrpcEncryptionType::Wpa23,
        EncryptionType::Wpa3 => GrpcEncryptionType::Wpa3,
    }
}

fn try_from_grpc_encryption_type(value: GrpcEncryptionType) -> Result<EncryptionType, Box<Status>> {
    match value {
        GrpcEncryptionType::Unspecified => Err(Box::new(Status::invalid_argument(
            "encryption_type must be specified and cannot be UNSPECIFIED",
        ))),
        GrpcEncryptionType::None => Ok(EncryptionType::None),
        GrpcEncryptionType::Wep => Ok(EncryptionType::Wep),
        GrpcEncryptionType::WepShared => Ok(EncryptionType::WepShared),
        GrpcEncryptionType::Wpa => Ok(EncryptionType::Wpa),
        GrpcEncryptionType::Wpa12 => Ok(EncryptionType::Wpa1_2),
        GrpcEncryptionType::Wpa2 => Ok(EncryptionType::Wpa2),
        GrpcEncryptionType::Wpa23 => Ok(EncryptionType::Wpa2_3),
        GrpcEncryptionType::Wpa3 => Ok(EncryptionType::Wpa3),
    }
}

fn into_grpc_signal_strength(value: SignalStrength) -> GrpcSignalStrength {
    match value {
        SignalStrength::Offline => GrpcSignalStrength::Unspecified,
        SignalStrength::Low => GrpcSignalStrength::Weak,
        SignalStrength::Fair => GrpcSignalStrength::Moderate,
        SignalStrength::Excellent => GrpcSignalStrength::Strong,
    }
}

struct InitSetupService<P: InitPlatform> {
    platform: Arc<P>,
    wifi_connected_tx: tokio::sync::mpsc::Sender<Result<(), String>>,
    state_tx: tokio::sync::mpsc::Sender<InitState>,
    connect_in_progress: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl<P: InitPlatform + 'static> InitialSetupService for InitSetupService<P> {
    async fn scan_wifi(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<TonicResponse<ScanWifiResponse>, Status> {
        let networks = self.platform.scan_wifi().await.map_err(|e| {
            tracing::warn!("WiFi scan failed: {e}");
            Status::internal("failed to scan WiFi networks")
        })?;

        let grpc_networks = networks
            .into_iter()
            .filter(|wifi| wifi.signal_strength() != SignalStrength::Offline)
            .map(|wifi| WifiNetwork {
                signal_strength: into_grpc_signal_strength(wifi.signal_strength()) as i32,
                ssid: wifi.ssid,
                encryption_type: into_grpc_encryption_type(wifi.encryption_type) as i32,
            })
            .collect();

        Ok(TonicResponse::new(ScanWifiResponse {
            networks: grpc_networks,
        }))
    }

    async fn set_wifi(
        &self,
        request: tonic::Request<SetWifiRequest>,
    ) -> Result<TonicResponse<()>, Status> {
        let req = request.into_inner();
        let encryption = try_from_grpc_encryption_type(req.encryption_type()).map_err(|e| *e)?;
        let ssid = req.ssid;
        let password = req.password;

        self.connect_in_progress
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            )
            .map_err(|_| {
                Status::failed_precondition("WiFi connection attempt already in progress")
            })?;

        let platform = self.platform.clone();
        let wifi_connected_tx = self.wifi_connected_tx.clone();
        let state_tx = self.state_tx.clone();
        let in_progress = self.connect_in_progress.clone();

        tokio::spawn(async move {
            let _guard = ConnectGuard(in_progress);

            // Notify UI immediately so the display shows "Connecting..."
            let _ = state_tx
                .send(InitState::Connecting { ssid: ssid.clone() })
                .await;

            // Give the gRPC response time to reach the client before
            // tearing down the AP network.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            // Trust save_and_connect's success semantics — the WiFi driver
            // already waits for an IPv4 address on the STA interface before
            // returning Ok. No additional box-wide IP polling needed.
            let result = platform
                .save_and_connect(ssid.clone(), password, encryption)
                .await
                .map_err(|e| format!("WiFi connect to '{ssid}' failed: {e}"));

            let is_success = result.is_ok();
            if wifi_connected_tx.send(result).await.is_err() {
                // Receiver dropped — init flow is gone. Only revert to AP if
                // the connection failed. If it succeeded, leave STA mode —
                // the init flow will detect connectivity on retry.
                if is_success {
                    tracing::info!("WiFi connected but result channel closed, keeping STA mode");
                } else {
                    tracing::warn!(
                        "WiFi result channel closed and connection failed, \
                         performing last-resort AP revert"
                    );
                    if let Err(ap_err) = platform.configure_wifi_ap().await {
                        tracing::error!("last-resort AP revert failed: {ap_err}");
                    }
                    if let Err(cp_err) = platform.enable_captive_portal().await {
                        tracing::error!("last-resort captive portal enable failed: {cp_err}");
                    }
                }
            }

            // _guard drops here, resetting connect_in_progress
        });

        Ok(TonicResponse::new(()))
    }

    async fn get_settings_data(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<TonicResponse<SettingsDataResponse>, Status> {
        Err(Status::unimplemented(
            "get_settings_data is not available during init",
        ))
    }

    async fn setup_device(
        &self,
        _request: tonic::Request<SettingsRequest>,
    ) -> Result<TonicResponse<()>, Status> {
        Err(Status::unimplemented(
            "setup_device is not available during init",
        ))
    }
}

const CAPTIVE_SUFFIXES: &[&str] = &[".com", ".net", ".info", ".us", ".network"];

struct CaptivePortalState<P: InitPlatform> {
    platform: Arc<P>,
}

impl<P: InitPlatform> Clone for CaptivePortalState<P> {
    fn clone(&self) -> Self {
        Self {
            platform: self.platform.clone(),
        }
    }
}

async fn captive_portal_middleware<P: InitPlatform + 'static>(
    State(state): State<CaptivePortalState<P>>,
    request: Request,
    next: axum::middleware::Next,
) -> Response {
    let host = request
        .headers()
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);

    let should_redirect = host
        .as_deref()
        .is_some_and(|h| CAPTIVE_SUFFIXES.iter().any(|suffix| h.ends_with(suffix)));

    if should_redirect {
        // Build absolute redirect URL — OS captive portal detectors
        // (iOS/Android) require an absolute URL to complete interception.
        // Always use IP address, never Host header (which would cause
        // redirect loops via dnsmasq DNS redirect).
        let redirect_ip = state.platform.ip_address().await.unwrap_or(AP_IP);
        let target = format!("http://{redirect_ip}/init_connect");
        return (
            StatusCode::FOUND,
            [(
                LOCATION,
                HeaderValue::from_str(&target)
                    .unwrap_or_else(|_| HeaderValue::from_static("/init_connect")),
            )],
        )
            .into_response();
    }

    next.run(request).await
}

const INIT_CONNECT_INDEX: &str = "index-connect.html";

#[derive(Clone)]
struct WwwState {
    www_path: PathBuf,
}

async fn serve_file(www_path: &std::path::Path, file_path: &str) -> Response {
    let full_path = www_path.join(file_path);
    let Ok(file) = File::open(&full_path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mime_type = mime_guess::from_path(file_path).first_or_text_plain();
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut response = body.into_response();
    if let Ok(header) = HeaderValue::from_str(mime_type.as_ref()) {
        response.headers_mut().insert(CONTENT_TYPE, header);
    }
    response
}

async fn handle_init_connect(State(state): State<WwwState>) -> Response {
    serve_file(&state.www_path, INIT_CONNECT_INDEX).await
}

async fn handle_root() -> Response {
    (
        StatusCode::FOUND,
        [(LOCATION, HeaderValue::from_static("/init_connect"))],
    )
        .into_response()
}

async fn handle_wildcard(State(state): State<WwwState>, Path(path): Path<String>) -> Response {
    let response = serve_file(&state.www_path, &path).await;
    if response.status() == StatusCode::NOT_FOUND {
        return serve_file(&state.www_path, INIT_CONNECT_INDEX).await;
    }
    response
}

/// Run the WiFi setup HTTP/gRPC server during AP mode.
///
/// Serves static frontend files and handles `ScanWifi`/`SetWifi` RPCs
/// for the captive portal. Runs until `shutdown` signal is received.
pub async fn run_wifi_setup_server<P: InitPlatform + 'static>(
    platform: Arc<P>,
    www_path: PathBuf,
    listen_addr: SocketAddr,
    shutdown: tokio::sync::oneshot::Receiver<()>,
    wifi_connected_tx: tokio::sync::mpsc::Sender<Result<(), String>>,
    state_tx: tokio::sync::mpsc::Sender<InitState>,
) -> Result<(), InitError> {
    let grpc_service = InitialSetupServiceServer::new(InitSetupService {
        platform: platform.clone(),
        wifi_connected_tx,
        state_tx,
        connect_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });

    let grpc_router =
        tonic::service::Routes::new(GrpcWebLayer::new().layer(grpc_service)).into_axum_router();

    let www_state = WwwState {
        www_path: www_path.clone(),
    };
    let captive_state = CaptivePortalState {
        platform: platform.clone(),
    };

    let http_router = Router::new()
        .route("/init_connect", get(handle_init_connect))
        .route("/", get(handle_root))
        .route("/{*path}", get(handle_wildcard))
        .with_state(www_state)
        .layer(axum::middleware::from_fn_with_state(
            captive_state,
            captive_portal_middleware::<P>,
        ));

    let service = Steer::new(
        vec![http_router, grpc_router],
        |req: &Request, _services: &[_]| {
            usize::from(
                req.headers()
                    .get(CONTENT_TYPE)
                    .map(HeaderValue::as_bytes)
                    .is_some_and(|ct| ct.starts_with(b"application/grpc")),
            )
        },
    );

    let make_service =
        ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(service);

    let listener = TcpListener::bind(listen_addr)
        .await
        .map_err(|e| InitError::config(format!("failed to bind {listen_addr}: {e}")))?;

    tracing::info!("WiFi setup server listening on {listen_addr}");

    axum::serve(listener, make_service)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
            tracing::info!("WiFi setup server shutting down");
        })
        .await
        .map_err(|e| InitError::config(format!("server error: {e}")))?;

    Ok(())
}
