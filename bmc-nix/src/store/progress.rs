// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;

use cognos::internal::json::{Actions, Activities, Id, ResultType};

/// Download status for a single active file transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadStatus {
    pub id: Id,
    pub store_path: Option<String>,
    pub source: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub remaining_bytes: Option<u64>,
}

/// Snapshot of all active and aggregated download progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadSnapshot {
    pub active: Vec<DownloadStatus>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub remaining_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
struct SubstituteActivity {
    store_path: Option<String>,
    source: Option<String>,
}

#[derive(Debug, Clone)]
struct FileTransferActivity {
    store_path: Option<String>,
    source: Option<String>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AggregateTotalBytes {
    Known(u64),
    Unknown,
}

impl Default for AggregateTotalBytes {
    fn default() -> Self {
        Self::Known(0)
    }
}

/// Tracks download progress by parsing `nix --log-format internal-json` output.
#[derive(Debug, Default)]
pub struct DownloadStatusTracker {
    substitutes: BTreeMap<Id, SubstituteActivity>,
    transfers: BTreeMap<Id, FileTransferActivity>,
    finished_downloaded_bytes: u64,
    finished_total_bytes: AggregateTotalBytes,
}

fn remaining(downloaded: u64, total: Option<u64>) -> Option<u64> {
    total.map(|total| total.saturating_sub(downloaded))
}

impl DownloadStatusTracker {
    /// Returns the current snapshot of all active and aggregated download progress.
    #[must_use]
    pub fn snapshot(&self) -> DownloadSnapshot {
        let active: Vec<DownloadStatus> = self
            .transfers
            .iter()
            .map(|(&id, t)| DownloadStatus {
                id,
                store_path: t.store_path.clone(),
                source: t.source.clone(),
                downloaded_bytes: t.downloaded_bytes,
                total_bytes: t.total_bytes,
                remaining_bytes: remaining(t.downloaded_bytes, t.total_bytes),
            })
            .collect();

        let active_downloaded: u64 = active.iter().map(|s| s.downloaded_bytes).sum();
        let downloaded_bytes = self.finished_downloaded_bytes + active_downloaded;

        let total_bytes = match self.finished_total_bytes {
            AggregateTotalBytes::Unknown => None,
            AggregateTotalBytes::Known(finished_total) => {
                let all_known = active.iter().all(|s| s.total_bytes.is_some());
                if all_known {
                    let active_total: u64 = active.iter().filter_map(|s| s.total_bytes).sum();
                    Some(finished_total + active_total)
                } else {
                    None
                }
            }
        };

        let remaining_bytes = remaining(downloaded_bytes, total_bytes);

        DownloadSnapshot {
            active,
            downloaded_bytes,
            total_bytes,
            remaining_bytes,
        }
    }

    /// Parse a single line of nix `--log-format internal-json` output and
    /// update internal state. Returns a snapshot when byte-level progress
    /// changed, `None` otherwise.
    pub fn ingest_line(&mut self, line: &str) -> Option<DownloadSnapshot> {
        let action = cognos::internal::json::parse_line(line)?;

        match action {
            Actions::Start {
                id,
                activity: Activities::Substitute,
                fields,
                ..
            } => {
                let store_path = fields.first().and_then(|v| v.as_str()).map(str::to_owned);
                let source = fields.get(1).and_then(|v| v.as_str()).map(str::to_owned);
                self.substitutes
                    .insert(id, SubstituteActivity { store_path, source });
                None
            }

            Actions::Start {
                id,
                parent,
                activity: Activities::FileTransfer,
                fields,
                ..
            } => {
                let transfer_source = fields.first().and_then(|v| v.as_str()).map(str::to_owned);
                let parent_sub = self.substitutes.get(&parent);
                let source = transfer_source.or_else(|| parent_sub.and_then(|s| s.source.clone()));
                let store_path = parent_sub.and_then(|s| s.store_path.clone());
                self.transfers.insert(
                    id,
                    FileTransferActivity {
                        store_path,
                        source,
                        downloaded_bytes: 0,
                        total_bytes: None,
                    },
                );
                Some(self.snapshot())
            }

            Actions::Result {
                id,
                result_type: ResultType::Progress,
                fields,
                ..
            } => {
                let transfer = self.transfers.get_mut(&id)?;
                let downloaded = fields.first().and_then(serde_json::Value::as_u64)?;
                let total = fields.get(1).and_then(serde_json::Value::as_u64);
                // A retry restarts the transfer within the same activity and
                // resets its counter; keep the high-water mark so aggregate
                // progress never runs backwards.
                transfer.downloaded_bytes = transfer.downloaded_bytes.max(downloaded);
                transfer.total_bytes = total.or(transfer.total_bytes);
                Some(self.snapshot())
            }

            Actions::Stop { id } => {
                if let Some(transfer) = self.transfers.remove(&id) {
                    self.finished_downloaded_bytes += transfer.downloaded_bytes;
                    if let Some(total) = transfer.total_bytes {
                        if let AggregateTotalBytes::Known(sum) = self.finished_total_bytes {
                            self.finished_total_bytes = AggregateTotalBytes::Known(sum + total);
                        }
                    } else {
                        self.finished_total_bytes = AggregateTotalBytes::Unknown;
                    }
                    Some(self.snapshot())
                } else {
                    // A substitute stop changes no bytes and no active
                    // transfers, so a snapshot here would duplicate the
                    // previous one. Drop the tracked substitute and emit
                    // nothing.
                    self.substitutes.remove(&id);
                    None
                }
            }

            Actions::Start { .. } | Actions::Message { .. } | Actions::Result { .. } => None,
        }
    }
}

#[cfg(test)]
mod cognos_api_tests {
    use cognos::internal::json::{Actions, Activities, Id, ResultType, parse_line};

    #[test]
    fn cognos_internal_json_api_matches_expected_paths() {
        let _: Id = 42;
        let start = parse_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a"]}"#,
        )
        .expect("BUG: start line should parse");
        assert!(matches!(
            start,
            Actions::Start {
                activity: Activities::Substitute,
                ..
            }
        ));

        let transfer = parse_line(
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
        )
        .expect("BUG: file transfer line should parse");
        assert!(matches!(
            transfer,
            Actions::Start {
                parent: 42,
                activity: Activities::FileTransfer,
                ..
            }
        ));

        let progress =
            parse_line(r#"@nix {"action":"result","id":43,"type":105,"fields":[300,1000,1,0]}"#)
                .expect("BUG: progress line should parse");
        assert!(matches!(
            progress,
            Actions::Result {
                result_type: ResultType::Progress,
                ..
            }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_ignores_non_internal_json_lines() {
        let mut tracker = DownloadStatusTracker::default();

        assert!(tracker.ingest_line("copying path").is_none());
        assert!(tracker.snapshot().active.is_empty());
        assert_eq!(tracker.snapshot().downloaded_bytes, 0);
        assert_eq!(tracker.snapshot().total_bytes, Some(0));
        assert_eq!(tracker.snapshot().remaining_bytes, Some(0));
    }

    #[test]
    fn tracker_associates_file_transfer_with_parent_substitute_path() {
        let mut tracker = DownloadStatusTracker::default();
        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );

        let snapshot = tracker
            .ingest_line(
                r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
            )
            .expect("BUG: file transfer start should produce a snapshot");

        assert_eq!(snapshot.active.len(), 1);
        assert_eq!(snapshot.active[0].id, 43);
        assert_eq!(
            snapshot.active[0].store_path.as_deref(),
            Some("/nix/store/a")
        );
        assert_eq!(
            snapshot.active[0].source.as_deref(),
            Some("https://cache/nar/a")
        );
        assert_eq!(snapshot.active[0].downloaded_bytes, 0);
        assert_eq!(snapshot.active[0].total_bytes, None);
        assert_eq!(snapshot.active[0].remaining_bytes, None);
    }

    #[test]
    fn tracker_updates_byte_totals_from_file_transfer_progress() {
        let mut tracker = DownloadStatusTracker::default();
        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
        );

        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"result","id":43,"type":105,"fields":[300,1000,1,0]}"#)
            .expect("BUG: file transfer progress should update byte totals");

        assert_eq!(snapshot.active[0].downloaded_bytes, 300);
        assert_eq!(snapshot.active[0].total_bytes, Some(1000));
        assert_eq!(snapshot.active[0].remaining_bytes, Some(700));
        assert_eq!(snapshot.downloaded_bytes, 300);
        assert_eq!(snapshot.total_bytes, Some(1000));
        assert_eq!(snapshot.remaining_bytes, Some(700));
    }

    #[test]
    fn tracker_accumulates_finished_transfer_bytes() {
        let mut tracker = DownloadStatusTracker::default();
        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
        );
        tracker
            .ingest_line(r#"@nix {"action":"result","id":43,"type":105,"fields":[300,1000,1,0]}"#);

        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"stop","id":43}"#)
            .expect("BUG: file transfer stop should produce a snapshot");

        assert!(snapshot.active.is_empty());
        assert_eq!(snapshot.downloaded_bytes, 300);
        assert_eq!(snapshot.total_bytes, Some(1000));
        assert_eq!(snapshot.remaining_bytes, Some(700));
    }

    #[test]
    fn tracker_ignores_substitute_count_progress_for_bytes() {
        let mut tracker = DownloadStatusTracker::default();
        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );

        assert!(
            tracker
                .ingest_line(r#"@nix {"action":"result","id":42,"type":105,"fields":[1,2,0,0]}"#)
                .is_none()
        );
        assert!(tracker.snapshot().active.is_empty());
        assert_eq!(tracker.snapshot().downloaded_bytes, 0);
        assert_eq!(tracker.snapshot().total_bytes, Some(0));
        assert_eq!(tracker.snapshot().remaining_bytes, Some(0));
    }

    #[test]
    fn tracker_handles_unknown_total_bytes() {
        let mut tracker = DownloadStatusTracker::default();
        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
        );

        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"result","id":43,"type":105,"fields":[300,null,1,0]}"#)
            .expect("BUG: file transfer progress with unknown total should produce a snapshot");

        assert_eq!(snapshot.active[0].downloaded_bytes, 300);
        assert_eq!(snapshot.active[0].total_bytes, None);
        assert_eq!(snapshot.active[0].remaining_bytes, None);
        assert_eq!(snapshot.downloaded_bytes, 300);
        assert_eq!(snapshot.total_bytes, None);
        assert_eq!(snapshot.remaining_bytes, None);
    }

    #[test]
    fn tracker_unknown_total_stays_unknown_after_later_known_transfer() {
        let mut tracker = DownloadStatusTracker::default();

        // First transfer: no known total → drives finished_total_bytes to Unknown.
        tracker.ingest_line(
            r#"@nix {"action":"start","id":10,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":11,"level":3,"parent":10,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
        );
        tracker
            .ingest_line(r#"@nix {"action":"result","id":11,"type":105,"fields":[200,null,1,0]}"#);
        tracker.ingest_line(r#"@nix {"action":"stop","id":11}"#);
        tracker.ingest_line(r#"@nix {"action":"stop","id":10}"#);

        // Second transfer: known total. Stopping it must NOT flip Unknown back to Known.
        tracker.ingest_line(
            r#"@nix {"action":"start","id":20,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/b","https://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":21,"level":3,"parent":20,"text":"","type":101,"fields":["https://cache/nar/b"]}"#,
        );
        tracker
            .ingest_line(r#"@nix {"action":"result","id":21,"type":105,"fields":[500,1000,1,0]}"#);
        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"stop","id":21}"#)
            .expect("BUG: file transfer stop should produce a snapshot");

        assert_eq!(
            snapshot.total_bytes, None,
            "Unknown total from first transfer must not be overridden by later Known transfer"
        );
    }

    #[test]
    fn tracker_holds_high_water_mark_across_transfer_retry() {
        let mut tracker = DownloadStatusTracker::default();

        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","http://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["http://cache/nar/a"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"result","id":43,"type":105,"fields":[8000000,31500000,1,0]}"#,
        );

        // The transfer fails and nix retries within the same activity,
        // resetting the counter to zero.
        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"result","id":43,"type":105,"fields":[0,null,0,0]}"#)
            .expect("BUG: file transfer progress should produce a snapshot");

        assert_eq!(snapshot.downloaded_bytes, 8_000_000);
        assert_eq!(snapshot.total_bytes, Some(31_500_000));
    }

    #[test]
    fn tracker_substitute_stop_emits_no_snapshot() {
        let mut tracker = DownloadStatusTracker::default();
        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );

        assert!(
            tracker
                .ingest_line(r#"@nix {"action":"stop","id":42}"#)
                .is_none(),
            "a substitute stop changes nothing and must not emit a snapshot"
        );
    }

    #[test]
    fn tracker_does_not_report_substitute_start_as_active_download() {
        let mut tracker = DownloadStatusTracker::default();

        assert!(
            tracker
                .ingest_line(
                    r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
                )
                .is_none()
        );
        assert!(tracker.snapshot().active.is_empty());
        assert_eq!(tracker.snapshot().downloaded_bytes, 0);
        assert_eq!(tracker.snapshot().total_bytes, Some(0));
        assert_eq!(tracker.snapshot().remaining_bytes, Some(0));
    }
}
