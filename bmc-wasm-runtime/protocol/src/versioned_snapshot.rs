// Copyright (C) 2026  Braiins Systems s.r.o.

//! Reusable host-side container for "snapshot + version + cached encoded bytes".
//!
//! Each guest-observable channel that ships from host to wasm
//! (`params`, system settings, …) follows the same shape:
//! a typed source-of-truth value, an opaque version counter
//! the SDK reads via a `host_*_version` import, and a lazily-encoded
//! wire-format buffer that the `host_*_snapshot` import fills
//! into guest memory.
//!
//! [`VersionedSnapshotCache`] holds those three together so the encapsulation
//! invariant (mutate only via [`Self::replace`], cache invalidates atomically)
//! is enforced once and reused everywhere. Channels just thread their own
//! payload type through it.

extern crate alloc;
use alloc::vec::Vec;

/// Encode a snapshot value into the channel's packed wire format.
///
/// The wire format itself is the channel's concern;
/// this trait just abstracts the "T → Vec<u8>" step
/// so [`VersionedSnapshotCache`] can stay generic.
pub trait WireEncode {
    fn encode(&self) -> Vec<u8>;
}

/// Host-side snapshot store with version counter and lazily-filled encoded cache.
///
/// Mutate via [`Self::replace`] — direct field writes are blocked at compile time so
/// the version bump and cache invalidation can't drift from the source-of-truth value.
/// Reads via [`Self::version`], [`Self::snapshot`], [`Self::encoded`]; the last fills
/// the cache on first call after each replacement, so a guest spinning on the matching
/// `host_*_snapshot` import reuses the encoded bytes until the next mutation.
#[derive(Debug)]
pub struct VersionedSnapshotCache<T> {
    snapshot: T,
    /// Opaque change marker. "Different = changed" semantics; wrapping is fine.
    version: u64,
    /// Filled on first [`Self::encoded`] call after each [`Self::replace`].
    cached_encoded: Option<Vec<u8>>,
}

impl<T: WireEncode> VersionedSnapshotCache<T> {
    /// Construct with `initial` as the starting snapshot.
    /// Version starts at 0; the cache is empty until the first [`Self::encoded`] call.
    pub const fn new(initial: T) -> Self {
        Self {
            snapshot: initial,
            version: 0,
            cached_encoded: None,
        }
    }

    /// Atomically replace the snapshot, bump the version, invalidate the cache.
    /// The only correct way to mutate the contained value.
    pub fn replace(&mut self, new: T) {
        self.snapshot = new;
        self.version = self.version.wrapping_add(1);
        self.cached_encoded = None;
    }

    /// Opaque change marker.
    /// Returned to guests by the channel's `host_*_version` import.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Borrow the current snapshot value.
    /// Read-only — mutation goes through [`Self::replace`].
    pub fn snapshot(&self) -> &T {
        &self.snapshot
    }

    /// Encoded snapshot bytes in the channel's wire format.
    ///
    /// Lazily fills the cache on first call after each [`Self::replace`];
    /// subsequent calls without an intervening mutation return
    /// the cached buffer without re-encoding.
    pub fn encoded(&mut self) -> &[u8] {
        if self.cached_encoded.is_none() {
            self.cached_encoded = Some(self.snapshot.encode());
        }
        self.cached_encoded
            .as_deref()
            .expect("BUG: just inserted Some")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test snapshot whose `encode` reflects its current value,
    /// so the round-trip behaviours of [`VersionedSnapshotCache`]
    /// are observable from the encoded bytes.
    struct TestSnap(u32);

    impl WireEncode for TestSnap {
        fn encode(&self) -> Vec<u8> {
            self.0.to_le_bytes().to_vec()
        }
    }

    #[test]
    fn version_starts_at_zero_and_bumps_on_each_replace() {
        let mut cache = VersionedSnapshotCache::new(TestSnap(0));
        assert_eq!(cache.version(), 0);
        cache.replace(TestSnap(1));
        assert_eq!(cache.version(), 1);
        cache.replace(TestSnap(2));
        assert_eq!(cache.version(), 2);
    }

    #[test]
    fn encoded_reflects_current_snapshot_after_replace() {
        let mut cache = VersionedSnapshotCache::new(TestSnap(0));
        assert_eq!(cache.encoded(), &0_u32.to_le_bytes());
        cache.replace(TestSnap(42));
        assert_eq!(cache.encoded(), &42_u32.to_le_bytes());
    }

    #[test]
    fn encoded_returns_same_bytes_across_calls_without_mutation() {
        let mut cache = VersionedSnapshotCache::new(TestSnap(7));
        let first = cache.encoded().to_vec();
        let second = cache.encoded().to_vec();
        assert_eq!(first, second);
    }

    #[test]
    fn replace_invalidates_cache_so_next_encoded_call_reflects_new_value() {
        let mut cache = VersionedSnapshotCache::new(TestSnap(7));
        let _ = cache.encoded(); // fill cache with 7's encoding
        cache.replace(TestSnap(99));
        assert_eq!(cache.encoded(), &99_u32.to_le_bytes());
    }
}
