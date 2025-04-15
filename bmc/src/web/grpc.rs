// Copyright (C) 2025  Braiins Systems s.r.o.

use super::auth::AuthInterceptor;
use crate::BmcManager;
use crate::web::SessionManager;
use bmc_grpc::web;
use std::sync::Arc;
use tonic::service::Routes;
use tonic_middleware::InterceptorFor;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower::Layer;

pub mod authentication;
mod system;

#[derive(Clone)]
pub(crate) struct GrpcWeb<T: BmcManager, S: SessionManager> {
    manager: Arc<T>,
    session_manager: Arc<S>,
}

impl<T: BmcManager + Clone, S: SessionManager + Clone> GrpcWeb<T, S> {
    pub(crate) fn new(manager: Arc<T>, session_manager: Arc<S>) -> Self {
        Self {
            manager,
            session_manager,
        }
    }

    pub(crate) fn build(self) -> Routes {
        let auth_interceptor = AuthInterceptor {
            session_manager: self.session_manager.clone(),
        };

        let reflection_service = Builder::configure()
            .register_encoded_file_descriptor_set(web::FILE_DESCRIPTOR_SET)
            .build_v1alpha()
            .expect("BUG: Unable to decode reflection descriptor");

        let authentication_service =
            web::authentication_service_server::AuthenticationServiceServer::new(
                authentication::AuthenticationService::new(self.session_manager.clone()),
            );

        let system_service = web::system_service_server::SystemServiceServer::new(
            system::SystemService::new(self.manager.clone(), self.session_manager.clone()),
        );

        // GrpcWebLayer is badly named, it's not a "layer", it's re-wrapper for other Services
        // All services requiring authentication have to be wrapped in GrpcWebLayer and use "InterceptorFor"
        Routes::new(GrpcWebLayer::new().layer(reflection_service))
            .add_service(GrpcWebLayer::new().layer(authentication_service))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                system_service,
                auth_interceptor.clone(),
            )))
    }
}
