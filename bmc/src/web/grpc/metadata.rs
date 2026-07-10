// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_grpc::web::{
    GetMetadataRequest, GetServerInstanceRequest, GetServerInstanceResponse, Metadata,
    metadata_service_server::MetadataService as GrpcMetadataService,
};
use tonic::Request;

use crate::BmcManager;

#[derive(Clone)]
pub(crate) struct MetadataService<T>
where
    T: BmcManager,
{
    bmc_manager: Arc<T>,
    /// Opaque per-process identifier; the service is constructed once per
    /// application start, so the value lives exactly as long as the process.
    server_instance_id: String,
}

impl<T> MetadataService<T>
where
    T: BmcManager,
{
    pub(crate) fn new(bmc_manager: Arc<T>) -> Self {
        Self {
            bmc_manager,
            server_instance_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}

#[async_trait::async_trait]
impl<T> GrpcMetadataService for MetadataService<T>
where
    T: BmcManager,
{
    async fn get_metadata(
        &self,
        _request: Request<GetMetadataRequest>,
    ) -> Result<tonic::Response<Metadata>, tonic::Status> {
        let version = self
            .bmc_manager
            .version()
            .await
            .ok_or_else(|| tonic::Status::internal("Failed to detect current version"))?;

        Ok(tonic::Response::new(Metadata {
            version: version.full,
        }))
    }

    async fn get_server_instance(
        &self,
        _request: Request<GetServerInstanceRequest>,
    ) -> Result<tonic::Response<GetServerInstanceResponse>, tonic::Status> {
        Ok(tonic::Response::new(GetServerInstanceResponse {
            server_instance_id: self.server_instance_id.clone(),
        }))
    }
}
