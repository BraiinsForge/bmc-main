// Copyright (C) 2026  Braiins Systems s.r.o.

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

/// A one-pass cursor over a snapshot of device ids. Snapshotting at pass start
/// means devices added or removed mid-pass are only seen on the next pass.
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

/// The result of fetching one telemetry endpoint in a pass.
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

/// A family driver's contribution to frame scheduling.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FamilyWake {
    /// No devices and not counting down — contributes no timer.
    Idle,
    /// A pass is in progress; progress is driven by fetch delivery, not a
    /// timer. Contributes no timer (returning one would busy-render).
    Active,
    /// Between passes; the value is the remaining ms until the next pass.
    Waiting(u32),
}

/// The single next frame-after delay to arm: the soonest pending pass across
/// the families. `Active`/`Idle` families never contribute, so a slow mid-pass
/// family cannot stretch another family's cadence.
#[must_use]
pub fn next_wake(wakes: &[FamilyWake]) -> Option<u32> {
    wakes
        .iter()
        .filter_map(|w| match w {
            FamilyWake::Waiting(ms) => Some(*ms),
            FamilyWake::Idle | FamilyWake::Active => None,
        })
        .min()
}

#[cfg(target_arch = "wasm32")]
mod driver;

#[cfg(target_arch = "wasm32")]
pub use driver::{clear_tokens, ensure_running, on_frame, remove_token};

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
    fn next_wake_is_min_of_waiting_families() {
        use FamilyWake::{Active, Idle, Waiting};
        assert_eq!(
            next_wake(&[Waiting(30_000), Active, Waiting(12_000)]),
            Some(12_000)
        );
        assert_eq!(next_wake(&[Idle, Waiting(5_000), Idle]), Some(5_000));
    }

    #[test]
    fn next_wake_arms_nothing_when_no_family_is_waiting() {
        use FamilyWake::{Active, Idle};
        assert_eq!(next_wake(&[Active, Active, Idle]), None);
        assert_eq!(next_wake(&[]), None);
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
