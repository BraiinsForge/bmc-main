// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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
