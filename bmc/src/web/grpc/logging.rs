// Copyright (C) 2025  Braiins Systems s.r.o.

use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Instant;
use tonic::server::NamedService;
use tower::{Layer, Service};
use tracing::info;

const GRPC_STATUS_HEADER: &str = "grpc-status";

#[derive(Clone)]
pub struct GrpcLoggingLayer;

impl GrpcLoggingLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for GrpcLoggingLayer {
    type Service = GrpcLoggingService<S>;

    fn layer(&self, service: S) -> Self::Service {
        GrpcLoggingService { inner: service }
    }
}

#[derive(Clone)]
pub struct GrpcLoggingService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for GrpcLoggingService<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: std::fmt::Display,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let method = req.uri().path().to_owned();

            let client_ip = req
                .extensions()
                .get::<axum::extract::ConnectInfo<SocketAddr>>()
                .map_or_else(
                    || String::from("-"),
                    |axum::extract::ConnectInfo(addr)| addr.to_string(),
                );

            let start = Instant::now();

            let response = inner.call(req).await;

            let latency = start.elapsed().as_secs_f64();

            match &response {
                Ok(res) => {
                    let http_status = res.status().as_u16();

                    let grpc_status = res
                        .headers()
                        .get(GRPC_STATUS_HEADER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<i32>().ok())
                        .unwrap_or(0); // NOTE: 0 = OK if not present

                    let status_name = grpc_status_to_name(grpc_status);

                    if grpc_status == 0 {
                        info!(
                            client_ip = %client_ip,
                            method = %method,
                            status = %status_name,
                            http_status = http_status,
                            latency = format!("{:.6}", latency),
                            "gRPC request"
                        );
                    } else {
                        info!(
                            client_ip = %client_ip,
                            method = %method,
                            status = %status_name,
                            grpc_code = grpc_status,
                            http_status = http_status,
                            latency = format!("{:.6}", latency),
                            "gRPC request error"
                        );
                    }
                }
                Err(err) => {
                    // NOTE: Transport-level error (connection failed, etc.)
                    info!(
                        client_ip = %client_ip,
                        method = %method,
                        error = %err,
                        latency = format!("{:.6}", latency),
                        "gRPC transport error"
                    );
                }
            }

            response
        })
    }
}

// Implement NamedService to pass through the service name
impl<S: NamedService> NamedService for GrpcLoggingService<S> {
    const NAME: &'static str = S::NAME;
}

/// Convert gRPC status code to human-readable name
fn grpc_status_to_name(code: i32) -> &'static str {
    match code {
        0 => "OK",
        1 => "CANCELLED",
        2 => "UNKNOWN",
        3 => "INVALID_ARGUMENT",
        4 => "DEADLINE_EXCEEDED",
        5 => "NOT_FOUND",
        6 => "ALREADY_EXISTS",
        7 => "PERMISSION_DENIED",
        8 => "RESOURCE_EXHAUSTED",
        9 => "FAILED_PRECONDITION",
        10 => "ABORTED",
        11 => "OUT_OF_RANGE",
        12 => "UNIMPLEMENTED",
        13 => "INTERNAL",
        14 => "UNAVAILABLE",
        15 => "DATA_LOSS",
        16 => "UNAUTHENTICATED",
        _ => "UNKNOWN_CODE",
    }
}
