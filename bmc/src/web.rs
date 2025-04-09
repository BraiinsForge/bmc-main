// Copyright (C) 2025  Braiins Systems s.r.o.

mod grpc;
mod http_server;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{ServiceExt, extract::Request, http::header::CONTENT_TYPE};
use tokio::net::TcpListener;
use tower::{Layer, steer::Steer};
use tower_http::normalize_path::NormalizePathLayer;

use crate::BmcManager;

pub(crate) struct WebService<T: BmcManager> {
    manager: Arc<T>,
    config: ServerConfig,
}

impl<T: BmcManager> WebService<T> {
    pub(crate) fn new(manager: Arc<T>, config: ServerConfig) -> Self {
        Self { manager, config }
    }

    pub(crate) async fn run(self, listener: TcpListener) -> Result<()> {
        let http_router = http_server::HttpServer::new(self.config).build();
        let grpc_router = grpc::GrpcWeb::new(self.manager).build().into_axum_router();

        // combine grpc and http router into one service
        let service = Steer::new(
            vec![http_router, grpc_router],
            |req: &Request, _services: &[_]| {
                if req
                    .headers()
                    .get(CONTENT_TYPE)
                    .map(|content_type| content_type.as_bytes())
                    .filter(|content_type| content_type.starts_with(b"application/grpc"))
                    .is_some()
                {
                    // grpc service
                    1
                } else {
                    // http service
                    0
                }
            },
        );

        let service = NormalizePathLayer::trim_trailing_slash().layer(service);
        let service =
            ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(service);

        axum::serve(listener, service).await?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct ServerConfig {
    pub www_root_path: PathBuf,
    pub www_assets_path: PathBuf,
    pub www_var_path: PathBuf,
}

impl ServerConfig {
    pub const WWW_ROOT_PATH: &'static str = "/www/bmc";

    pub fn set_www_root_path(mut self, www_root_path: PathBuf) -> Self {
        self.www_root_path = www_root_path;
        self
    }

    pub fn set_www_assets_path(mut self, www_assets_path: PathBuf) -> Self {
        self.www_assets_path = www_assets_path;
        self
    }

    pub fn set_www_var_path(mut self, www_var_path: PathBuf) -> Self {
        self.www_var_path = www_var_path;
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            www_root_path: Self::WWW_ROOT_PATH.into(),
            www_assets_path: PathBuf::from(Self::WWW_ROOT_PATH).join("assets"),
            www_var_path: PathBuf::from(Self::WWW_ROOT_PATH).join("var"),
        }
    }
}
