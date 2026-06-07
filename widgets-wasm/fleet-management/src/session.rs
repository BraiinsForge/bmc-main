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

/// A family driver's lifecycle phase. `Waiting` and `Active` both hold a
/// cursor; they are told apart by whether an opening kick is still pending.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// No devices: nothing scheduled.
    Idle,
    /// Between passes: the next pass's opening fetch is scheduled via
    /// `send_after` and has not yet been delivered.
    Waiting,
    /// A pass is in progress; the opening fetch was sent and fetches are
    /// in flight.
    Active,
}

/// What to do when a device is discovered for a family.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiscoveryAction {
    /// Begin a pass now with an immediate opening fetch.
    StartNow,
    /// A scheduled kick is already in flight; let it drive the pass.
    LetRun,
    /// A pass is already active; the device joins the next snapshot.
    Ignore,
}

/// What to do when a device leaves a family.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RemovalAction {
    /// The device was the in-flight one; abandon it and advance the cursor.
    Abandon,
    /// The device's scheduled kick was cancelled; re-defer the next pass.
    Redefer,
    /// The kick already fired for the gone device; let it fail and advance.
    LetRun,
    /// Nothing to do for this family.
    Ignore,
}

/// Decide the response to a discovery. `is_new` is whether the event added a
/// device not previously listed. `kick_cancelled` is the result of cancelling
/// the pending kick — `Some(true)` when the queued fetch was removed before
/// firing, `Some(false)` when it was already in flight, and `None` when the
/// phase had no kick to cancel.
///
/// While `Waiting`, only a genuinely new device collapses the inter-pass gap;
/// mDNS re-announcements of an already-known device (cache refresh, periodic
/// re-announce, our own browse) leave the parked kick alone so the poll cadence
/// holds instead of restarting back-to-back.
#[must_use]
pub fn on_discovery(phase: Phase, is_new: bool, kick_cancelled: Option<bool>) -> DiscoveryAction {
    match phase {
        Phase::Idle => DiscoveryAction::StartNow,
        Phase::Active => DiscoveryAction::Ignore,
        Phase::Waiting if !is_new => DiscoveryAction::LetRun,
        Phase::Waiting => match kick_cancelled {
            Some(true) => DiscoveryAction::StartNow,
            Some(false) | None => DiscoveryAction::LetRun,
        },
    }
}

/// Decide the response to a removal. `removed_is_focus` is whether the gone
/// device is the family's current focus — the in-flight device when `Active`,
/// the parked device-0 when `Waiting`. `kick_cancelled` mirrors
/// [`on_discovery`].
#[must_use]
pub fn on_removal(
    phase: Phase,
    removed_is_focus: bool,
    kick_cancelled: Option<bool>,
) -> RemovalAction {
    match phase {
        Phase::Idle => RemovalAction::Ignore,
        Phase::Active => {
            if removed_is_focus {
                RemovalAction::Abandon
            } else {
                RemovalAction::Ignore
            }
        }
        Phase::Waiting => {
            if !removed_is_focus {
                return RemovalAction::Ignore;
            }
            match kick_cancelled {
                Some(true) => RemovalAction::Redefer,
                Some(false) | None => RemovalAction::LetRun,
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod driver;

#[cfg(target_arch = "wasm32")]
pub use driver::{
    clear_tokens_for, ensure_running, family_enabled, on_discovered, refresh_params, remove_token,
    stop,
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
    fn discovery_starts_a_pass_only_from_idle() {
        assert_eq!(
            on_discovery(Phase::Idle, true, None),
            DiscoveryAction::StartNow
        );
    }

    #[test]
    fn discovery_is_ignored_while_a_pass_is_active() {
        assert_eq!(
            on_discovery(Phase::Active, true, None),
            DiscoveryAction::Ignore
        );
    }

    #[test]
    fn discovery_restarts_when_the_pending_kick_is_cancelled() {
        assert_eq!(
            on_discovery(Phase::Waiting, true, Some(true)),
            DiscoveryAction::StartNow
        );
    }

    #[test]
    fn discovery_lets_an_in_flight_kick_run() {
        assert_eq!(
            on_discovery(Phase::Waiting, true, Some(false)),
            DiscoveryAction::LetRun
        );
    }

    #[test]
    fn reannouncement_while_waiting_does_not_collapse_the_interval() {
        // A re-announced (already-known) device must not restart the pass: the
        // parked kick stays, so the 30 s cadence holds. `kick_cancelled` is
        // `None` because the driver does not even cancel the kick in this case.
        assert_eq!(
            on_discovery(Phase::Waiting, false, None),
            DiscoveryAction::LetRun
        );
    }

    #[test]
    fn removal_abandons_the_in_flight_device() {
        assert_eq!(
            on_removal(Phase::Active, true, None),
            RemovalAction::Abandon
        );
    }

    #[test]
    fn removal_of_a_non_focus_device_is_ignored() {
        assert_eq!(
            on_removal(Phase::Active, false, None),
            RemovalAction::Ignore
        );
        assert_eq!(
            on_removal(Phase::Waiting, false, None),
            RemovalAction::Ignore
        );
        assert_eq!(on_removal(Phase::Idle, false, None), RemovalAction::Ignore);
    }

    #[test]
    fn removal_redefers_when_the_parked_kick_is_cancelled() {
        assert_eq!(
            on_removal(Phase::Waiting, true, Some(true)),
            RemovalAction::Redefer
        );
    }

    #[test]
    fn removal_lets_an_in_flight_kick_to_a_gone_device_run() {
        assert_eq!(
            on_removal(Phase::Waiting, true, Some(false)),
            RemovalAction::LetRun
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
