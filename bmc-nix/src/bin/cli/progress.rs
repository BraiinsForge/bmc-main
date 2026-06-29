// Copyright (C) 2026  Braiins Systems s.r.o.

use std::sync::Mutex;

use bmc_nix::gc::{CollectGarbagePhase, CollectGarbageProgress};
use bmc_nix::store::progress::DownloadSnapshot;
use bmc_nix::upgrade::{UpgradePhase, UpgradeProgress};
use serde::Serialize;

use super::LogFormat;

const ONE_MEGABYTE: u64 = 1_000_000;

/// Emit a garbage-collection progress line only after this many additional
/// paths have been deleted, bounding output over a store-wide sweep that can
/// delete tens of thousands of paths.
const GC_PROGRESS_STEP: usize = 100;

/// One progress event the trait methods map onto, rendered by the pure
/// `internal_json_line` / `human_line` functions.
enum ProgressEvent<'a> {
    Phase(UpgradePhase),
    RealizationStarted {
        total_paths: usize,
    },
    Download(&'a DownloadSnapshot),
    RealizationFinished,
    GcPhase(CollectGarbagePhase),
    GcDeleted {
        deleted_paths: usize,
    },
    GcFinished {
        deleted_paths: usize,
        freed_bytes: Option<u64>,
    },
}

/// Throttle cursor for the human format. `last_percent` drives
/// known-total throttling; `last_emitted_bytes` drives the unknown-total
/// ~1 MB step.
#[derive(Default, Clone, Copy, Debug)]
struct HumanThrottle {
    last_percent: Option<u8>,
    last_emitted_bytes: u64,
    gc_count_at_last_emit: usize,
}

/// Render bytes as decimal megabytes with one decimal place.
#[expect(
    clippy::cast_precision_loss,
    reason = "store-download byte counts stay within f64 exact-integer range"
)]
fn format_bytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / ONE_MEGABYTE as f64)
}

/// Serializable mirror of a single active transfer (no `remaining_bytes`:
/// the schema carries it only at the top level).
#[derive(Serialize)]
struct JsonActive<'a> {
    store_path: Option<&'a str>,
    source: Option<&'a str>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

/// Serializable mirror of `ProgressEvent`. Internally tagged so the
/// `type` key is emitted first and struct fields follow in declaration
/// order, matching the documented schema byte-for-byte.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JsonEvent<'a> {
    Phase {
        phase: &'a str,
    },
    RealizationStarted {
        total_paths: usize,
    },
    Download {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        remaining_bytes: Option<u64>,
        active: Vec<JsonActive<'a>>,
    },
    RealizationFinished,
    GcPhase {
        phase: &'a str,
    },
    GcProgress {
        deleted_paths: usize,
    },
    GcFinished {
        deleted_paths: usize,
        freed_bytes: Option<u64>,
    },
}

/// Render one `@bmc {…}` line for the `internal-json` format.
fn internal_json_line(event: &ProgressEvent<'_>) -> String {
    let json = match event {
        ProgressEvent::Phase(phase) => JsonEvent::Phase {
            phase: phase.as_str(),
        },
        ProgressEvent::RealizationStarted { total_paths } => JsonEvent::RealizationStarted {
            total_paths: *total_paths,
        },
        ProgressEvent::RealizationFinished => JsonEvent::RealizationFinished,
        ProgressEvent::Download(snapshot) => JsonEvent::Download {
            downloaded_bytes: snapshot.downloaded_bytes,
            total_bytes: snapshot.total_bytes,
            remaining_bytes: snapshot.remaining_bytes,
            active: snapshot
                .active
                .iter()
                .map(|s| JsonActive {
                    store_path: s.store_path.as_deref(),
                    source: s.source.as_deref(),
                    downloaded_bytes: s.downloaded_bytes,
                    total_bytes: s.total_bytes,
                })
                .collect(),
        },
        ProgressEvent::GcPhase(phase) => JsonEvent::GcPhase {
            phase: phase.as_str(),
        },
        ProgressEvent::GcDeleted { deleted_paths } => JsonEvent::GcProgress {
            deleted_paths: *deleted_paths,
        },
        ProgressEvent::GcFinished {
            deleted_paths,
            freed_bytes,
        } => JsonEvent::GcFinished {
            deleted_paths: *deleted_paths,
            freed_bytes: *freed_bytes,
        },
    };
    format!(
        "@bmc {}",
        serde_json::to_string(&json).expect("BUG: progress event must serialize")
    )
}

/// Render one human line, or `None` when throttled away. Returns the
/// (possibly updated) throttle cursor.
fn human_line(
    event: &ProgressEvent<'_>,
    throttle: HumanThrottle,
) -> (Option<String>, HumanThrottle) {
    match event {
        ProgressEvent::Phase(phase) => (Some(format!("\u{2192} {}", phase.as_str())), throttle),
        ProgressEvent::RealizationStarted { total_paths } => (
            Some(format!("  realizing {total_paths} store paths")),
            throttle,
        ),
        ProgressEvent::RealizationFinished => (None, throttle),
        ProgressEvent::Download(snapshot) => human_download_line(snapshot, throttle),
        ProgressEvent::GcPhase(phase) => {
            let text = match phase {
                CollectGarbagePhase::FindingRoots => "\u{2192} collecting garbage",
                CollectGarbagePhase::DeterminingLiveness => "  determining live/dead paths",
            };
            (Some(text.to_owned()), throttle)
        }
        ProgressEvent::GcDeleted { deleted_paths } => {
            (Some(format!("  deleted {deleted_paths} paths")), throttle)
        }
        ProgressEvent::GcFinished {
            deleted_paths,
            freed_bytes,
        } => {
            let freed = freed_bytes
                .map(|b| format!(", {} MB freed", format_bytes(b)))
                .unwrap_or_default();
            (
                Some(format!(
                    "  collected garbage: {deleted_paths} paths deleted{freed}"
                )),
                throttle,
            )
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::single_match_else,
    reason = "percent is clamped to 0..=100 and byte counts stay within f64 range; \
              match arms both contain early returns, if-let-else would require nesting"
)]
fn human_download_line(
    snapshot: &DownloadSnapshot,
    mut throttle: HumanThrottle,
) -> (Option<String>, HumanThrottle) {
    let downloaded = snapshot.downloaded_bytes;
    match snapshot.total_bytes {
        Some(total) => {
            let percent = if total == 0 {
                100
            } else {
                ((downloaded as f64 / total as f64) * 100.0)
                    .round()
                    .min(100.0) as u8
            };
            if throttle.last_percent == Some(percent) {
                return (None, throttle);
            }
            throttle.last_percent = Some(percent);
            throttle.last_emitted_bytes = downloaded;
            (
                Some(format!(
                    "  downloading {} / {} MB ({percent}%)",
                    format_bytes(downloaded),
                    format_bytes(total),
                )),
                throttle,
            )
        }
        None => {
            if downloaded.saturating_sub(throttle.last_emitted_bytes) < ONE_MEGABYTE {
                return (None, throttle);
            }
            throttle.last_emitted_bytes = downloaded;
            (
                Some(format!("  downloading {} MB", format_bytes(downloaded))),
                throttle,
            )
        }
    }
}

/// Renders upgrade progress to stderr in the selected `LogFormat`.
#[derive(Debug)]
pub struct CliProgress {
    format: LogFormat,
    throttle: Mutex<HumanThrottle>,
}

impl CliProgress {
    #[must_use]
    pub fn new(format: LogFormat) -> Self {
        Self {
            format,
            throttle: Mutex::new(HumanThrottle::default()),
        }
    }

    fn emit(&self, event: &ProgressEvent<'_>) {
        // GC per-path deletion is throttled by a fixed path-count step in
        // both formats; a store-wide sweep deletes far too many paths to
        // emit a line for each one.
        if let ProgressEvent::GcDeleted { deleted_paths } = event {
            let mut throttle = self
                .throttle
                .lock()
                .expect("BUG: progress throttle mutex poisoned");
            if deleted_paths.saturating_sub(throttle.gc_count_at_last_emit) < GC_PROGRESS_STEP {
                return;
            }
            throttle.gc_count_at_last_emit = *deleted_paths;
        }

        match self.format {
            LogFormat::InternalJson => eprintln!("{}", internal_json_line(event)),
            LogFormat::Human => {
                let mut throttle = self
                    .throttle
                    .lock()
                    .expect("BUG: progress throttle mutex poisoned");
                let (line, updated) = human_line(event, *throttle);
                *throttle = updated;
                if let Some(line) = line {
                    eprintln!("{line}");
                }
            }
        }
    }
}

impl UpgradeProgress for CliProgress {
    fn on_phase(&self, phase: UpgradePhase) {
        // A GC sub-phase that rode in on the upgrade channel is unwrapped
        // back to a GC event so it renders identically to the standalone
        // `gc` subcommand (dedicated text / `gc_phase` JSON).
        match phase {
            UpgradePhase::CollectingGarbage(gc_phase) => {
                self.emit(&ProgressEvent::GcPhase(gc_phase));
            }
            UpgradePhase::Realizing
            | UpgradePhase::Verifying
            | UpgradePhase::Building
            | UpgradePhase::Activating
            | UpgradePhase::Cleaning => self.emit(&ProgressEvent::Phase(phase)),
        }
    }

    fn on_realization_started(&self, total_paths: usize) {
        self.emit(&ProgressEvent::RealizationStarted { total_paths });
    }

    fn on_realization_finished(&self) {
        self.emit(&ProgressEvent::RealizationFinished);
    }

    fn on_download_status(&self, snapshot: &DownloadSnapshot) {
        self.emit(&ProgressEvent::Download(snapshot));
    }

    fn on_gc_deleted(&self, deleted_paths: usize) {
        self.emit(&ProgressEvent::GcDeleted { deleted_paths });
    }

    fn on_gc_finished(&self, deleted_paths: usize, freed_bytes: Option<u64>) {
        self.emit(&ProgressEvent::GcFinished {
            deleted_paths,
            freed_bytes,
        });
    }
}

impl CollectGarbageProgress for CliProgress {
    fn on_phase(&self, phase: CollectGarbagePhase) {
        self.emit(&ProgressEvent::GcPhase(phase));
    }

    fn on_deleted(&self, deleted_paths: usize) {
        self.emit(&ProgressEvent::GcDeleted { deleted_paths });
    }

    fn on_finished(&self, deleted_paths: usize, freed_bytes: Option<u64>) {
        self.emit(&ProgressEvent::GcFinished {
            deleted_paths,
            freed_bytes,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_nix::store::progress::{DownloadSnapshot, DownloadStatus};
    use bmc_nix::upgrade::UpgradePhase;

    fn snapshot(total: Option<u64>) -> DownloadSnapshot {
        DownloadSnapshot {
            active: vec![DownloadStatus {
                id: 43,
                store_path: Some("/nix/store/x".to_owned()),
                source: Some("https://h/nar".to_owned()),
                downloaded_bytes: 12_300_000,
                total_bytes: total,
                remaining_bytes: total.map(|t| t - 12_300_000),
            }],
            downloaded_bytes: 12_300_000,
            total_bytes: total,
            remaining_bytes: total.map(|t| t - 12_300_000),
        }
    }

    #[test]
    fn internal_json_line_renders_download_with_known_total() {
        let snap = snapshot(Some(45_600_000));
        let line = internal_json_line(&ProgressEvent::Download(&snap));
        assert_eq!(
            line,
            r#"@bmc {"type":"download","downloaded_bytes":12300000,"total_bytes":45600000,"remaining_bytes":33300000,"active":[{"store_path":"/nix/store/x","source":"https://h/nar","downloaded_bytes":12300000,"total_bytes":45600000}]}"#
        );
    }

    #[test]
    fn internal_json_line_renders_null_total_as_json_null() {
        let snap = snapshot(None);
        let line = internal_json_line(&ProgressEvent::Download(&snap));
        assert_eq!(
            line,
            r#"@bmc {"type":"download","downloaded_bytes":12300000,"total_bytes":null,"remaining_bytes":null,"active":[{"store_path":"/nix/store/x","source":"https://h/nar","downloaded_bytes":12300000,"total_bytes":null}]}"#
        );
    }

    #[test]
    fn internal_json_line_emits_correct_phase_string_for_every_variant() {
        for (phase, name) in [
            (UpgradePhase::Realizing, "realizing"),
            (UpgradePhase::Verifying, "verifying"),
            (UpgradePhase::Building, "building"),
            (UpgradePhase::Activating, "activating"),
            (UpgradePhase::Cleaning, "cleaning"),
        ] {
            assert_eq!(
                internal_json_line(&ProgressEvent::Phase(phase)),
                format!(r#"@bmc {{"type":"phase","phase":"{name}"}}"#),
            );
        }
    }

    #[test]
    fn internal_json_line_renders_realization_started_and_finished() {
        assert_eq!(
            internal_json_line(&ProgressEvent::RealizationStarted { total_paths: 3 }),
            r#"@bmc {"type":"realization_started","total_paths":3}"#
        );
        assert_eq!(
            internal_json_line(&ProgressEvent::RealizationFinished),
            r#"@bmc {"type":"realization_finished"}"#
        );
    }

    #[test]
    fn format_bytes_renders_one_decimal_megabytes() {
        assert_eq!(format_bytes(12_300_000), "12.3");
        assert_eq!(format_bytes(45_600_000), "45.6");
        assert_eq!(format_bytes(500_000), "0.5");
    }

    fn download_with(downloaded: u64, total: Option<u64>) -> DownloadSnapshot {
        DownloadSnapshot {
            active: vec![],
            downloaded_bytes: downloaded,
            total_bytes: total,
            remaining_bytes: total.map(|t| t.saturating_sub(downloaded)),
        }
    }

    #[test]
    fn human_line_phase_and_started_always_emit() {
        let throttle = HumanThrottle::default();
        let (line, _) = human_line(&ProgressEvent::Phase(UpgradePhase::Realizing), throttle);
        assert_eq!(line.as_deref(), Some("\u{2192} realizing"));

        let (line, _) = human_line(
            &ProgressEvent::RealizationStarted { total_paths: 3 },
            throttle,
        );
        assert_eq!(line.as_deref(), Some("  realizing 3 store paths"));

        let (line, _) = human_line(&ProgressEvent::RealizationFinished, throttle);
        assert_eq!(line, None);
    }

    #[test]
    fn human_line_known_total_throttles_to_integer_percent_change() {
        let mut throttle = HumanThrottle::default();
        let mut emitted = Vec::new();
        // 1000-byte total; percents 10, 10, 15, 16 (rounded).
        for downloaded in [100_u64, 104, 150, 156] {
            let snap = download_with(downloaded, Some(1000));
            let (line, next) = human_line(&ProgressEvent::Download(&snap), throttle);
            throttle = next;
            if let Some(line) = line {
                emitted.push(line);
            }
        }
        assert_eq!(emitted.len(), 3);
        assert_eq!(emitted[0], "  downloading 0.0 / 0.0 MB (10%)");
        assert_eq!(emitted[2], "  downloading 0.0 / 0.0 MB (16%)");
    }

    #[test]
    fn human_line_unknown_total_throttles_to_one_megabyte_steps() {
        let mut throttle = HumanThrottle::default();
        let mut emitted = Vec::new();
        for downloaded in [500_000_u64, 1_200_000, 1_500_000, 2_300_000] {
            let snap = download_with(downloaded, None);
            let (line, next) = human_line(&ProgressEvent::Download(&snap), throttle);
            throttle = next;
            if let Some(line) = line {
                emitted.push(line);
            }
        }
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0], "  downloading 1.2 MB");
        assert_eq!(emitted[1], "  downloading 2.3 MB");
    }

    #[test]
    fn internal_json_line_renders_gc_events() {
        assert_eq!(
            internal_json_line(&ProgressEvent::GcPhase(
                CollectGarbagePhase::DeterminingLiveness
            )),
            r#"@bmc {"type":"gc_phase","phase":"determining_liveness"}"#
        );
        assert_eq!(
            internal_json_line(&ProgressEvent::GcDeleted {
                deleted_paths: 1200
            }),
            r#"@bmc {"type":"gc_progress","deleted_paths":1200}"#
        );
        assert_eq!(
            internal_json_line(&ProgressEvent::GcFinished {
                deleted_paths: 2,
                freed_bytes: Some(1536)
            }),
            r#"@bmc {"type":"gc_finished","deleted_paths":2,"freed_bytes":1536}"#
        );
        assert_eq!(
            internal_json_line(&ProgressEvent::GcFinished {
                deleted_paths: 0,
                freed_bytes: None
            }),
            r#"@bmc {"type":"gc_finished","deleted_paths":0,"freed_bytes":null}"#
        );
    }

    #[test]
    fn human_line_renders_gc_events() {
        let throttle = HumanThrottle::default();

        let (line, _) = human_line(
            &ProgressEvent::GcPhase(CollectGarbagePhase::FindingRoots),
            throttle,
        );
        assert_eq!(line.as_deref(), Some("\u{2192} collecting garbage"));

        let (line, _) = human_line(
            &ProgressEvent::GcPhase(CollectGarbagePhase::DeterminingLiveness),
            throttle,
        );
        assert_eq!(line.as_deref(), Some("  determining live/dead paths"));

        let (line, _) = human_line(&ProgressEvent::GcDeleted { deleted_paths: 300 }, throttle);
        assert_eq!(line.as_deref(), Some("  deleted 300 paths"));

        let (line, _) = human_line(
            &ProgressEvent::GcFinished {
                deleted_paths: 2,
                freed_bytes: Some(1_500_000),
            },
            throttle,
        );
        assert_eq!(
            line.as_deref(),
            Some("  collected garbage: 2 paths deleted, 1.5 MB freed")
        );

        let (line, _) = human_line(
            &ProgressEvent::GcFinished {
                deleted_paths: 0,
                freed_bytes: None,
            },
            throttle,
        );
        assert_eq!(
            line.as_deref(),
            Some("  collected garbage: 0 paths deleted")
        );
    }
}
