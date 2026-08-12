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

//! Background connectivity prober shared by OS-driven overlays.
//!
//! A detached thread probes once per second via [`bmc_net_observe::probe`] and
//! publishes a [`Snapshot`]. Overlay ticks read it via [`snapshot_if_changed`]
//! and never block on the kernel's rtnl lock. The probe is observational — the
//! actual network reading lives in `bmc-net-observe`; this module only owns the
//! thread and the change-versioned publish/read handoff.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use std::time::Duration;

pub use bmc_net_observe::Snapshot;
use bmc_net_observe::probe;

/// Opaque change marker of a published [`Snapshot`]. Returned by
/// [`snapshot_if_changed`] and handed back as `seen` on the next poll;
/// "different = changed" is the only semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotVersion(NonZeroU64);

impl SnapshotVersion {
    /// Version of the prober's first publish; a fixed point for test doubles
    /// that fake a single published snapshot.
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    /// The version after `self`, for test doubles faking a sequence
    /// of publishes.
    #[must_use]
    pub fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("BUG: SnapshotVersion overflowed u64"),
        )
    }
}

/// A published [`Snapshot`] paired with its change marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedSnapshot {
    /// Change marker to hand back as `seen` on the next poll.
    pub version: SnapshotVersion,
    /// The published readings.
    pub snapshot: Snapshot,
}

/// Publisher/reader pair between the prober thread and overlay ticks. The
/// mutex is held only to swap or clone the value, never across a probe.
///
/// The raw `version` counter is 0 until the first publish (readers see that
/// as "no version yet") and bumps only when the published content differs
/// from the previous snapshot. It is incremented exclusively while the mutex
/// is held, so a (version, snapshot) pair read under the lock is always
/// consistent; the lock-free load in [`Self::read_if_changed`] is only a
/// cheap "anything new?" gate.
#[derive(Default)]
struct ProbeState {
    version: AtomicU64,
    snapshot: Mutex<Option<Snapshot>>,
}

impl ProbeState {
    fn publish(&self, snapshot: Snapshot) {
        let mut guard = self.lock();
        if guard.as_ref() != Some(&snapshot) {
            self.version.fetch_add(1, Ordering::Relaxed);
            *guard = Some(snapshot);
        }
    }

    /// Latest snapshot with its version, or `None` when the version still
    /// equals `seen` (or nothing has been published yet). The unchanged case
    /// is one atomic load — no lock, no allocation — so this is safe to poll
    /// per frame.
    fn read_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
        let seen = seen.map_or(0, |v| v.0.get());
        // Relaxed suffices: this is only a gate. A reader that passes it
        // synchronizes through the mutex acquire below and re-reads the
        // version there; one that races a bump just catches up next poll.
        if self.version.load(Ordering::Relaxed) == seen {
            return None;
        }
        let guard = self.lock();
        let version = NonZeroU64::new(self.version.load(Ordering::Relaxed)).map(SnapshotVersion)?;
        guard
            .clone()
            .map(|snapshot| VersionedSnapshot { version, snapshot })
    }

    fn lock(&self) -> MutexGuard<'_, Option<Snapshot>> {
        // A panic can only poison a plain value swap or clone, so the inner
        // value is always intact; recover it instead of propagating.
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Pause between probe passes.
const PROBE_PERIOD: Duration = Duration::from_secs(1);

/// Spawn the detached prober thread. The probe path only walks getifaddrs,
/// spawns one subprocess, and parses small strings; 128 KiB of stack is
/// plenty, and the 2 MiB Rust default would waste address space on 32-bit
/// ARM. On spawn failure the snapshot stays `None` forever, which readers
/// treat as "never probed".
fn spawn_prober(state: &'static ProbeState) {
    let spawned = std::thread::Builder::new()
        .name("connectivity-prober".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            loop {
                // AssertUnwindSafe: the pass only produces a value; ProbeState
                // recovers from poisoning, so no broken state can leak.
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(probe)) {
                    Ok(Some(snapshot)) => state.publish(snapshot),
                    Ok(None) => tracing::warn!("connectivity probe pass could not read interfaces"),
                    Err(_) => tracing::error!("connectivity probe pass panicked"),
                }
                std::thread::sleep(PROBE_PERIOD);
            }
        });
    if let Err(err) = spawned {
        tracing::error!("failed to spawn connectivity prober thread: {err}");
    }
}

/// Shared prober state; the first access spawns the prober thread.
fn prober_state() -> &'static ProbeState {
    static STATE: ProbeState = ProbeState {
        version: AtomicU64::new(0),
        snapshot: Mutex::new(None),
    };
    static SPAWN: Once = Once::new();
    SPAWN.call_once(|| spawn_prober(&STATE));
    &STATE
}

/// Latest connectivity snapshot and its version, or `None` while the content
/// has not changed since `seen` (pass `None` initially, then the last
/// returned version — `None` also covers "prober has not published yet",
/// possibly forever if its thread failed to spawn). The unchanged case does
/// no allocation, so this is safe to poll on a per-frame animation tick.
/// Spawns the prober on first call; never blocks beyond a value-swap mutex.
#[must_use]
pub fn snapshot_if_changed(seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
    prober_state().read_if_changed(seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn probe_state_read_returns_latest_publish() {
        let state = ProbeState::default();
        state.publish(Snapshot {
            ipv4: None,
            station_ssid: None,
            wifi_signal_dbm: None,
        });
        state.publish(Snapshot {
            ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
            station_ssid: Some("Office WiFi".to_owned()),
            wifi_signal_dbm: Some(-52),
        });
        assert_eq!(
            state.read_if_changed(None).map(|update| update.snapshot),
            Some(Snapshot {
                ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
                station_ssid: Some("Office WiFi".to_owned()),
                wifi_signal_dbm: Some(-52),
            })
        );
    }

    #[test]
    fn read_if_changed_returns_none_until_first_publish() {
        let state = ProbeState::default();
        assert_eq!(state.read_if_changed(None), None);
    }

    // The tray polls on a ~30 Hz animation tick; the version gate is what lets
    // those ticks skip the snapshot clone, so an unchanged re-publish (the
    // prober re-reads every second) must not look like a change.
    #[test]
    fn identical_republish_does_not_bump_version() {
        let state = ProbeState::default();
        let snapshot = Snapshot {
            ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
            station_ssid: Some("Office WiFi".to_owned()),
            wifi_signal_dbm: Some(-52),
        };
        state.publish(snapshot.clone());
        let first = state
            .read_if_changed(None)
            .expect("BUG: first publish must be visible");
        assert_eq!(first.snapshot, snapshot);

        state.publish(snapshot.clone());
        assert_eq!(state.read_if_changed(Some(first.version)), None);
    }

    #[test]
    fn changed_publish_bumps_version_and_returns_new_content() {
        let state = ProbeState::default();
        let offline = Snapshot {
            ipv4: None,
            station_ssid: None,
            wifi_signal_dbm: None,
        };
        let online = Snapshot {
            ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
            station_ssid: Some("Office WiFi".to_owned()),
            wifi_signal_dbm: Some(-52),
        };
        state.publish(offline);
        let first = state
            .read_if_changed(None)
            .expect("BUG: first publish must be visible");

        state.publish(online.clone());
        assert_eq!(
            state
                .read_if_changed(Some(first.version))
                .map(|update| update.snapshot),
            Some(online)
        );
    }
}
