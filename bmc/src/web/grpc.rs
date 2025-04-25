// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::web::SessionManager;
use crate::web::session::{extract_cookies, extract_token};
use axum_extra::extract::cookie::Cookie;
use bmc_grpc::web;
use http::header;
use std::sync::Arc;
use tonic::service::Routes;
use tonic::{Status, body::Body, codegen::http::Request};
use tonic_middleware::InterceptorFor;
use tonic_middleware::RequestInterceptor;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower::Layer;
use tracing::debug;

pub mod authentication;
mod system;

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
            // make sure, there is no authentication header anymore
            req.headers_mut().remove(header::AUTHORIZATION.as_str());
        }
        Ok(req)
    }
}

#[derive(Clone)]
pub(crate) struct GrpcWeb<T: BmcManager, S: SessionManager> {
    manager: Arc<T>,
    session_manager: Arc<S>,
}

impl<T: BmcManager, S: SessionManager> GrpcWeb<T, S> {
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
