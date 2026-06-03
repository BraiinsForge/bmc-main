// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::adapter::{DiscoveredDevice, FamilyAdapter};
use crate::device::{DeviceFamily, DeviceId, DeviceIdentity};
use crate::discovery::{JsonLookup, extract_endpoint};

/// BOS advertises `_http._tcp` with the `_bos` subtype. Browsing the subtype
/// directly means every event on this browse is a BOS device.
pub const BOS_SERVICE_TYPES: &[&str] = &["_bos._sub._http._tcp"];

pub struct BosAdapter;

impl FamilyAdapter for BosAdapter {
    fn browse_service_types(&self) -> &'static [&'static str] {
        BOS_SERVICE_TYPES
    }

    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::new(name.clone()),
                family: DeviceFamily::Bos,
                name,
                host,
                port,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::tests_support::MapJson;

    fn bos_shaped() -> MapJson {
        let mut json = MapJson::default();
        // A subtype browse delivers the BASE service type, not `_bos`.
        json.strings.insert("/service_type", "_http._tcp.local.");
        json.strings.insert("/name", "miner-a._http._tcp.local.");
        json.strings.insert("/host", "10.0.0.5");
        json.ints.insert("/port", 80);
        json
    }

    #[test]
    fn browses_the_bos_subtype() {
        assert_eq!(BosAdapter.browse_service_types(), &["_bos._sub._http._tcp"]);
    }

    #[test]
    fn parses_a_bos_device_from_a_found_event() {
        let found = BosAdapter
            .parse_found(&bos_shaped())
            .expect("device parsed");
        assert_eq!(found.identity.id.as_str(), "miner-a._http._tcp.local.");
        assert_eq!(found.identity.host, "10.0.0.5");
        assert_eq!(found.identity.port, 80);
    }

    #[test]
    fn stamps_family_from_adapter_not_service_type() {
        let found = BosAdapter
            .parse_found(&bos_shaped())
            .expect("device parsed");
        assert_eq!(found.identity.family, DeviceFamily::Bos);
    }

    #[test]
    fn rejects_event_missing_host() {
        let mut json = bos_shaped();
        json.strings.remove("/host");
        assert_eq!(BosAdapter.parse_found(&json), None);
    }
}
