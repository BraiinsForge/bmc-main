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

use bmc_wasm_sdk::ufmt;

use crate::device::DeviceIdentity;
use crate::discovery::JsonLookup;
use crate::model::{MinerModel, ModelAccumulator};
use crate::telemetry::TelemetryReading;

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredDevice {
    pub identity: DeviceIdentity,
    pub model_hint: Option<MinerModel>,
}

/// A device family's discovery behavior: which mDNS service types to browse,
/// and how to turn one of its `Found` payloads into an identity. The family
/// is fixed by the adapter, never read from the event.
pub trait FamilyAdapter {
    fn browse_service_types(&self) -> &'static [&'static str];
    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice>;

    #[cfg_attr(
        all(not(target_arch = "wasm32"), test),
        expect(dead_code, reason = "used by the driver on wasm")
    )]
    fn api_base_path(&self) -> &'static str;
    #[cfg_attr(
        all(not(target_arch = "wasm32"), test),
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

    /// Fill model fields from one telemetry response. Default no-op so families
    /// that do not report a model are unaffected.
    fn parse_model(&self, _endpoint: &str, _json: &dyn JsonLookup, _model: &mut ModelAccumulator) {}

    /// A proactive credential header attached to every request, preferred over
    /// any cached token. The driver passes the operator-configured uBOS
    /// credentials; families that ignore them return none. Default none.
    fn credential_header(&self, _username: &str, _password: &str) -> Option<String> {
        None
    }

    // Authentication — default NONE. Auth families (BOS) override these;
    // no-auth families (Bitaxe) inherit the defaults and fetch unauthenticated.
    fn auth_endpoint(&self) -> Option<&'static str> {
        None
    }
    #[cfg_attr(
        all(not(target_arch = "wasm32"), test),
        expect(dead_code, reason = "used by the driver on wasm")
    )]
    fn login_body(&self, _password: &str) -> String {
        String::new()
    }
    fn parse_login(&self, _json: &dyn JsonLookup) -> Option<String> {
        None
    }
    #[cfg_attr(
        all(not(target_arch = "wasm32"), test),
        expect(dead_code, reason = "used by the driver on wasm")
    )]
    fn auth_header(&self, token: &str) -> String {
        bmc_wasm_sdk::fmt!("Authorization: {token}")
    }
    fn is_auth_error(&self, _status: u32) -> bool {
        false
    }
}
