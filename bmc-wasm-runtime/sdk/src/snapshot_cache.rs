// Copyright (C) 2026  Braiins Systems s.r.o.

//! Guest-side rotating-snapshot cache, shared across the channels
//! that ship from host to wasm (`params`, system settings, …).
//!
//! Each channel exposes the same pair of host imports: a version probe
//! and a probe-then-fetch snapshot reader. The SDK keeps `current` + `previous`
//! typed snapshots per channel, refreshes them only when the host's version differs,
//! and rotates `current → previous` on each version bump so widgets can diff against
//! the just-replaced state inside the matching channel's lifecycle hook
//! (`on_params_update` for the params channel, `on_system_update` for
//! the system channel).
//!
//! This module owns the generic machinery; each channel module
//! (e.g. `params`, `system` later) plugs in its own snapshot type
//! via [`FromHostBytes`] and its own wasm-target externs
//! via a [`HostSnapshotProvider`] impl.

/// Host-side bindings for a single snapshot channel.
///
/// The wasm-target implementation wraps the channel's `host_*_version`
/// and `host_*_snapshot` externs; tests swap in a fake to drive
/// the cache logic from native code without crossing the wasm boundary.
pub trait HostSnapshotProvider {
    /// Opaque change marker — different value from last observation = re-fetch.
    fn version(&self) -> u64;
    /// Probe-then-fill snapshot reader. `out` empty = probe path, returns
    /// required byte length. `out` sized = fill path, returns bytes actually written.
    fn fill_snapshot(&self, out: &mut [u8]) -> usize;
}

/// Construct a channel's typed snapshot from the raw wire bytes the host wrote.
///
/// Each channel implements this for its own snapshot type so [`refresh_cache`]
/// can stay generic — the cache machinery treats every channel uniformly
/// and the channel module is the only place that knows the wire format.
pub trait FromHostBytes: Default {
    fn from_bytes(bytes: Vec<u8>) -> Self;
}

/// Rotating snapshot cache for a single channel.
///
/// Holds the current and previous typed snapshots plus the last host version observed.
/// `last_seen_version == None` distinguishes "host returned 0" from "never fetched"
/// — the first observation seeds `current` and leaves `previous` at default.
pub struct Cache<T> {
    current: T,
    previous: T,
    last_seen_version: Option<u64>,
}

impl<T: Default> Cache<T> {
    pub fn new() -> Self {
        Self {
            current: T::default(),
            previous: T::default(),
            last_seen_version: None,
        }
    }
}

impl<T: Default> Default for Cache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Refresh the cache if the host's reported version differs from the last seen.
///
/// On a version bump, fetches the new snapshot through `host` and rotates:
///  - old `current` becomes the new `previous`,
///  - old `previous` is dropped. Idempotent on repeat calls with the same version (short-circuits via `last_seen_version`).
pub fn refresh_cache<T: FromHostBytes, H: HostSnapshotProvider>(host: &H, cache: &mut Cache<T>) {
    let host_version = host.version();
    if cache.last_seen_version == Some(host_version) {
        return;
    }

    // Probe path: empty buffer, host returns required byte length.
    let needed = host.fill_snapshot(&mut []);
    let mut buf = vec![0_u8; needed];
    let written = if needed > 0 {
        host.fill_snapshot(&mut buf)
    } else {
        0
    };
    buf.truncate(written);

    // Rotate: old `current` becomes the new `previous`.
    // `previous` from before this update is dropped — only one step of history is kept.
    let new_current = T::from_bytes(buf);
    let old_current = core::mem::replace(&mut cache.current, new_current);
    cache.previous = old_current;
    cache.last_seen_version = Some(host_version);
}

/// Latest snapshot delivered for this channel.
///
/// Refreshes the cache if the host has bumped the version since last observation.
pub fn current_using<T: FromHostBytes + Clone, H: HostSnapshotProvider>(
    host: &H,
    cache: &mut Cache<T>,
) -> T {
    refresh_cache(host, cache);
    cache.current.clone()
}

/// Snapshot delivered immediately before the current one.
///
/// Refreshes the cache so the read is consistent regardless
/// of whether [`current_using`] has been called yet — a widget
/// that reads `previous()` before `current()` after a version bump
/// must still see the just-replaced snapshot, not the one before that.
pub fn previous_using<T: FromHostBytes + Clone, H: HostSnapshotProvider>(
    host: &H,
    cache: &mut Cache<T>,
) -> T {
    refresh_cache(host, cache);
    cache.previous.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test snapshot type — wraps the raw bytes so we can assert what was fetched.
    #[derive(Clone, Default, Debug, PartialEq, Eq)]
    struct TestSnap(Vec<u8>);

    impl FromHostBytes for TestSnap {
        fn from_bytes(bytes: Vec<u8>) -> Self {
            Self(bytes)
        }
    }

    struct MockHost {
        version: u64,
        snapshot: Vec<u8>,
    }

    impl HostSnapshotProvider for MockHost {
        fn version(&self) -> u64 {
            self.version
        }

        fn fill_snapshot(&self, out: &mut [u8]) -> usize {
            if out.is_empty() {
                return self.snapshot.len();
            }
            let n = self.snapshot.len().min(out.len());
            out[..n].copy_from_slice(&self.snapshot[..n]);
            n
        }
    }

    #[test]
    fn current_returns_freshly_fetched_snapshot_on_version_bump() {
        let mut host = MockHost {
            version: 1,
            snapshot: vec![1, 2, 3],
        };
        let mut cache = Cache::<TestSnap>::new();
        assert_eq!(current_using(&host, &mut cache), TestSnap(vec![1, 2, 3]));
        host.version = 2;
        host.snapshot = vec![9, 9];
        assert_eq!(current_using(&host, &mut cache), TestSnap(vec![9, 9]));
    }

    #[test]
    fn previous_returns_default_before_any_rotation() {
        let host = MockHost {
            version: 1,
            snapshot: vec![1, 2],
        };
        let mut cache = Cache::<TestSnap>::new();
        let _ = current_using(&host, &mut cache);
        assert_eq!(previous_using(&host, &mut cache), TestSnap::default());
    }

    #[test]
    fn previous_first_after_version_bump_returns_just_replaced_snapshot() {
        // Companion to BDK-432 #1's regression test, now exercised at the generic layer.
        let mut host = MockHost {
            version: 1,
            snapshot: vec![1, 1, 1],
        };
        let mut cache = Cache::<TestSnap>::new();
        let _ = current_using(&host, &mut cache); // seed v=1
        host.version = 2;
        host.snapshot = vec![2, 2, 2];
        let prev = previous_using(&host, &mut cache);
        let cur = current_using(&host, &mut cache);
        assert_eq!(prev, TestSnap(vec![1, 1, 1]));
        assert_eq!(cur, TestSnap(vec![2, 2, 2]));
    }
}
