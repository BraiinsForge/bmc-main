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

use crate::adapter::FamilyAdapter;
use crate::device::{DeviceFamily, DeviceId};
use crate::families::bitaxe::BitaxeAdapter;
use crate::families::bos::BosAdapter;
use crate::families::ubos::UbosAdapter;

/// Map a device family to its adapter.
#[expect(
    clippy::unnecessary_wraps,
    reason = "keep the existing driver contract while unsupported-family handling remains in place"
)]
#[must_use]
pub fn adapter_for(family: DeviceFamily) -> Option<&'static dyn FamilyAdapter> {
    match family {
        DeviceFamily::Bos => Some(&BosAdapter),
        DeviceFamily::Ubos => Some(&UbosAdapter),
        DeviceFamily::Bitaxe => Some(&BitaxeAdapter),
    }
}

/// A cursor over a snapshot of device ids — the poller's rotation ring. The
/// snapshot is taken at each rebuild, so devices added or removed mid-rotation
/// are only seen on the next one.
pub struct PassCursor {
    ids: Vec<DeviceId>,
    index: usize,
}

impl PassCursor {
    #[must_use]
    pub fn new(ids: Vec<DeviceId>) -> Self {
        Self { ids, index: 0 }
    }

    #[must_use]
    pub fn current(&self) -> Option<&DeviceId> {
        self.ids.get(self.index)
    }

    pub fn advance(&mut self) {
        self.index += 1;
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.index >= self.ids.len()
    }
}

/// The result of fetching one of a device's telemetry endpoints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndpointOutcome {
    Ok,
    Failed,
    AuthFailed,
}

/// What the barrier does once every endpoint of a device has reported.
#[derive(PartialEq, Eq, Debug)]
pub enum ReauthDecision {
    Finalize,
    Reauth { endpoints: Vec<usize> },
}

/// Decide the post-burst action: re-authenticate and re-fire the auth-failed
/// endpoints once per device per pass, otherwise finalize. The `reauthed` guard
/// prevents a 401 -> login -> 401 loop.
#[must_use]
pub fn reauth_decision(outcomes: &[EndpointOutcome], reauthed: bool) -> ReauthDecision {
    if reauthed {
        return ReauthDecision::Finalize;
    }
    let endpoints: Vec<usize> = outcomes
        .iter()
        .enumerate()
        .filter(|(_, o)| **o == EndpointOutcome::AuthFailed)
        .map(|(i, _)| i)
        .collect();
    if endpoints.is_empty() {
        ReauthDecision::Finalize
    } else {
        ReauthDecision::Reauth { endpoints }
    }
}

/// A device is reachable when at least one endpoint returned usable telemetry.
#[must_use]
pub fn pass_reachable(outcomes: &[EndpointOutcome]) -> bool {
    outcomes.contains(&EndpointOutcome::Ok)
}

/// Fail every still-pending (`None`) endpoint, keeping any already-reported one —
/// so a login that can't be sent doesn't discard the `Ok` readings this pass.
pub fn fail_pending(outcomes: &mut [Option<EndpointOutcome>]) {
    for outcome in outcomes {
        outcome.get_or_insert(EndpointOutcome::Failed);
    }
}

#[cfg(target_arch = "wasm32")]
mod driver;

#[cfg(target_arch = "wasm32")]
pub use driver::{
    clear_tokens_for, ensure_running, family_enabled, on_discovered, probe_bos_candidate,
    refresh_params, remove_token, stop,
};

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::ufmt;

    use super::*;

    fn ids(n: usize) -> Vec<DeviceId> {
        (0..n)
            .map(|i| DeviceId::new(bmc_wasm_sdk::fmt!("d{i}._http._tcp.local.")))
            .collect()
    }

    #[test]
    fn cursor_advances_then_completes() {
        let mut c = PassCursor::new(ids(2));
        assert_eq!(
            c.current().map(DeviceId::as_str),
            Some("d0._http._tcp.local.")
        );
        assert!(!c.is_done());
        c.advance();
        assert_eq!(
            c.current().map(DeviceId::as_str),
            Some("d1._http._tcp.local.")
        );
        c.advance();
        assert!(c.is_done());
        assert_eq!(c.current(), None);
    }

    #[test]
    fn empty_cursor_is_immediately_done() {
        let c = PassCursor::new(ids(0));
        assert!(c.is_done());
        assert_eq!(c.current(), None);
    }

    #[test]
    fn cursor_iterates_exactly_its_snapshot_in_order() {
        let mut c = PassCursor::new(ids(3));
        let mut seen = Vec::new();
        while let Some(id) = c.current() {
            seen.push(id.as_str().to_owned());
            c.advance();
        }
        assert_eq!(
            seen,
            vec![
                "d0._http._tcp.local.".to_owned(),
                "d1._http._tcp.local.".to_owned(),
                "d2._http._tcp.local.".to_owned(),
            ]
        );
        assert!(c.is_done());
    }

    #[test]
    fn reachable_only_when_an_endpoint_succeeded() {
        use EndpointOutcome::{AuthFailed, Failed, Ok};
        assert!(!pass_reachable(&[]));
        assert!(!pass_reachable(&[Failed, AuthFailed]));
        assert!(pass_reachable(&[Failed, Ok, Failed]));
    }

    #[test]
    fn fail_pending_only_touches_unreported_endpoints() {
        use EndpointOutcome::{AuthFailed, Failed, Ok};
        let mut outcomes = vec![Some(Ok), None, Some(AuthFailed)];
        fail_pending(&mut outcomes);
        assert_eq!(
            outcomes,
            vec![Some(Ok), Some(Failed), Some(AuthFailed)],
            "pending becomes Failed; already-reported outcomes stay",
        );
    }

    #[test]
    fn a_surviving_ok_keeps_the_pass_reachable_when_a_relogin_fails() {
        use EndpointOutcome::Ok;
        // A reauth's re-login send is rejected after an endpoint already returned
        // Ok — the pass must stay reachable, not be recorded unreachable.
        let mut outcomes = vec![Some(Ok), None, None];
        fail_pending(&mut outcomes);
        let resolved: Vec<EndpointOutcome> = outcomes.into_iter().flatten().collect();
        assert!(pass_reachable(&resolved));
    }

    #[test]
    fn reauth_decision_finalizes_when_no_auth_failure() {
        use EndpointOutcome::{Failed, Ok};
        assert_eq!(
            reauth_decision(&[Ok, Failed], false),
            ReauthDecision::Finalize
        );
    }

    #[test]
    fn reauth_decision_refires_only_auth_failed_endpoints() {
        use EndpointOutcome::{AuthFailed, Ok};
        assert_eq!(
            reauth_decision(&[Ok, AuthFailed, AuthFailed], false),
            ReauthDecision::Reauth {
                endpoints: vec![1, 2]
            }
        );
    }

    #[test]
    fn reauth_decision_finalizes_once_already_reauthed() {
        use EndpointOutcome::AuthFailed;
        assert_eq!(
            reauth_decision(&[AuthFailed], true),
            ReauthDecision::Finalize
        );
    }

    #[test]
    fn adapter_for_maps_every_supported_family() {
        assert_eq!(
            adapter_for(DeviceFamily::Bos).map(FamilyAdapter::browse_service_types),
            Some(crate::families::bos::BOS_SERVICE_TYPES)
        );
        assert_eq!(
            adapter_for(DeviceFamily::Ubos).map(FamilyAdapter::browse_service_types),
            Some(crate::families::ubos::UBOS_SERVICE_TYPES)
        );
        assert_eq!(
            adapter_for(DeviceFamily::Bitaxe).map(FamilyAdapter::browse_service_types),
            Some(crate::families::bitaxe::BITAXE_SERVICE_TYPES)
        );
    }
}
