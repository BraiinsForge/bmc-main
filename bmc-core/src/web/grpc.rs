// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_grpc::web;
use tonic::service::Routes;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower::Layer;

use crate::BmcManager;

mod system_service;

pub(crate) struct GrpcWeb<T: BmcManager> {
    manager: Arc<T>,
}

impl<T: BmcManager> GrpcWeb<T> {
    pub(crate) fn new(manager: Arc<T>) -> Self {
        Self { manager }
    }

    pub(crate) fn build(self) -> Routes {
        let system_service = web::system_service_server::SystemServiceServer::new(
            system_service::SystemService::new(self.manager),
        );

        let reflection_service = Builder::configure()
            .register_encoded_file_descriptor_set(web::FILE_DESCRIPTOR_SET)
            .build_v1alpha()
            .expect("BUG: Unable to decode reflection descriptor");

        Routes::new(GrpcWebLayer::new().layer(system_service))
            .add_service(GrpcWebLayer::new().layer(reflection_service))
    }
}
