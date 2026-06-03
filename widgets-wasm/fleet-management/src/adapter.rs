// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::device::{DeviceFamily, DeviceIdentity};
use crate::discovery::JsonLookup;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub identity: DeviceIdentity,
}

/// A device family's discovery behavior: which mDNS service types to browse,
/// and how to turn one of its `Found` payloads into an identity. The family
/// is fixed by the adapter, never read from the event.
#[expect(dead_code, reason = "first implemented by the BOS adapter in the next task")]
pub trait FamilyAdapter {
    fn family(&self) -> DeviceFamily;
    fn browse_service_types(&self) -> &'static [&'static str];
    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice>;
}
