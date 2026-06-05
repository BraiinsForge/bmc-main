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

/// A device is reachable when its latest pass obtained usable telemetry from
/// at least one endpoint. Login failure leaves every endpoint failed, so this
/// also captures "shared-password rejected -> unreachable".
#[must_use]
pub fn pass_reachable(endpoint_oks: &[bool]) -> bool {
    endpoint_oks.iter().any(|&ok| ok)
}

#[cfg(target_arch = "wasm32")]
mod driver;

#[cfg(target_arch = "wasm32")]
pub use driver::{clear_tokens, ensure_running, on_frame, remove_token};

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<DeviceId> {
        (0..n)
            .map(|i| DeviceId::new(format!("d{i}._http._tcp.local.")))
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
        assert!(!pass_reachable(&[]));
        assert!(!pass_reachable(&[false, false]));
        assert!(pass_reachable(&[false, true, false]));
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
