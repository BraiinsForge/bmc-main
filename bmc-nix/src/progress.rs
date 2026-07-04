// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared `@bmc {json}` progress format: the owned, round-trippable event
//! schema plus a line parser that drives an [`UpgradeProgress`] sink. The
//! CLI's progress emitter serializes these types; `bmc` parses the same
//! lines off the `sysupgrade` child's stderr during a firmware upgrade.

use serde::{Deserialize, Serialize};

use crate::gc::CollectGarbagePhase;
use crate::store::progress::DownloadSnapshot;
use crate::upgrade::{UpgradePhase, UpgradeProgress};

/// Prefix every progress line carries.
pub const BMC_PREFIX: &str = "@bmc ";

/// A single active transfer within a [`ProgressEvent::Download`]. Mirrors the
/// live `DownloadStatus` minus its runtime `id` and per-transfer
/// `remaining_bytes` (the schema carries `remaining_bytes` only at top level).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveDownload {
    pub store_path: Option<String>,
    pub source: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// One `@bmc` progress event. Internally tagged so `type` leads and fields
/// follow in declaration order, matching the wire schema byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    Phase {
        phase: String,
    },
    RealizationStarted {
        total_paths: usize,
    },
    Download {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        remaining_bytes: Option<u64>,
        active: Vec<ActiveDownload>,
    },
    RealizationFinished,
    GcPhase {
        phase: String,
    },
    GcProgress {
        deleted_paths: usize,
    },
    GcFinished {
        deleted_paths: usize,
        freed_bytes: Option<u64>,
    },
}

impl ProgressEvent {
    /// Render the `@bmc {json}` line for this event.
    #[must_use]
    pub fn to_bmc_line(&self) -> String {
        format!(
            "{BMC_PREFIX}{}",
            serde_json::to_string(self).expect("BUG: progress event must serialize")
        )
    }
}

/// Parse one line. Returns `None` for lines without the `@bmc ` prefix or
/// whose payload is not a valid event — a firmware upgrade interleaves these
/// events with `fwtool`/validation noise on the same stderr.
#[must_use]
pub fn parse_line(line: &str) -> Option<ProgressEvent> {
    let json = line.trim_end().strip_prefix(BMC_PREFIX)?;
    serde_json::from_str(json).ok()
}

/// Parse `line` and drive `sink`. Returns `true` when a `@bmc` event was
/// consumed. `Phase`/`GcPhase` with an unknown phase name are dropped.
pub fn feed_line(line: &str, sink: &dyn UpgradeProgress) -> bool {
    let Some(event) = parse_line(line) else {
        return false;
    };
    drive(&event, sink);
    true
}

fn drive(event: &ProgressEvent, sink: &dyn UpgradeProgress) {
    match event {
        ProgressEvent::Phase { phase } => {
            if let Ok(phase) = UpgradePhase::try_from(phase.as_str()) {
                sink.on_phase(phase);
            }
        }
        ProgressEvent::GcPhase { phase } => {
            if let Ok(phase) = CollectGarbagePhase::try_from(phase.as_str()) {
                sink.on_phase(UpgradePhase::CollectingGarbage(phase));
            }
        }
        ProgressEvent::RealizationStarted { total_paths } => {
            sink.on_realization_started(*total_paths);
        }
        ProgressEvent::RealizationFinished => sink.on_realization_finished(),
        ProgressEvent::Download {
            downloaded_bytes,
            total_bytes,
            remaining_bytes,
            ..
        } => {
            // The sink needs only the aggregate; per-transfer detail (and the
            // runtime `id`) are intentionally not reconstructed.
            sink.on_download_status(&DownloadSnapshot {
                active: Vec::new(),
                downloaded_bytes: *downloaded_bytes,
                total_bytes: *total_bytes,
                remaining_bytes: *remaining_bytes,
            });
        }
        ProgressEvent::GcProgress { deleted_paths } => sink.on_gc_deleted(*deleted_paths),
        ProgressEvent::GcFinished {
            deleted_paths,
            freed_bytes,
        } => sink.on_gc_finished(*deleted_paths, *freed_bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn sample_events() -> Vec<ProgressEvent> {
        vec![
            ProgressEvent::Phase {
                phase: "realizing".to_owned(),
            },
            ProgressEvent::RealizationStarted { total_paths: 12 },
            ProgressEvent::Download {
                downloaded_bytes: 1024,
                total_bytes: Some(4096),
                remaining_bytes: Some(3072),
                active: vec![ActiveDownload {
                    store_path: Some("/nix/store/x".to_owned()),
                    source: Some("https://cache".to_owned()),
                    downloaded_bytes: 1024,
                    total_bytes: Some(4096),
                }],
            },
            ProgressEvent::RealizationFinished,
            ProgressEvent::GcPhase {
                phase: "finding_roots".to_owned(),
            },
            ProgressEvent::GcProgress { deleted_paths: 300 },
            ProgressEvent::GcFinished {
                deleted_paths: 300,
                freed_bytes: Some(9999),
            },
        ]
    }

    #[test]
    fn event_roundtrips_through_bmc_line() {
        for event in sample_events() {
            let line = event.to_bmc_line();
            assert!(line.starts_with("@bmc {"));
            assert_eq!(parse_line(&line), Some(event));
        }
    }

    #[test]
    fn parser_skips_non_bmc_and_malformed() {
        assert_eq!(parse_line("fwtool: validating image"), None);
        assert_eq!(parse_line("@bmc not-json"), None);
        assert_eq!(parse_line("@bmc {\"type\":\"unknown\"}"), None);
    }

    type DownloadSample = (u64, Option<u64>, Option<u64>);

    #[derive(Default)]
    struct Recorder {
        phases: Mutex<Vec<UpgradePhase>>,
        downloads: Mutex<Vec<DownloadSample>>,
    }
    impl UpgradeProgress for Recorder {
        fn on_phase(&self, phase: UpgradePhase) {
            self.phases.lock().expect("BUG: lock").push(phase);
        }
        fn on_realization_started(&self, _: usize) {}
        fn on_realization_finished(&self) {}
        fn on_download_status(&self, snapshot: &DownloadSnapshot) {
            self.downloads.lock().expect("BUG: lock").push((
                snapshot.downloaded_bytes,
                snapshot.total_bytes,
                snapshot.remaining_bytes,
            ));
        }
        fn on_gc_deleted(&self, _: usize) {}
        fn on_gc_finished(&self, _: usize, _: Option<u64>) {}
    }

    #[test]
    fn feed_line_delivers_download_aggregate_to_the_sink() {
        // Locks the field mapping: swapping `total_bytes` and
        // `remaining_bytes` (or dropping one) must fail here — the mock
        // emitter and `bmc` both trust this exact wire contract.
        let sink = Recorder::default();
        assert!(feed_line(
            "@bmc {\"type\":\"download\",\"downloaded_bytes\":1024,\
             \"total_bytes\":4096,\"remaining_bytes\":3072,\"active\":[]}",
            &sink
        ));
        assert_eq!(
            *sink.downloads.lock().expect("BUG: lock"),
            vec![(1024, Some(4096), Some(3072))]
        );
    }

    #[test]
    fn parser_tolerates_unknown_fields_from_newer_emitters() {
        // A newer emitter may add fields; an older parser must keep
        // consuming the events it understands.
        let event = parse_line(
            "@bmc {\"type\":\"download\",\"downloaded_bytes\":1,\
             \"total_bytes\":null,\"remaining_bytes\":null,\"active\":[],\
             \"added_in_a_future_version\":true}",
        )
        .expect("BUG: unknown extra fields must not break parsing");
        assert!(matches!(
            event,
            ProgressEvent::Download {
                downloaded_bytes: 1,
                ..
            }
        ));
    }

    #[test]
    fn feed_line_maps_gc_phase_to_collecting_garbage() {
        let sink = Recorder::default();
        assert!(feed_line(
            "@bmc {\"type\":\"gc_phase\",\"phase\":\"finding_roots\"}",
            &sink
        ));
        assert!(!feed_line("noise", &sink));
        assert_eq!(
            *sink.phases.lock().expect("BUG: lock"),
            vec![UpgradePhase::CollectingGarbage(
                CollectGarbagePhase::FindingRoots
            )]
        );
    }
}
