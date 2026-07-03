// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;

use cognos::internal::json::{Actions, Activities, Id, ResultType, Verbosity};

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
    /// Whole-closure download total nix announces via `SetExpected` once it
    /// has queried the missing paths' narinfos, before the bulk bytes flow.
    /// When present it is the authoritative denominator, since it counts every
    /// missing path rather than only the transfers that have already started.
    expected_download_bytes: Option<u64>,
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

        // A zero expected total means nix had no size information (a cache
        // whose narinfos omit FileSize), not an empty download — fall back
        // to the per-transfer totals in that case.
        let total_bytes = self
            .expected_download_bytes
            .filter(|&expected| expected > 0)
            .or(match self.finished_total_bytes {
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
            });

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

            Actions::Result {
                result_type: ResultType::SetExpected,
                fields,
                ..
            } => {
                // nix reports per-activity-type expected totals; for the
                // FileTransfer activity the expected value is download bytes
                // (other activity types carry counts, not bytes).
                let activity = fields.first().and_then(serde_json::Value::as_u64);
                if activity == Some(Activities::FileTransfer as u64)
                    && let Some(expected) = fields.get(1).and_then(serde_json::Value::as_u64)
                {
                    self.expected_download_bytes = Some(expected);
                    return Some(self.snapshot());
                }
                None
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

/// Collects human-readable error diagnostics from nix `--log-format
/// internal-json` output.
///
/// Under `internal-json` every stderr line is a JSON object, so a failed
/// realization can no longer surface nix's own error text directly. This
/// collector keeps the error-level messages (e.g. an unreachable
/// substituter, a 404 NAR, a signature mismatch) so the failure can be
/// reported in the terms a user understands rather than as raw JSON.
#[derive(Debug, Default)]
pub struct RealizeDiagnostics {
    errors: Vec<String>,
    omitted: usize,
}

/// Upper bound on retained distinct error messages. A failing
/// `nix-store --realise` can emit many distinct error lines; the consumer
/// joins all retained messages, so an unbounded set would grow the failure
/// report without bound. The earliest messages are the most useful, so the
/// cap keeps the head and counts the rest as omitted.
const MAX_DIAGNOSTIC_MESSAGES: usize = 50;

impl RealizeDiagnostics {
    /// Parse one line of nix internal-json output and record it when it is an
    /// error-level diagnostic message. Non-error messages and non-message
    /// actions are ignored. Once [`MAX_DIAGNOSTIC_MESSAGES`] distinct
    /// messages are retained, further distinct messages are counted but not
    /// stored.
    pub fn ingest_line(&mut self, line: &str) {
        let Some(Actions::Message {
            level: Verbosity::Error,
            msg,
            raw_msg,
            ..
        }) = cognos::internal::json::parse_line(line)
        else {
            return;
        };

        // Prefer `raw_msg` (no error trace) but strip either variant: Lix
        // pre-strips ANSI from `raw_msg`, nix does not.
        let text = strip_ansi(raw_msg.as_deref().unwrap_or(&msg));
        let text = text.trim();
        if text.is_empty() || self.errors.iter().any(|e| e == text) {
            return;
        }
        if self.errors.len() >= MAX_DIAGNOSTIC_MESSAGES {
            self.omitted += 1;
            return;
        }
        self.errors.push(text.to_owned());
    }

    /// The collected error messages, in emission order, deduplicated.
    #[must_use]
    pub fn messages(&self) -> &[String] {
        &self.errors
    }

    /// Whether any error-level message was collected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Whether the retained set was capped and further distinct messages
    /// were dropped.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.omitted > 0
    }

    /// Consume the collector, returning the collected error messages. When
    /// the retained set was capped, a final note records how many further
    /// distinct messages were omitted.
    #[must_use]
    pub fn into_messages(mut self) -> Vec<String> {
        if self.omitted > 0 {
            self.errors.push(format!(
                "… ({} additional error messages omitted)",
                self.omitted
            ));
        }
        self.errors
    }
}

/// Section of `nix-store --realise --dry-run` output currently being read.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum DryRunSection {
    #[default]
    None,
    /// Paths listed under "… will be fetched (… download, … unpacked):".
    Fetch,
    /// Paths listed under "don't know how to build these paths:" or
    /// "… will be built:". The device never builds locally, so both mean
    /// the realization cannot be satisfied from substituters.
    Unsubstitutable,
}

/// Parses the output of `nix-store --realise --dry-run` from nix
/// `internal-json` message lines.
///
/// Byte totals are reconstructed from nix's human-readable summary line,
/// which prints one decimal in binary units — accurate to ~0.05 of the
/// printed unit (e.g. ±~50 KiB for a MiB-scale download).
#[derive(Debug, Default)]
pub struct DryRunEstimate {
    section: DryRunSection,
    fetch_path_count: usize,
    download_bytes: u64,
    unpacked_bytes: u64,
    unparsed_summary: Option<String>,
    unsubstitutable: Vec<String>,
    unsubstitutable_omitted: usize,
}

impl DryRunEstimate {
    /// Parse one line of nix internal-json output, recording the fetch
    /// summary sizes, fetched-path count and unsubstitutable paths. Lines
    /// that are not dry-run messages are ignored.
    pub fn ingest_line(&mut self, line: &str) {
        let Some(Actions::Message { msg, raw_msg, .. }) = cognos::internal::json::parse_line(line)
        else {
            return;
        };
        // Prefer `raw_msg` (no error trace) but strip either variant: Lix
        // pre-strips ANSI from `raw_msg`, nix does not.
        let text = strip_ansi(raw_msg.as_deref().unwrap_or(&msg));

        // Section body lines are indented store paths.
        if text.starts_with("  ") && text.trim_start().starts_with('/') {
            match self.section {
                DryRunSection::Fetch => self.fetch_path_count += 1,
                DryRunSection::Unsubstitutable => {
                    if self.unsubstitutable.len() >= MAX_DIAGNOSTIC_MESSAGES {
                        self.unsubstitutable_omitted += 1;
                    } else {
                        self.unsubstitutable.push(text.trim().to_owned());
                    }
                }
                DryRunSection::None => {}
            }
            return;
        }

        let trimmed = text.trim();
        if trimmed.starts_with("this path will be fetched")
            || (trimmed.starts_with("these ") && trimmed.contains("paths will be fetched"))
        {
            self.section = DryRunSection::Fetch;
            match parse_fetch_summary(trimmed) {
                Some((download_bytes, unpacked_bytes)) => {
                    self.download_bytes = download_bytes;
                    self.unpacked_bytes = unpacked_bytes;
                }
                None => self.unparsed_summary = Some(trimmed.to_owned()),
            }
        } else if trimmed == "don't know how to build these paths:"
            || trimmed.ends_with("will be built:")
        {
            self.section = DryRunSection::Unsubstitutable;
        } else {
            self.section = DryRunSection::None;
        }
    }

    /// Number of store paths nix reported it would fetch.
    #[must_use]
    pub fn fetch_path_count(&self) -> usize {
        self.fetch_path_count
    }

    /// Total download size in bytes; zero when nothing would be fetched.
    #[must_use]
    pub fn download_bytes(&self) -> u64 {
        self.download_bytes
    }

    /// Total unpacked (NAR) size in bytes; zero when nothing would be fetched.
    #[must_use]
    pub fn unpacked_bytes(&self) -> u64 {
        self.unpacked_bytes
    }

    /// A fetch-summary line that was recognized but whose sizes could not
    /// be parsed. The caller must treat this as an error rather than
    /// report a zero-byte estimate.
    #[must_use]
    pub fn unparsed_summary(&self) -> Option<&str> {
        self.unparsed_summary.as_deref()
    }

    /// Whether any path cannot be fetched from a substituter.
    #[must_use]
    pub fn has_unsubstitutable(&self) -> bool {
        !self.unsubstitutable.is_empty()
    }

    /// Consume the collector, returning the unsubstitutable paths. When the
    /// retained set was capped, a final note records how many further paths
    /// were omitted.
    #[must_use]
    pub fn into_unsubstitutable(mut self) -> Vec<String> {
        if self.unsubstitutable_omitted > 0 {
            self.unsubstitutable.push(format!(
                "… ({} additional paths omitted)",
                self.unsubstitutable_omitted
            ));
        }
        self.unsubstitutable
    }
}

/// Parse the parenthesized sizes from a dry-run fetch summary line, e.g.
/// `this path will be fetched (57.2 KiB download, 273.1 KiB unpacked):`.
fn parse_fetch_summary(msg: &str) -> Option<(u64, u64)> {
    let (_, rest) = msg.split_once('(')?;
    let (inner, _) = rest.split_once(')')?;
    let (download, unpacked) = inner.split_once(", ")?;
    let download = download.strip_suffix(" download")?;
    let unpacked = unpacked.strip_suffix(" unpacked")?;
    Some((parse_binary_size(download)?, parse_binary_size(unpacked)?))
}

/// Parse a size like `57.2 KiB` into bytes.
fn parse_binary_size(size: &str) -> Option<u64> {
    let (value, unit) = size.split_once(' ')?;
    let value: f64 = value.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let factor: f64 = match unit {
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is finite and non-negative; TiB-scale inputs stay far below u64::MAX"
    )]
    Some((value * factor).round() as u64)
}

/// Remove ANSI CSI escape sequences (color codes and cursor control) from a
/// string.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // ESC: drop a CSI sequence `ESC [ params... final-byte`, where the
        // final byte is in the range `@`..=`~` (0x40..=0x7e).
        if chars.peek() == Some(&'[') {
            chars.next();
            while let Some(&seq) = chars.peek() {
                chars.next();
                if ('@'..='~').contains(&seq) {
                    break;
                }
            }
        }
    }
    out
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
    fn diagnostics_strips_ansi_from_raw_msg() {
        let mut diagnostics = RealizeDiagnostics::default();

        // nix populates raw_msg with the color codes intact (only Lix
        // pre-strips them).
        diagnostics.ingest_line(
            r#"@nix {"action":"msg","level":0,"msg":"error: colored","raw_msg":"error: path '\u001b[35;1m/nix/store/abc-core\u001b[0m' is required"}"#,
        );

        assert_eq!(diagnostics.messages().len(), 1);
        assert_eq!(
            diagnostics.messages()[0],
            "error: path '/nix/store/abc-core' is required"
        );
    }

    #[test]
    fn tracker_uses_set_expected_as_whole_closure_download_total() {
        let mut tracker = DownloadStatusTracker::default();

        // nix announces the closure-wide download total (FileTransfer = 101)
        // once narinfos are queried, before any bytes flow.
        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"result","id":1,"type":106,"fields":[101,57684]}"#)
            .expect("BUG: SetExpected for FileTransfer should produce a snapshot");

        assert_eq!(snapshot.downloaded_bytes, 0);
        assert_eq!(snapshot.total_bytes, Some(57684));
        assert_eq!(snapshot.remaining_bytes, Some(57684));
    }

    #[test]
    fn tracker_ignores_set_expected_for_non_file_transfer_activity() {
        let mut tracker = DownloadStatusTracker::default();

        // CopyPath (100) carries unpacked bytes, not the download denominator.
        assert!(
            tracker
                .ingest_line(r#"@nix {"action":"result","id":1,"type":106,"fields":[100,274640]}"#)
                .is_none()
        );
        // Falls back to the per-transfer accumulation (nothing started yet).
        assert_eq!(tracker.snapshot().total_bytes, Some(0));
    }

    #[test]
    fn tracker_ignores_zero_set_expected_download_total() {
        let mut tracker = DownloadStatusTracker::default();

        // A cache whose narinfos omit FileSize yields a zero closure-wide
        // estimate; zero means "no information", not an empty download.
        tracker.ingest_line(r#"@nix {"action":"result","id":1,"type":106,"fields":[101,0]}"#);

        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
        );
        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"result","id":43,"type":105,"fields":[200,400,1,0]}"#)
            .expect("BUG: file transfer progress should produce a snapshot");

        assert_eq!(snapshot.downloaded_bytes, 200);
        // The denominator comes from the per-transfer totals, not the
        // useless zero estimate.
        assert_eq!(snapshot.total_bytes, Some(400));
        assert_eq!(snapshot.remaining_bytes, Some(200));
    }

    #[test]
    fn tracker_prefers_set_expected_over_started_transfer_accumulation() {
        let mut tracker = DownloadStatusTracker::default();

        // Closure-wide download total known up front: 1000 bytes.
        tracker.ingest_line(r#"@nix {"action":"result","id":1,"type":106,"fields":[101,1000]}"#);

        // Only the first of several paths has begun transferring, reporting its
        // own smaller per-file total.
        tracker.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#,
        );
        tracker.ingest_line(
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#,
        );
        let snapshot = tracker
            .ingest_line(r#"@nix {"action":"result","id":43,"type":105,"fields":[200,400,1,0]}"#)
            .expect("BUG: file transfer progress should produce a snapshot");

        assert_eq!(snapshot.downloaded_bytes, 200);
        // The denominator stays the closure-wide expected total, not the lone
        // started transfer's 400 — this is the whole point of consuming
        // SetExpected instead of accumulating started transfers.
        assert_eq!(snapshot.total_bytes, Some(1000));
        assert_eq!(snapshot.remaining_bytes, Some(800));
    }

    #[test]
    fn diagnostics_collects_error_level_message_with_failing_url() {
        let mut diagnostics = RealizeDiagnostics::default();

        diagnostics.ingest_line(
            r#"@nix {"action":"msg","level":0,"msg":"error: unable to download 'https://cache.example.com/nar/a.nar.xz': Couldn't resolve host name (6)"}"#,
        );

        assert_eq!(diagnostics.messages().len(), 1);
        assert_eq!(
            diagnostics.messages()[0],
            "error: unable to download 'https://cache.example.com/nar/a.nar.xz': Couldn't resolve host name (6)"
        );
    }

    #[test]
    fn diagnostics_ignores_non_error_messages_and_other_actions() {
        let mut diagnostics = RealizeDiagnostics::default();

        diagnostics
            .ingest_line(r#"@nix {"action":"msg","level":1,"msg":"warning: substituter is slow"}"#);
        diagnostics.ingest_line(r#"@nix {"action":"msg","level":3,"msg":"copying path"}"#);
        diagnostics.ingest_line(
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a"]}"#,
        );
        diagnostics.ingest_line(r#"@nix {"action":"stop","id":42}"#);
        diagnostics.ingest_line("not internal-json at all");

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn diagnostics_strips_ansi_color_codes_from_nix_message() {
        let mut diagnostics = RealizeDiagnostics::default();

        // Nix embeds ANSI color codes in `msg` and provides no `raw_msg`.
        // Build the wire form so the escape is the real control byte, JSON
        // escaped exactly as nix would emit it.
        let esc = '\u{1b}';
        let colored = format!("{esc}[31;1merror:{esc}[0m host unreachable");
        let line = format!(
            r#"@nix {{"action":"msg","level":0,"msg":{}}}"#,
            serde_json::to_string(&colored).expect("BUG: serialize msg"),
        );
        diagnostics.ingest_line(&line);

        assert_eq!(
            diagnostics.messages(),
            &["error: host unreachable".to_owned()]
        );
    }

    #[test]
    fn diagnostics_prefers_lix_raw_msg_over_ansi_msg() {
        let mut diagnostics = RealizeDiagnostics::default();

        let esc = '\u{1b}';
        let colored = format!("{esc}[31merror:{esc}[0m boom");
        let line = format!(
            r#"@nix {{"action":"msg","level":0,"msg":{},"raw_msg":"error: boom"}}"#,
            serde_json::to_string(&colored).expect("BUG: serialize msg"),
        );
        diagnostics.ingest_line(&line);

        assert_eq!(diagnostics.messages(), &["error: boom".to_owned()]);
    }

    #[test]
    fn diagnostics_deduplicates_repeated_error_messages() {
        let mut diagnostics = RealizeDiagnostics::default();
        let line = r#"@nix {"action":"msg","level":0,"msg":"error: build failed"}"#;

        diagnostics.ingest_line(line);
        diagnostics.ingest_line(line);

        assert_eq!(diagnostics.messages().len(), 1);
    }

    #[test]
    fn diagnostics_caps_retained_messages_and_flags_truncation() {
        let mut diagnostics = RealizeDiagnostics::default();

        let total = MAX_DIAGNOSTIC_MESSAGES + 7;
        for i in 0..total {
            diagnostics.ingest_line(&format!(
                r#"@nix {{"action":"msg","level":0,"msg":"error: failure number {i}"}}"#
            ));
        }

        assert_eq!(diagnostics.messages().len(), MAX_DIAGNOSTIC_MESSAGES);
        assert!(diagnostics.truncated());

        let messages = diagnostics.into_messages();
        assert_eq!(messages.len(), MAX_DIAGNOSTIC_MESSAGES + 1);
        assert_eq!(
            messages.last().map(String::as_str),
            Some("… (7 additional error messages omitted)")
        );
        assert_eq!(messages[0], "error: failure number 0");
    }

    #[test]
    fn diagnostics_under_cap_appends_no_truncation_note() {
        let mut diagnostics = RealizeDiagnostics::default();

        for i in 0..3 {
            diagnostics.ingest_line(&format!(
                r#"@nix {{"action":"msg","level":0,"msg":"error: failure number {i}"}}"#
            ));
        }

        assert!(!diagnostics.truncated());
        let messages = diagnostics.into_messages();
        assert_eq!(messages.len(), 3);
        assert!(
            !messages.iter().any(|m| m.contains("omitted")),
            "no truncation note must appear under the cap"
        );
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

    // Real transcript captured from `nix-store -r --dry-run --log-format
    // internal-json /nix/store/…-hello-2.12.3` against cache.nixos.org.
    #[test]
    fn dry_run_parses_singular_fetch_summary_from_real_transcript() {
        let mut estimate = DryRunEstimate::default();
        for line in [
            r#"@nix {"action":"start","id":117175297769472,"level":6,"parent":0,"text":"querying info about missing paths","type":0}"#,
            r#"@nix {"action":"result","fields":[600,600,0,0],"id":117175297769474,"type":105}"#,
            r#"@nix {"action":"stop","id":117175297769472}"#,
            r#"@nix {"action":"msg","level":3,"msg":"this path will be fetched (57.2 KiB download, 273.1 KiB unpacked):"}"#,
            r#"@nix {"action":"msg","level":3,"msg":"  /nix/store/a58bx0sw2r7fhk4qyg7wvjdd81zw561h-hello-2.12.3"}"#,
        ] {
            estimate.ingest_line(line);
        }

        assert_eq!(estimate.fetch_path_count(), 1);
        // 57.2 KiB and 273.1 KiB, rounded to whole bytes.
        assert_eq!(estimate.download_bytes(), 58573);
        assert_eq!(estimate.unpacked_bytes(), 279654);
        assert_eq!(estimate.unparsed_summary(), None);
        assert!(!estimate.has_unsubstitutable());
    }

    #[test]
    fn dry_run_parses_plural_summary_and_counts_paths() {
        let mut estimate = DryRunEstimate::default();
        for line in [
            r#"@nix {"action":"msg","level":3,"msg":"these 2 paths will be fetched (12.5 MiB download, 1.2 GiB unpacked):"}"#,
            r#"@nix {"action":"msg","level":3,"msg":"  /nix/store/aaa-pkg-a"}"#,
            r#"@nix {"action":"msg","level":3,"msg":"  /nix/store/bbb-pkg-b"}"#,
        ] {
            estimate.ingest_line(line);
        }

        assert_eq!(estimate.fetch_path_count(), 2);
        assert_eq!(estimate.download_bytes(), 13_107_200);
        assert_eq!(estimate.unpacked_bytes(), 1_288_490_189);
    }

    #[test]
    fn dry_run_nothing_to_fetch_reports_zeros() {
        let mut estimate = DryRunEstimate::default();
        estimate.ingest_line(r#"@nix {"action":"stop","id":1}"#);

        assert_eq!(estimate.fetch_path_count(), 0);
        assert_eq!(estimate.download_bytes(), 0);
        assert_eq!(estimate.unpacked_bytes(), 0);
        assert_eq!(estimate.unparsed_summary(), None);
        assert!(!estimate.has_unsubstitutable());
    }

    // Real transcript: dry run of a path no substituter provides.
    #[test]
    fn dry_run_collects_unsubstitutable_paths_from_real_transcript() {
        let mut estimate = DryRunEstimate::default();
        for line in [
            r#"@nix {"action":"msg","level":3,"msg":"don't know how to build these paths:"}"#,
            r#"@nix {"action":"msg","level":3,"msg":"  /nix/store/zzzbx0sw2r7fhk4qyg7wvjdd81zw561h-hello-2.12.3"}"#,
        ] {
            estimate.ingest_line(line);
        }

        assert!(estimate.has_unsubstitutable());
        assert_eq!(
            estimate.into_unsubstitutable(),
            vec!["/nix/store/zzzbx0sw2r7fhk4qyg7wvjdd81zw561h-hello-2.12.3".to_owned()]
        );
    }

    #[test]
    fn dry_run_treats_will_be_built_as_unsubstitutable() {
        let mut estimate = DryRunEstimate::default();
        for line in [
            r#"@nix {"action":"msg","level":3,"msg":"these 1 derivations will be built:"}"#,
            r#"@nix {"action":"msg","level":3,"msg":"  /nix/store/ccc-pkg.drv"}"#,
        ] {
            estimate.ingest_line(line);
        }

        assert!(estimate.has_unsubstitutable());
    }

    #[test]
    fn dry_run_flags_unparseable_summary_instead_of_reporting_zero() {
        let mut estimate = DryRunEstimate::default();
        estimate.ingest_line(
            r#"@nix {"action":"msg","level":3,"msg":"these 3 paths will be fetched (sizes unknown):"}"#,
        );

        assert_eq!(
            estimate.unparsed_summary(),
            Some("these 3 paths will be fetched (sizes unknown):")
        );
    }

    #[test]
    fn dry_run_strips_ansi_from_summary_line() {
        let mut estimate = DryRunEstimate::default();
        estimate.ingest_line(
            r#"@nix {"action":"msg","level":3,"msg":"\u001b[32mthis path will be fetched (1.0 KiB download, 2.0 KiB unpacked):\u001b[0m"}"#,
        );

        assert_eq!(estimate.download_bytes(), 1024);
        assert_eq!(estimate.unpacked_bytes(), 2048);
    }

    #[test]
    fn parse_binary_size_rejects_unknown_units_and_garbage() {
        assert_eq!(parse_binary_size("1.0 KiB"), Some(1024));
        assert_eq!(parse_binary_size("0.0 KiB"), Some(0));
        assert_eq!(parse_binary_size("1.0 kB"), None);
        assert_eq!(parse_binary_size("KiB"), None);
        assert_eq!(parse_binary_size("-1.0 KiB"), None);
        assert_eq!(parse_binary_size("NaN KiB"), None);
    }
}
