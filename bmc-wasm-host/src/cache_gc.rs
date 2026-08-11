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

//! Cross-host GC for the on-disk widget asset cache.
//!
//! Several hosts run at once — one per WASM SDK major, since a protocol break
//! keeps old and new hosts alive until every widget upgrades. So a host can't
//! treat its own slots as the whole live set. Each host publishes the tokens it
//! holds in a GC-root file (mtime = heartbeat); reconcile keeps the union over
//! all live hosts and prunes stale roots. Shape mirrors `cleanup_stale_files`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use bmc_wasm_thin_protocol::WIDGET_CACHE_DIR;

/// Default refresh/reconcile cadence; coarse to avoid flash churn.
const GC_PERIOD_DEFAULT: Duration = Duration::from_mins(30);

/// Override the cadence (seconds) for on-device testing. Set it in the
/// environment that launches `bmc-openwrt`; the host inherits it via the thin.
const GC_PERIOD_ENV: &str = "BMC_WIDGET_GC_PERIOD_SECS";

/// Refresh/reconcile cadence, resolved once from [`GC_PERIOD_ENV`] or the default.
#[must_use]
pub fn gc_period() -> Duration {
    static PERIOD: OnceLock<Duration> = OnceLock::new();
    *PERIOD.get_or_init(|| {
        match std::env::var(GC_PERIOD_ENV)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        {
            Some(secs) if secs > 0 => {
                tracing::info!(secs, "widget cache GC period overridden");
                Duration::from_secs(secs)
            }
            _ => GC_PERIOD_DEFAULT,
        }
    })
}

/// Roots older than 2× the period are a dead host's leftovers.
fn root_stale_after() -> Duration {
    gc_period().saturating_mul(2)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GcStats {
    pub roots_pruned: usize,
    pub buckets_removed: usize,
    pub buckets_kept: usize,
}

/// GC-root dir — a sibling of the buckets, so the bucket walk stays simple.
fn gc_roots_dir() -> PathBuf {
    let cache = Path::new(WIDGET_CACHE_DIR);
    cache.parent().unwrap_or(cache).join("widget-gc-roots")
}

/// Root filename, keyed by the full SDK version — free, and future-proof if
/// concurrent-host distinction tightens from major-only to any version mismatch.
fn host_root_filename() -> String {
    let (major, minor, patch) = bmc_wasm_protocol::SDK_VERSION;
    format!("sdk-v{major}.{minor}.{patch}")
}

/// Refresh this host's root file with the tokens it holds; mtime is the heartbeat.
pub fn write_root(tokens: &[String]) -> std::io::Result<()> {
    write_root_in(&gc_roots_dir(), &host_root_filename(), tokens)
}

fn write_root_in(roots_dir: &Path, filename: &str, tokens: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(roots_dir)?;
    let mut body = String::new();
    for token in tokens {
        body.push_str(token);
        body.push('\n');
    }
    // Temp dotfile + rename so a concurrent reconcile never reads a partial file.
    let tmp = roots_dir.join(format!(".{filename}.tmp"));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, roots_dir.join(filename))
}

/// Reconcile the cache against every live host's root file.
#[must_use]
pub fn reconcile() -> GcStats {
    reconcile_in(
        Path::new(WIDGET_CACHE_DIR),
        &gc_roots_dir(),
        SystemTime::now(),
    )
}

/// Core, parameterized on dirs + `now` for tests. Best-effort: per-entry
/// failures are logged and skipped.
fn reconcile_in(cache_dir: &Path, roots_dir: &Path, now: SystemTime) -> GcStats {
    let mut stats = GcStats::default();
    let mut keep: HashSet<String> = HashSet::new();
    let mut live_roots = 0_usize;

    if let Ok(entries) = std::fs::read_dir(roots_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue; // write-temp dotfile
            }
            if is_stale(&path, now) {
                match std::fs::remove_file(&path) {
                    Ok(()) => stats.roots_pruned += 1,
                    // A peer host pruned it first — not an error.
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => {
                        tracing::warn!(path = %path.display(), %err, "failed to prune stale GC root");
                    }
                }
                continue;
            }
            live_roots += 1;
            read_tokens_into(&path, &mut keep);
        }
    }

    // A correct host writes its own root before reconciling, so zero live roots
    // means a missing picture — refuse to wipe the cache.
    if live_roots == 0 {
        tracing::warn!("no live GC roots; skipping sweep");
        return stats;
    }

    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let token = entry.file_name().to_string_lossy().into_owned();
            if keep.contains(&token) {
                stats.buckets_kept += 1;
                continue;
            }
            let path = entry.path();
            match std::fs::remove_dir_all(&path) {
                Ok(()) => stats.buckets_removed += 1,
                // A peer host removed it first — not an error.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "failed to remove orphan bucket");
                }
            }
        }
    }

    stats
}

/// Stale = mtime older than `ROOT_STALE_AFTER`; a future mtime counts as fresh.
fn is_stale(path: &Path, now: SystemTime) -> bool {
    let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    now.duration_since(mtime)
        .is_ok_and(|age| age > root_stale_after())
}

/// Collect a root file's tokens into `keep`, skipping blanks and '#' comments.
fn read_tokens_into(path: &Path, keep: &mut HashSet<String>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        tracing::warn!(path = %path.display(), "failed to read GC root");
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with('#') {
            keep.insert(line.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{reconcile_in, root_stale_after, write_root_in};
    use std::time::{Duration, SystemTime};

    fn mk_bucket(cache: &std::path::Path, token: &str) {
        let dir = cache.join(token);
        std::fs::create_dir_all(&dir).expect("BUG: create bucket dir");
        std::fs::write(dir.join("image"), b"blob").expect("BUG: write bucket blob");
    }

    fn write(roots: &std::path::Path, host: &str, tokens: &[&str]) {
        let owned: Vec<String> = tokens.iter().map(|t| (*t).to_owned()).collect();
        write_root_in(roots, host, &owned).expect("BUG: write root file");
    }

    #[test]
    fn keeps_claimed_removes_orphan() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let cache = tmp.path().join("widget-cache");
        let roots = tmp.path().join("widget-gc-roots");
        mk_bucket(&cache, "uuid-a-full");
        mk_bucket(&cache, "uuid-b-2x1");
        write(&roots, "sdk-v0", &["uuid-a-full"]);

        let stats = reconcile_in(&cache, &roots, SystemTime::now());

        assert_eq!(stats.buckets_kept, 1);
        assert_eq!(stats.buckets_removed, 1);
        assert!(cache.join("uuid-a-full").exists());
        assert!(!cache.join("uuid-b-2x1").exists());
    }

    #[test]
    fn unions_across_live_hosts() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let cache = tmp.path().join("widget-cache");
        let roots = tmp.path().join("widget-gc-roots");
        mk_bucket(&cache, "tok-a");
        mk_bucket(&cache, "tok-b");
        mk_bucket(&cache, "tok-c");
        write(&roots, "sdk-v0", &["tok-a"]);
        write(&roots, "sdk-v1", &["tok-b"]);

        let stats = reconcile_in(&cache, &roots, SystemTime::now());

        assert_eq!(stats.buckets_kept, 2);
        assert_eq!(stats.buckets_removed, 1);
        assert!(!cache.join("tok-c").exists());
    }

    #[test]
    fn prunes_stale_root_and_drops_its_claim() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let cache = tmp.path().join("widget-cache");
        let roots = tmp.path().join("widget-gc-roots");
        mk_bucket(&cache, "tok-live");
        mk_bucket(&cache, "tok-dead");
        write(&roots, "sdk-v0", &["tok-live"]);
        write(&roots, "sdk-v9", &["tok-dead"]);

        let old = SystemTime::now() - (root_stale_after() + Duration::from_mins(1));
        set_mtime(&roots.join("sdk-v9"), old);

        let stats = reconcile_in(&cache, &roots, SystemTime::now());

        assert_eq!(stats.roots_pruned, 1);
        assert!(!roots.join("sdk-v9").exists());
        assert!(cache.join("tok-live").exists());
        assert!(!cache.join("tok-dead").exists());
    }

    #[test]
    fn skips_sweep_when_no_live_roots() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let cache = tmp.path().join("widget-cache");
        let roots = tmp.path().join("widget-gc-roots");
        mk_bucket(&cache, "tok-a");

        let stats = reconcile_in(&cache, &roots, SystemTime::now());

        assert_eq!(stats.buckets_removed, 0);
        assert!(cache.join("tok-a").exists());
    }

    fn set_mtime(path: &std::path::Path, when: SystemTime) {
        let f = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("BUG: open for mtime");
        f.set_times(std::fs::FileTimes::new().set_modified(when))
            .expect("BUG: set mtime");
    }
}
