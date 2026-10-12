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

//! Guest-side deck-wide system snapshot.
//!
//! Parallel to [`crate::params`]: the reader, [`Snapshot`], its wire format and the
//! [`SnapshotBuilder`] mirroring the host encoder live in `bmc_wasm_protocol::system`;
//! this module fetches the snapshots the host delivers and keeps the previous one around.
//!
//! ## Snapshot lifecycle
//!
//! Mirrors [`crate::params`]: [`current`] returns the latest snapshot,
//! [`previous`] the one just before it. The `on_system_update`
//! lifecycle hook fires on system-snapshot changes (sibling of `on_params_update`
//! for the system channel); diff `current()` vs `previous()` inside the hook
//! to react only to fields whose value actually changed.

// `SystemFieldKind` stays out: it tags the wire entries, which only the reader walks.
pub use bmc_wasm_protocol::system::{
    DateFormat, NextAlarmView, NumberFormat, Snapshot, SnapshotBuilder, TemperatureUnit,
    TimeFormat, UnitSystem, Weekday,
};

// `FromHostBytes` lets the generic snapshot cache construct `Snapshot`
// from the raw bytes the host writes. Mirror of the params impl.
#[cfg(any(target_arch = "wasm32", test))]
impl crate::snapshot_cache::FromHostBytes for Snapshot {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_bytes(bytes)
    }
}

/// Latest system snapshot delivered for this widget instance.
///
/// First call inside `init` fetches via `host_system_snapshot`. Subsequent
/// calls reuse the cached bytes until `host_system_version` changes; at that
/// point the old snapshot is moved into [`previous`] and the new one is
/// fetched.
///
/// Off-device there is no host to deliver a snapshot, so one is held per
/// thread. It starts empty: every accessor `None`, every caller left on
/// its own default.
///
/// `set_current` installs one, which is how the storybook and the tests
/// render a view the way an operator on imperial units, or a twelve-hour
/// clock, would see it.
#[must_use]
pub fn current() -> Snapshot {
    Platform::current()
}

/// Snapshot delivered immediately before [`current`].
///
/// [`Snapshot::default`] until at least one update has been observed.
/// Inside `on_system_update`, holds the just-replaced snapshot
/// — diff against [`current`] to react only to fields whose value actually changed.
///
/// Always empty in a native build: nothing delivers a second snapshot,
/// so diffing it against [`current`] reports every field as changed.
/// Under the wasm test host, `set_current` stands in for a device update:
/// the snapshot it replaces moves here on the next read, not at the install.
#[must_use]
pub fn previous() -> Snapshot {
    Platform::previous()
}

trait SystemSnapshotProvider {
    fn current() -> Snapshot;
    fn previous() -> Snapshot;
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
#[cfg(not(target_arch = "wasm32"))]
pub use native::set_current;
#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;
#[cfg(all(target_arch = "wasm32", any(test, feature = "test-support")))]
pub use wasm::set_current;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn current_and_previous_are_none_on_native() {
        assert_eq!(current().timezone(), None);
        assert_eq!(previous().timezone(), None);
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod test_host {
    use super::*;
    use crate::wasm_test;

    #[wasm_test]
    fn a_second_install_rotates_the_first_into_previous() {
        set_current(SnapshotBuilder::new().timezone("Europe/Prague").build());
        assert_eq!(current().timezone(), Some("Europe/Prague"));
        set_current(SnapshotBuilder::new().timezone("Asia/Tokyo").build());

        assert_eq!(current().timezone(), Some("Asia/Tokyo"));
        assert_eq!(previous().timezone(), Some("Europe/Prague"));
    }

    #[wasm_test]
    fn installs_without_a_read_between_rotate_nothing() {
        set_current(SnapshotBuilder::new().timezone("Europe/Prague").build());
        set_current(SnapshotBuilder::new().timezone("Asia/Tokyo").build());

        assert_eq!(previous().timezone(), None);
        assert_eq!(current().timezone(), Some("Asia/Tokyo"));
    }
}
