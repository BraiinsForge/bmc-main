// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_grpc::web::{Metadata, system_service_server::SystemService as GrpcSystemService};
use tonic::Request;

use crate::BmcManager;

pub(crate) struct SystemService<T>
where
    T: BmcManager,
{
    manager: Arc<T>,
}

impl<T> SystemService<T>
where
    T: BmcManager,
{
    pub(crate) fn new(manager: Arc<T>) -> Self {
        Self { manager }
    }
}

#[tonic::async_trait]
impl<T> GrpcSystemService for SystemService<T>
where
    T: BmcManager,
{
    async fn get_metadata(
        &self,
        _request: Request<()>,
    ) -> Result<tonic::Response<Metadata>, tonic::Status> {
        let version = self.manager.version();
        Ok(tonic::Response::new(Metadata { version }))
    }
}
