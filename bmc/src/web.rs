// Copyright (C) 2025  Braiins Systems s.r.o.

mod grpc;
mod http_server;
mod no_password;
mod session;

use crate::BmcManager;
use crate::session::Manager as SessionManager;
use anyhow::Result;
use axum::{ServiceExt, extract::Request, http::header::CONTENT_TYPE};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
use tokio::net::TcpListener;
use tower::{Layer, steer::Steer};
use tower_http::normalize_path::NormalizePathLayer;

pub(crate) struct WebService<T: BmcManager, S: SessionManager> {
    manager: Arc<T>,
    session_manager: Arc<S>,
    config: ServerConfig,
}

impl<T: BmcManager, S: SessionManager> WebService<T, S> {
    pub(crate) fn new(manager: Arc<T>, session_manager: Arc<S>, config: ServerConfig) -> Self {
        Self {
            manager,
            session_manager,
            config,
        }
    }

    pub(crate) async fn run(self, listener: TcpListener) -> Result<()> {
        let http_router = http_server::HttpServer::new(self.config).build();
        let grpc_router = grpc::GrpcWeb::new(self.manager, self.session_manager.clone())
            .build()
            .into_axum_router()
            .layer(session::SessionLayer::new(self.session_manager.clone()))
            .layer(no_password::NoPasswordLayer::new(self.session_manager));
        // combine grpc and http router into one service
        let service = Steer::new(
            vec![http_router, grpc_router],
            |req: &Request, _services: &[_]| {
                // grpc service -> 1
                // http service -> 0
                usize::from(
                    req.headers()
                        .get(CONTENT_TYPE)
                        .map(axum::http::HeaderValue::as_bytes)
                        .filter(|content_type| content_type.starts_with(b"application/grpc"))
                        .is_some(),
                )
            },
        );

        let service = NormalizePathLayer::trim_trailing_slash().layer(service);
        let service =
            ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(service);

        axum::serve(listener, service).await?;

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub www_root_path: PathBuf,
    pub www_assets_path: PathBuf,
    pub www_var_path: PathBuf,
    pub grpc_address: std::net::SocketAddr,
}

impl ServerConfig {
    pub const WWW_ROOT_PATH: &'static str = "/www";

    #[must_use]
    pub fn set_www_root_path(mut self, www_root_path: PathBuf) -> Self {
        self.www_root_path = www_root_path;
        self
    }

    #[must_use]
    pub fn set_www_assets_path(mut self, www_assets_path: PathBuf) -> Self {
        self.www_assets_path = www_assets_path;
        self
    }

    #[must_use]
    pub fn set_www_var_path(mut self, www_var_path: PathBuf) -> Self {
        self.www_var_path = www_var_path;
        self
    }

    #[must_use]
    pub fn set_grpc_address(mut self, address: std::net::SocketAddr) -> Self {
        self.grpc_address = address;
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            www_root_path: Self::WWW_ROOT_PATH.into(),
            www_assets_path: PathBuf::from(Self::WWW_ROOT_PATH).join("assets"),
            www_var_path: PathBuf::from(Self::WWW_ROOT_PATH).join("var"),
            grpc_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 50051),
        }
    }
}
