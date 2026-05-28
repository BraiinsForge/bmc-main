// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_grpc::web;
use tonic::{Request, Response, Status};

/// Temporary stage-2 hardware service returning BMC100-compatible
/// capabilities so generated clients integrate before stage 3 wires the
/// real compositor-backed values.
pub(crate) struct HardwareService;

impl HardwareService {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl web::hardware_service_server::HardwareService for HardwareService {
    async fn get_hardware_capabilities(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::HardwareCapabilities>, Status> {
        Ok(Response::new(bmc100_capabilities()))
    }
}

fn bmc100_capabilities() -> web::HardwareCapabilities {
    web::HardwareCapabilities {
        combined_scenes_supported: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmc100_supports_combined_scenes() {
        assert!(bmc100_capabilities().combined_scenes_supported);
    }
}
