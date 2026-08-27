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

//! Tag-keyed audio sample registry with playback-sink lifecycle.
//!
//! Mirrors the bitmap / icon / mesh registries on the renderer side:
//! registrations are deduped by tag, look up by ID is O(1), and eviction
//! frees both the encoded sample data and any active `rodio::Sink`s for
//! that ID. Eviction invalidates the returned ID and releases its numeric
//! value for a later registration; callers must discard evicted IDs.
//!
//! The active-sink half is gated on `feature = "audio"`. With the feature
//! off, the registry still tracks samples (so callers can record fixture
//! events without an audio backend) but `push_sink` / `stop` are no-ops.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use bmc_wasm_protocol::{AudioId, IdPool};

use crate::host_api::AudioSample;

pub struct AudioRegistry {
    samples: HashMap<AudioId, AudioSample>,
    by_name: HashMap<String, AudioId>,
    ids: IdPool<AudioId>,
    /// Active playback sinks keyed by sample ID.
    /// A single sample may have several overlapping plays; sinks are pruned lazily by `push_sink`.
    #[cfg(feature = "audio")]
    sinks: HashMap<AudioId, Vec<rodio::Sink>>,
}

impl fmt::Debug for AudioRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioRegistry")
            .field("count", &self.samples.len())
            .field("ids", &self.ids)
            .finish_non_exhaustive()
    }
}

impl Default for AudioRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: HashMap::new(),
            by_name: HashMap::new(),
            ids: IdPool::new(u16::MAX),
            #[cfg(feature = "audio")]
            sinks: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_id_cap(exclusive_cap: u16) -> Self {
        Self {
            ids: IdPool::new(exclusive_cap),
            ..Self::new()
        }
    }

    /// Look up an existing tag without registering.
    #[must_use]
    pub fn get_by_tag(&self, tag: &str) -> Option<AudioId> {
        self.by_name.get(tag).copied()
    }

    /// Look up a sample by ID.
    #[must_use]
    pub fn get(&self, id: AudioId) -> Option<&AudioSample> {
        self.samples.get(&id)
    }

    /// Register a sample under `tag`. Idempotent: a second call with the
    /// same tag returns the cached ID; `data` / `duration_ms` from later
    /// calls are ignored.
    pub fn register(&mut self, tag: String, data: Arc<[u8]>, duration_ms: u32) -> Option<AudioId> {
        if let Some(&id) = self.by_name.get(&tag) {
            return Some(id);
        }
        let Some(id) = self.ids.alloc() else {
            tracing::error!("audio registry exhausted ({tag})");
            return None;
        };
        self.samples.insert(
            id,
            AudioSample {
                data,
                name: tag.clone(),
                duration_ms,
            },
        );
        self.by_name.insert(tag, id);
        Some(id)
    }

    /// Evict a single tag: drop the sample, the `tag → AudioId` mapping,
    /// and stop any active playback sinks for that ID.
    /// Returns `true` if a tag was found and evicted.
    pub fn evict(&mut self, tag: &str) -> bool {
        let Some(id) = self.by_name.remove(tag) else {
            return false;
        };
        self.samples.remove(&id);
        #[cfg(feature = "audio")]
        self.stop(id);
        self.ids.release(id);
        true
    }

    /// Evict every tag matching `prefix` at segment boundaries (the tag is
    /// either exactly `prefix` or a descendant under it).
    /// Returns the number of tags removed; sinks for each evicted ID are
    /// stopped alongside.
    pub fn evict_prefix(&mut self, prefix: &str) -> usize {
        let tags: Vec<String> = self
            .by_name
            .keys()
            .filter(|k| bmc_wasm_protocol::tag_matches_prefix(k, prefix))
            .cloned()
            .collect();
        let mut n = 0;
        for tag in tags {
            if self.evict(&tag) {
                n += 1;
            }
        }
        n
    }

    /// Push a freshly-started playback sink onto the bucket for `id`,
    /// pruning any sinks that have already drained. No-op when the
    /// `audio` feature is disabled.
    #[cfg(feature = "audio")]
    pub fn push_sink(&mut self, id: AudioId, sink: rodio::Sink) {
        let bucket = self.sinks.entry(id).or_default();
        bucket.retain(|s| !s.empty());
        bucket.push(sink);
    }

    /// Stop and drop every sink registered for `id`.
    /// Only defined when the `audio` feature is enabled; with the feature off,
    /// no sink storage exists so the call site must gate the invocation too.
    #[cfg(feature = "audio")]
    pub fn stop(&mut self, id: AudioId) {
        if let Some(bucket) = self.sinks.remove(&id) {
            for sink in bucket {
                sink.stop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(payload: &[u8]) -> Arc<[u8]> {
        Arc::from(payload.to_vec())
    }

    fn register(registry: &mut AudioRegistry, tag: &str) -> AudioId {
        registry
            .register(tag.into(), data(b"x"), 0)
            .expect("BUG: test audio ID space must not be exhausted")
    }

    #[test]
    fn register_is_idempotent_on_tag() {
        let mut reg = AudioRegistry::new();
        let id1 = reg
            .register("ping".into(), data(b"first"), 100)
            .expect("BUG: test audio ID space must not be exhausted");
        let id2 = reg
            .register("ping".into(), data(b"second"), 200)
            .expect("BUG: test audio ID space must not be exhausted");
        assert_eq!(id1, id2);
        // The original sample wins — the second call returns the cached ID
        // without overwriting.
        let sample = reg.get(id1).expect("BUG: sample missing after register");
        assert_eq!(&*sample.data, b"first");
        assert_eq!(sample.duration_ms, 100);
    }

    #[test]
    fn evict_removes_tag_and_sample() {
        let mut reg = AudioRegistry::new();
        let id = register(&mut reg, "ping");
        assert!(reg.get(id).is_some());

        assert!(reg.evict("ping"));
        assert!(reg.get(id).is_none());
        assert!(reg.get_by_tag("ping").is_none());
        // Idempotent: second evict on the same tag is a no-op.
        assert!(!reg.evict("ping"));
    }

    #[test]
    fn evict_prefix_only_touches_matching_tags() {
        let mut reg = AudioRegistry::new();
        register(&mut reg, "a:1");
        register(&mut reg, "a:2");
        let id_b = register(&mut reg, "b:1");

        assert_eq!(reg.evict_prefix("a"), 2);
        assert!(reg.get(id_b).is_some());
    }

    #[test]
    fn evict_prefix_respects_segment_boundaries() {
        let mut reg = AudioRegistry::new();
        let id_foo = register(&mut reg, "foo");
        let id_foobar = register(&mut reg, "foobar");
        let id_foo_child = register(&mut reg, "foo:child");

        assert_eq!(reg.evict_prefix("foo"), 2);
        assert!(reg.get(id_foo).is_none());
        assert!(reg.get(id_foo_child).is_none());
        assert!(reg.get(id_foobar).is_some());
    }

    #[test]
    fn reused_id_resolves_to_the_replacement_sample() {
        let mut reg = AudioRegistry::new();
        let old_id = register(&mut reg, "old");
        assert!(reg.evict("old"));
        let replacement_id = register(&mut reg, "replacement");

        assert_eq!(old_id, replacement_id);
        assert_eq!(
            reg.get(replacement_id)
                .expect("BUG: replacement sample must be registered")
                .name,
            "replacement"
        );
    }

    #[test]
    fn register_returns_none_when_ids_are_exhausted() {
        let mut reg = AudioRegistry::with_id_cap(2);

        assert!(reg.register("first".into(), data(b"x"), 0).is_some());
        assert!(reg.register("second".into(), data(b"x"), 0).is_none());
    }
}
