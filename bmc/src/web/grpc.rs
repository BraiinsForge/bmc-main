// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::web::SessionManager;
use crate::web::session::{extract_cookies, extract_token};
use axum_extra::extract::cookie::Cookie;
use bmc_grpc::web;
use bmc_upgrade::firmware::FirmwareIndex;
use http::header;
use std::fmt::Display;
use std::sync::Arc;
use strum::EnumMessage;
use tonic::service::Routes;
use tonic::{Status, body::Body, codegen::http::Request};
use tonic_middleware::InterceptorFor;
use tonic_middleware::RequestInterceptor;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower::Layer;
use tracing::debug;

pub mod authentication;
mod metadata;
mod system;
use super::SystemUpgradeService;
mod upgrade_service;

struct AuthInterceptor<S: SessionManager> {
    pub session_manager: std::sync::Arc<S>,
}

impl<S: SessionManager> Clone for AuthInterceptor<S> {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
        }
    }
}

#[async_trait::async_trait]
impl<S: SessionManager> RequestInterceptor for AuthInterceptor<S> {
    async fn intercept(&self, mut req: Request<Body>) -> Result<Request<Body>, Status> {
        debug!("Intercepting request: {:?}", req);
        let token = extract_token(&req);
        let session_manager = self.session_manager.clone();
        let mut authenticated = false;

        let cookies = if let Some(token) = token.as_ref() {
            // NOTE: this is not an elegant integration of gRPC and existing session manager.
            // Session manager provides cookie interface, not a token interface. More of that
            // the name of cookie is defined by specific boser implementation, not a library.
            // this part has to be changed in future
            vec![Cookie::new("session_id", token)]
        } else {
            extract_cookies(req.headers()).collect::<Vec<Cookie<'_>>>()
        };

        // find the session by its ID from token/cookies
        if let Ok(session) = session_manager.find(&cookies).await {
            // extend the session
            let cookie = session_manager.extend(session.clone()).await;
            if cookie.is_ok() {
                req.extensions_mut().insert(session);
                authenticated = true;
            }
        }

        if !authenticated {
            return Err(tonic::Status::unauthenticated("Failed to get session"));
        }

        // make sure, there is no authentication header anymore
        req.headers_mut().remove(header::AUTHORIZATION.as_str());

        Ok(req)
    }
}

#[derive(Clone)]
pub(crate) struct GrpcWeb<T: BmcManager, S: SessionManager, U: FirmwareIndex> {
    manager: Arc<T>,
    session_manager: Arc<S>,
    system_upgrade_service: SystemUpgradeService<U, T>,
}

impl<T: BmcManager, S: SessionManager, U: FirmwareIndex> GrpcWeb<T, S, U> {
    pub(crate) fn new(
        manager: Arc<T>,
        session_manager: Arc<S>,
        system_upgrade_service: SystemUpgradeService<U, T>,
    ) -> Self {
        Self {
            manager,
            session_manager,
            system_upgrade_service,
        }
    }

    pub(crate) fn build(self) -> Routes {
        let auth_interceptor = AuthInterceptor {
            session_manager: self.session_manager.clone(),
        };

        let upgrade_service = web::upgrade_service_server::UpgradeServiceServer::new(
            upgrade_service::UpgradeService::new(self.system_upgrade_service),
        );

        let reflection_service = Builder::configure()
            .register_encoded_file_descriptor_set(web::FILE_DESCRIPTOR_SET)
            .build_v1alpha()
            .expect("BUG: Unable to decode reflection descriptor");

        let authentication_service =
            web::authentication_service_server::AuthenticationServiceServer::new(
                authentication::AuthenticationService::new(self.session_manager.clone()),
            );

        let metadata_service = web::metadata_service_server::MetadataServiceServer::new(
            metadata::MetadataService::new(self.manager.clone()),
        );

        let system_service = web::system_service_server::SystemServiceServer::new(
            system::SystemService::new(self.manager, self.session_manager),
        );

        // GrpcWebLayer is badly named, it's not a "layer", it's re-wrapper for other Services
        // All services requiring authentication have to be wrapped in GrpcWebLayer and use "InterceptorFor"
        Routes::new(GrpcWebLayer::new().layer(reflection_service))
            .add_service(GrpcWebLayer::new().layer(authentication_service))
            .add_service(GrpcWebLayer::new().layer(metadata_service))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                system_service,
                auth_interceptor.clone(),
            )))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                upgrade_service,
                auth_interceptor.clone(),
            )))
    }
}

#[derive(EnumMessage)]
pub(crate) enum GrpcError {
    #[strum(serialize = "bad request")]
    BadRequest,
}

impl Display for GrpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Write first serialization, if available (should be always true).
        // In an unlikely case of no serialization, report the platform as unknown.
        write!(
            f,
            "{}",
            self.get_serializations().first().unwrap_or(&"unknown")
        )
    }
}
