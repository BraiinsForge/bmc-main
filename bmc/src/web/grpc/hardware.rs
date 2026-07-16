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

use bmc_grpc::web;
use bmc_grpc::web::hardware_service_server::HardwareService as GrpcHardwareService;
use bmc_platform::HardwareCapabilities;
use tonic::{Request, Response, Status};

#[must_use]
pub(crate) fn caps_to_proto(caps: &HardwareCapabilities) -> web::HardwareCapabilities {
    web::HardwareCapabilities {
        combined_scenes_supported: caps.slot_grid.is_some(),
    }
}

pub(crate) struct HardwareCapabilitiesService {
    capabilities: HardwareCapabilities,
}

impl HardwareCapabilitiesService {
    pub(crate) fn new(capabilities: HardwareCapabilities) -> Self {
        Self { capabilities }
    }
}

#[async_trait::async_trait]
impl GrpcHardwareService for HardwareCapabilitiesService {
    async fn get_hardware_capabilities(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::HardwareCapabilities>, Status> {
        Ok(Response::new(caps_to_proto(&self.capabilities)))
    }
}

#[cfg(test)]
mod tests {
    use super::caps_to_proto;
    use bmc_platform::{DisplayInfo, DisplayShape, HardwareCapabilities, SlotGrid};

    #[test]
    fn slot_grid_present_supports_combined_scenes() {
        let proto = caps_to_proto(&HardwareCapabilities {
            display: DisplayInfo {
                width: 1_280,
                height: 480,
                shape: DisplayShape::Rectangular,
                dpi: 1,
            },
            slot_grid: Some(SlotGrid {
                columns: 4,
                rows: 2,
            }),
        });
        assert!(proto.combined_scenes_supported);
    }

    #[test]
    fn absent_slot_grid_disables_combined_scenes() {
        let proto = caps_to_proto(&HardwareCapabilities {
            display: DisplayInfo {
                width: 320,
                height: 240,
                shape: DisplayShape::Rectangular,
                dpi: 1,
            },
            slot_grid: None,
        });
        assert!(!proto.combined_scenes_supported);
    }
}
