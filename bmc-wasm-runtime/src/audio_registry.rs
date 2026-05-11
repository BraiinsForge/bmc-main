// Copyright (C) 2026  Braiins Systems s.r.o.

//! Tag-keyed audio sample registry with playback-sink lifecycle.
//!
//! Mirrors the bitmap / icon / mesh registries on the renderer side:
//! registrations are deduped by tag, look up by ID is O(1), and eviction
//! frees both the encoded sample data and any active `rodio::Sink`s for
//! that ID. ID slots are not recycled — re-registering a tag after
//! eviction allocates a fresh `AudioId`.
//!
//! The active-sink half is gated on `feature = "audio"`. With the feature
//! off, the registry still tracks samples (so callers can record fixture
//! events without an audio backend) but `push_sink` / `stop` are no-ops.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use bmc_wasm_protocol::AudioId;

use crate::host_api::AudioSample;

#[derive(Default)]
pub struct AudioRegistry {
    samples: HashMap<AudioId, AudioSample>,
    by_name: HashMap<String, AudioId>,
    next_id: u16,
    /// Active playback sinks keyed by sample ID.
    /// A single sample may have several overlapping plays; sinks are pruned lazily by `push_sink`.
    #[cfg(feature = "audio")]
    sinks: HashMap<AudioId, Vec<rodio::Sink>>,
}

impl fmt::Debug for AudioRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioRegistry")
            .field("count", &self.samples.len())
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl AudioRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: HashMap::new(),
            by_name: HashMap::new(),
            next_id: 1,
            #[cfg(feature = "audio")]
            sinks: HashMap::new(),
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
    pub fn register(&mut self, tag: String, data: Arc<[u8]>, duration_ms: u32) -> AudioId {
        if let Some(&id) = self.by_name.get(&tag) {
            return id;
        }
        let id = AudioId::alloc(&mut self.next_id);
        self.samples.insert(
            id,
            AudioSample {
                data,
                name: tag.clone(),
                duration_ms,
            },
        );
        self.by_name.insert(tag, id);
        id
    }

    /// Evict a single tag: drop the sample, the `tag → AudioId` mapping,
    /// and stop any active playback sinks for that ID.
    /// Returns `true` if a tag was found and evicted.
    pub fn evict(&mut self, tag: &str) -> bool {
        let Some(id) = self.by_name.remove(tag) else {
            return false;
        };
        self.samples.remove(&id);
        self.stop(id);
        true
    }

    /// Evict every tag whose key starts with `prefix`.
    /// Returns the number of tags removed; sinks for each evicted ID are stopped alongside.
    pub fn evict_prefix(&mut self, prefix: &str) -> usize {
        let tags: Vec<String> = self
            .by_name
            .keys()
            .filter(|k| k.starts_with(prefix))
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

    /// Stop and drop every sink registered for `id`. No-op when the
    /// `audio` feature is disabled (no sink storage exists in that build).
    pub fn stop(&mut self, id: AudioId) {
        #[cfg(feature = "audio")]
        if let Some(bucket) = self.sinks.remove(&id) {
            for sink in bucket {
                sink.stop();
            }
        }
        #[cfg(not(feature = "audio"))]
        let _ = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(payload: &[u8]) -> Arc<[u8]> {
        Arc::from(payload.to_vec())
    }

    #[test]
    fn register_is_idempotent_on_tag() {
        let mut reg = AudioRegistry::new();
        let id1 = reg.register("ping".into(), data(b"first"), 100);
        let id2 = reg.register("ping".into(), data(b"second"), 200);
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
        let id = reg.register("ping".into(), data(b"x"), 0);
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
        let _ = reg.register("a::1".into(), data(b"x"), 0);
        let _ = reg.register("a::2".into(), data(b"x"), 0);
        let id_b = reg.register("b::1".into(), data(b"x"), 0);

        assert_eq!(reg.evict_prefix("a::"), 2);
        assert!(reg.get(id_b).is_some());
    }

    #[test]
    fn register_after_evict_uses_fresh_id() {
        let mut reg = AudioRegistry::new();
        let id1 = reg.register("ephemeral".into(), data(b"x"), 0);
        assert!(reg.evict("ephemeral"));
        let id2 = reg.register("ephemeral".into(), data(b"x"), 0);
        // IDs are not recycled — eviction frees resources, not slots.
        assert_ne!(id1, id2);
    }
}
