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

//! Guest-side widget parameter snapshots.
//!
//! The reader, [`Params`], and its wire format live in `bmc_wasm_protocol::params`;
//! this module fetches the snapshots the host delivers and keeps the previous one around.
//!
//! ## Snapshot lifecycle
//!
//! [`current`] returns the latest snapshot the host has delivered.
//! The first call from `init` reads via the `host_params_snapshot` import; subsequent calls
//! reuse the cached bytes until the host bumps `host_params_version`, at which point the next
//! [`current`] re-fetches and the old snapshot is moved into [`previous`].
//!
//! [`previous`] returns the snapshot immediately before [`current`].
//! It is [`Params::default`] until at least one update has been observed.
//! `on_params_update` is the canonical place to diff the two for React-style change detection.
//!
//! ## End-to-end byte flow
//!
//! ```text
//! host BTreeMap<ParamKey, ParamValue>
//!   └─ encode_params  →  Vec<u8> in the wire format above
//!        └─ extern fn host_params_snapshot  (probe-then-fetch via wasm imports)
//!             └─ thread-local ParamsCache { current, previous, last_seen_version }
//!                  └─ snapshot::current()  →  Params { bytes: Vec<u8> }   (clone of cached buffer)
//!                       └─ Params::get_str / get_i32 / …
//!                            →  linear scan for the matching key
//!                            →  Option<&str>  borrowed into snap.bytes
//!                       └─ typed::ParamRead::read_required / read_optional
//!                            →  .to_owned() for String, by value for primitives
//!                            →  from_manifest_value(...) for enum_values fields
//! ```
//!
//! - `host_params_version` is a cheap separate import; idle frames don't cross the
//!   wasm boundary for params at all (the cache short-circuits when the version is unchanged).
//! - The fetch is probe-then-allocate: `host_params_snapshot(null, 0)` returns the
//!   required byte length; the SDK allocates an exact `Vec<u8>` and calls back to fill it.
//! - `Params::current()` clones the cache's `Vec<u8>` so the caller owns a snapshot
//!   independent of subsequent host updates. The clone is the only allocation per frame
//!   when nothing changed; accessors borrow into the cloned buffer until you copy out.

pub mod typed;

pub use bmc_wasm_protocol::params::{List, ListIter, Object, Params, Value};

// `FromHostBytes` lets the generic snapshot cache construct `Params` from the raw bytes
// the host writes. The wasm32 public API + the test module are the only consumers; native
// non-test builds don't pull in the cache machinery at all.
#[cfg(any(target_arch = "wasm32", test))]
impl crate::snapshot_cache::FromHostBytes for Params {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_bytes(bytes)
    }
}

/// Latest snapshot delivered for this widget instance.
///
/// First call inside `init` fetches via `host_params_snapshot`.
/// Subsequent calls reuse the cached bytes until `host_params_version` changes;
/// at that point the old snapshot is moved into [`previous`] and the new one is fetched.
///
/// Always empty off-device, where no host delivers a snapshot.
#[must_use]
pub fn current() -> Params {
    Platform::current()
}

/// Snapshot delivered immediately before [`current`].
///
/// [`Params::default`] until at least one update has been observed (i.e. during `init`
/// and the first `render` of any widget life).
///
/// Inside `on_params_update`, holds the just-replaced snapshot — diff against [`current`]
/// to react only to keys whose value actually changed.
///
/// Always empty off-device, where no host delivers a snapshot.
#[must_use]
pub fn previous() -> Params {
    Platform::previous()
}

/// Monotonic version of the latest params snapshot the host has delivered.
/// Cheap host call (no buffer copy); use it to gate per-frame re-parses.
///
/// Always `0` off-device, where no host delivers a snapshot.
#[must_use]
pub fn version() -> u64 {
    Platform::snapshot_version()
}

trait ParamsSnapshotProvider {
    fn current() -> Params;
    fn previous() -> Params;
    fn snapshot_version() -> u64;
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm_test;
    use bmc_wasm_protocol::params::kind;

    /// One packed entry mapping `key` to the string `value`, in the host's wire format.
    fn packed_str(key: &str, value: &str) -> Vec<u8> {
        let mut out = 1_u32.to_le_bytes().to_vec();
        out.push(kind::STR);
        let key_len =
            u16::try_from(key.len()).expect("BUG: test fixtures always use keys well under 64 KiB");
        out.extend_from_slice(&key_len.to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        let len = u32::try_from(value.len())
            .expect("BUG: test fixtures always use values well under 4 GiB");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod off_device {
        use super::*;

        #[test]
        fn current_and_previous_are_default_on_native() {
            // On `wasm32`, [`current`] and [`previous`] call the host imports.
            // On native (this test target), those imports don't exist; the native fallback returns
            // [`Params::default`] so non-wasm consumers of the SDK API compile and behave consistently.
            // End-to-end coverage of the wasm path lives in `bmc-wasm-runtime` integration tests.
            assert!(current().is_empty());
            assert!(previous().is_empty());
        }
    }

    /// Test-side fake of [`crate::snapshot_cache::HostSnapshotProvider`].
    /// Holds a current `(version, snapshot)` pair the test sets up; the SDK's cache logic
    /// pulls from it via the trait without touching wasm imports.
    struct MockHost {
        version: u64,
        snapshot: Vec<u8>,
    }

    impl MockHost {
        fn new() -> Self {
            Self {
                version: 0,
                snapshot: Vec::new(),
            }
        }

        fn set(&mut self, version: u64, snapshot: Vec<u8>) {
            self.version = version;
            self.snapshot = snapshot;
        }
    }

    impl crate::snapshot_cache::HostSnapshotProvider for MockHost {
        fn version(&self) -> u64 {
            self.version
        }

        fn fill_snapshot(&self, out: &mut [u8]) -> usize {
            if out.is_empty() {
                // Probe path: return required byte length without writing.
                return self.snapshot.len();
            }
            let to_write = self.snapshot.len().min(out.len());
            out[..to_write].copy_from_slice(&self.snapshot[..to_write]);
            to_write
        }
    }

    #[wasm_test]
    fn previous_first_after_version_bump_returns_just_replaced_snapshot() {
        // Channel-specific regression test for the BDK-432 #1 bug, exercised end-to-end through
        // the params wire format. The companion test in `snapshot_cache::tests` covers the
        // generic rotation logic; this one pins the Params type's parser-side decoding against
        // the same scenario.
        use crate::snapshot_cache::{Cache, current_using, previous_using};

        let mut host = MockHost::new();
        let mut cache = Cache::<Params>::new();

        // Seed version 1 by reading through `current_using`.
        host.set(1, packed_str("a", "v1"));
        let cur_v1 = current_using(&host, &mut cache);
        assert_eq!(cur_v1.get_str("a"), Some("v1"));
        assert!(
            previous_using(&host, &mut cache).is_empty(),
            "before any rotation, previous is empty"
        );

        // Host bumps to version 2. Critical: read `previous` FIRST.
        host.set(2, packed_str("a", "v2"));
        let prev_after_bump = previous_using(&host, &mut cache);
        let cur_after_bump = current_using(&host, &mut cache);

        assert_eq!(
            prev_after_bump.get_str("a"),
            Some("v1"),
            "previous() called before current() after a version bump must return the \
             just-replaced snapshot, not the version-before-previous (i.e. the empty default)"
        );
        assert_eq!(cur_after_bump.get_str("a"), Some("v2"));
    }
}
