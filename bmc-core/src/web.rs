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
