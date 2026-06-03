// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::ufmt;

use crate::device::DeviceIdentity;
use crate::discovery::JsonLookup;
use crate::telemetry::TelemetryReading;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub identity: DeviceIdentity,
}

/// A device family's discovery behavior: which mDNS service types to browse,
/// and how to turn one of its `Found` payloads into an identity. The family
/// is fixed by the adapter, never read from the event.
pub trait FamilyAdapter {
    fn browse_service_types(&self) -> &'static [&'static str];
    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice>;

    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(dead_code, reason = "used by the driver on wasm")
    )]
    fn api_base_path(&self) -> &'static str;
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(dead_code, reason = "used by the driver on wasm")
    )]
    fn telemetry_endpoints(&self) -> &'static [&'static str];
    fn parse_telemetry(
        &self,
        endpoint: &str,
        json: &dyn JsonLookup,
        reading: &mut TelemetryReading,
    );
    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading);

    // Authentication — default NONE. Auth families (BOS) override these;
    // no-auth families (Bitaxe) inherit the defaults and fetch unauthenticated.
    fn auth_endpoint(&self) -> Option<&'static str> {
        None
    }
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(dead_code, reason = "used by the driver on wasm")
    )]
    fn login_body(&self, _password: &str) -> String {
        String::new()
    }
    fn parse_login(&self, _json: &dyn JsonLookup) -> Option<String> {
        None
    }
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(dead_code, reason = "used by the driver on wasm")
    )]
    fn auth_header(&self, token: &str) -> String {
        bmc_wasm_sdk::fmt!("Authorization: {token}")
    }
    fn is_auth_error(&self, _status: u32) -> bool {
        false
    }
}
