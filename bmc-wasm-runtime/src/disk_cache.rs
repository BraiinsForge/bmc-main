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

//! Flash-backed blob cache keyed by an opaque path-safe id.
//!
//! Content-agnostic: it stores opaque `bytes` with a first-class `saved_at`
//! stamp and a caller-owned `metadata` blob — the caller owns what both mean.
//! Reads are `mmap`'d for zero-copy restore; the cache owns only the on-disk
//! format, a byte-cap LRU, and mark-and-sweep against a live-key set.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

/// On-disk header (LE): `saved_at` u64, `meta_len` u32; then metadata, then bytes.
const HEADER: usize = 12;
const EXT: &str = "blob";

/// Sidecar holding `saved_at` alone, so re-verifying an unchanged entry can
/// restamp it without rewriting the payload. Shadows the header when present.
const STAMP_EXT: &str = "ts";

/// How long a `.tmp` must sit untouched before a sweep treats it as dead.
/// A put finishes in milliseconds; the gap is what stops one process
/// from reclaiming another's temp mid-write.
const TMP_ORPHAN_AGE: std::time::Duration = std::time::Duration::from_hours(1);

/// Serialises a bucket's writers.
/// Nothing else matches a `lock` extension, so sweeps pass over it.
const LOCK_NAME: &str = ".put.lock";

/// Holds a bucket's write lock for one `put`.
///
/// The content check and the write it decides on are two steps; without one,
/// a writer can stamp its timestamp onto content another writer replaced.
#[derive(Debug)]
struct PutGuard {
    file: fs::File,
}

impl PutGuard {
    fn acquire(dir: &Path) -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join(LOCK_NAME))?;
        rustix::io::retry_on_intr(|| {
            rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        })
        .map_err(io::Error::from)?;
        Ok(Self { file })
    }
}

impl Drop for PutGuard {
    /// The kernel frees it with the descriptor; this only reports the odd failure.
    fn drop(&mut self) {
        if let Err(error) = rustix::fs::flock(&self.file, rustix::fs::FlockOperation::Unlock) {
            tracing::warn!(?error, "failed to release the cache write lock");
        }
    }
}

/// A temp path no concurrent writer can also be holding.
///
/// Two processes putting one key must not share a temp name:
/// they would interleave into one file and rename a half-written result.
/// The pid separates processes, the counter separates writes within one.
fn temp_path(final_path: &Path, suffix: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    final_path.with_extension(format!("{}.{seq}.{suffix}", std::process::id()))
}

/// Whether a `.tmp` has sat long enough to be a crashed put
/// rather than one still running.
/// An unreadable or future-dated mtime is left alone.
fn is_orphaned_temp(entry: &fs::DirEntry) -> bool {
    entry
        .metadata()
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age >= TMP_ORPHAN_AGE))
}

/// Delete the temps a crashed `put` left in a bucket, nothing else.
///
/// Not `sweep`: that deletes every entry whose key is absent from the live set,
/// which only the widget owning the bucket can supply. Temps need no such list,
/// so a caller holding just the directory can reclaim them safely.
pub fn reclaim_orphaned_temps(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("tmp") && is_orphaned_temp(&entry) {
            let _ = fs::remove_file(&path);
        }
    }
}

fn read_stamp(blob_path: &Path) -> Option<u64> {
    let raw = fs::read(blob_path.with_extension(STAMP_EXT)).ok()?;
    Some(u64::from_le_bytes(raw.get(..8)?.try_into().ok()?))
}

/// Total on-disk size of an entry (`header + metadata + payload`); `None` on overflow.
fn entry_len(metadata_len: usize, bytes_len: usize) -> Option<u64> {
    (HEADER as u64)
        .checked_add(metadata_len as u64)?
        .checked_add(bytes_len as u64)
}

/// Flash store under a single directory, trimmed to a byte cap.
#[derive(Debug, Clone)]
pub struct DiskCache {
    dir: PathBuf,
    max_bytes: u64,
}

/// An `mmap`'d entry; [`Self::metadata`]/[`Self::bytes`] borrow the mapped file.
#[derive(Debug)]
pub struct CachedBlob {
    map: Mmap,
    /// `HEADER + meta_len`, validated `<= map.len()` so the slices can't overflow.
    meta_end: usize,
    pub saved_at: u64,
}

impl CachedBlob {
    /// Opaque caller metadata stored beside the artifact (e.g. a URL hash, dimensions).
    #[must_use]
    pub fn metadata(&self) -> &[u8] {
        &self.map[HEADER..self.meta_end]
    }

    /// The cached artifact bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.map[self.meta_end..]
    }
}

impl DiskCache {
    #[must_use]
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self { dir, max_bytes }
    }

    /// `None` for a path-escaping key, so a guest tag can't reach a sibling bucket.
    fn path(&self, key: &str) -> Option<PathBuf> {
        if key.is_empty() || key.contains('/') || key.contains('\\') || key.contains("..") {
            return None;
        }
        Some(self.dir.join(format!("{key}.{EXT}")))
    }

    /// Write an entry (temp+rename, torn-read-safe), then trim. `saved_at` is a
    /// caller UTC epoch; `metadata` is an opaque blob returned verbatim by `get`.
    pub fn put(&self, key: &str, saved_at: u64, metadata: &[u8], bytes: &[u8]) -> io::Result<()> {
        let path = self
            .path(key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid cache key"))?;
        let meta_len = u32::try_from(metadata.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "metadata too large"))?;

        // Reject an over-cap entry before creating the temp file,
        // so an oversized write never lands and leaves no `.tmp` behind.
        if !self.accepts_entry(metadata.len(), bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache entry exceeds bucket cap",
            ));
        }

        fs::create_dir_all(&self.dir)?;
        let _writing = PutGuard::acquire(&self.dir)?;

        // A re-verified entry only needs a fresh stamp; rewriting an unchanged
        // payload would spend its full size in flash on every refresh.
        if self.matches(key, metadata, bytes) {
            return self.write_stamp(&path, saved_at);
        }

        let tmp = temp_path(&path, "tmp");
        // Stream header/metadata/payload rather than concatenating one big buffer.
        let mut w = io::BufWriter::new(fs::File::create(&tmp)?);
        w.write_all(&saved_at.to_le_bytes())?;
        w.write_all(&meta_len.to_le_bytes())?;
        w.write_all(metadata)?;
        w.write_all(bytes)?;
        w.flush()?;
        drop(w);
        fs::rename(&tmp, &path)?;
        // The header now carries the current stamp; a leftover sidecar would shadow it.
        let _ = fs::remove_file(path.with_extension(STAMP_EXT));
        self.trim();
        Ok(())
    }

    /// Whether the stored entry is byte-identical to what `put` would write.
    fn matches(&self, key: &str, metadata: &[u8], bytes: &[u8]) -> bool {
        self.get(key)
            .is_some_and(|blob| blob.metadata() == metadata && blob.bytes() == bytes)
    }

    /// Replace the sidecar stamp via temp+rename, so a concurrent reader sees
    /// either the old value or the new one, never a half-written u64.
    fn write_stamp(&self, blob_path: &Path, saved_at: u64) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let tmp = temp_path(blob_path, "ts.tmp");
        fs::write(&tmp, saved_at.to_le_bytes())?;
        fs::rename(&tmp, blob_path.with_extension(STAMP_EXT))
    }

    /// True if a `metadata_len` + `bytes_len` entry fits the bucket cap. Callers
    /// check the declared sizes before copying the payload out of guest memory.
    #[must_use]
    pub fn accepts_entry(&self, metadata_len: usize, bytes_len: usize) -> bool {
        entry_len(metadata_len, bytes_len).is_some_and(|len| len <= self.max_bytes)
    }

    /// `mmap` the entry for `key`, validating its header. `None` on a miss or a
    /// malformed file, so the caller falls back to a fresh fetch.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<CachedBlob> {
        let path = self.path(key)?;
        let file = fs::File::open(&path).ok()?;
        // SAFETY: host-owned dir, entries written atomically (temp+rename), so a
        // mapped file is never truncated mid-read by this process.
        let map = unsafe { Mmap::map(&file) }.ok()?;
        if map.len() < HEADER {
            return None;
        }
        let saved_at = u64::from_le_bytes(map[0..8].try_into().ok()?);
        let meta_len = u32::from_le_bytes(map[8..12].try_into().ok()?) as usize;
        // checked_add so a crafted meta_len near usize::MAX can't wrap on armv7.
        let meta_end = HEADER.checked_add(meta_len).filter(|&e| e <= map.len())?;
        Some(CachedBlob {
            map,
            meta_end,
            saved_at: read_stamp(&path).unwrap_or(saved_at),
        })
    }

    pub fn evict(&self, key: &str) {
        if let Some(path) = self.path(key) {
            let _ = fs::remove_file(path.with_extension(STAMP_EXT));
            let _ = fs::remove_file(path);
        }
    }

    /// Mark-and-sweep: delete every entry whose key is not in `live`. Models
    /// `bmc-nix` `cleanup_stale_files` — the GC root is the live config set.
    pub fn sweep(&self, live: &HashSet<&str>) {
        reclaim_orphaned_temps(&self.dir);
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !matches!(
                path.extension().and_then(|e| e.to_str()),
                Some(EXT | STAMP_EXT)
            ) {
                continue;
            }
            let key = path.file_stem().and_then(|s| s.to_str());
            if key.is_some_and(|k| !live.contains(k)) {
                let _ = fs::remove_file(&path);
            }
        }
    }

    /// Entry count and total bytes currently stored (for size reporting).
    #[must_use]
    pub fn stats(&self) -> (usize, u64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return (0, 0);
        };
        let mut count = 0;
        let mut bytes = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXT) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                count += 1;
                bytes += meta.len();
            }
        }
        (count, bytes)
    }

    /// Trim to `max_bytes`, evicting least-recently-used entries first.
    ///
    /// Recency is the newer of the payload and its sidecar: a deduplicated
    /// entry stops advancing its own mtime while still being verified, so
    /// payload mtime alone would evict precisely the entries still in use.
    fn trim(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some(EXT) {
                    return None;
                }
                let meta = e.metadata().ok()?;
                if !meta.is_file() {
                    return None;
                }
                let written = meta.modified().ok()?;
                let stamped = fs::metadata(path.with_extension(STAMP_EXT))
                    .and_then(|s| s.modified())
                    .ok();
                let used = stamped.map_or(written, |stamp| stamp.max(written));
                Some((path, used, meta.len()))
            })
            .collect();
        let total: u64 = files.iter().map(|(_, _, len)| len).sum();
        let Some(mut over) = total.checked_sub(self.max_bytes).filter(|o| *o > 0) else {
            return;
        };
        files.sort_by_key(|(_, used, _)| *used);
        for (path, _, len) in files {
            if over == 0 {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                let _ = fs::remove_file(path.with_extension(STAMP_EXT));
                over = over.saturating_sub(len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: u64 = 1_700_000_000_000;

    fn cache(max_bytes: u64) -> (tempfile::TempDir, DiskCache) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = DiskCache::new(dir.path().to_path_buf(), max_bytes);
        (dir, cache)
    }

    #[test]
    fn put_get_round_trips() {
        let (_d, c) = cache(1 << 20);
        c.put("k", TS, b"meta", b"payload").expect("put");
        let got = c.get("k").expect("hit");
        assert_eq!(got.saved_at, TS);
        assert_eq!(got.metadata(), b"meta");
        assert_eq!(got.bytes(), b"payload");
    }

    /// Path of the sidecar for `key`, whose presence marks a deduplicated write.
    fn stamp_of(dir: &tempfile::TempDir, key: &str) -> PathBuf {
        dir.path().join(format!("{key}.{STAMP_EXT}"))
    }

    #[test]
    fn identical_content_restamps_instead_of_rewriting() {
        let (d, c) = cache(1 << 20);
        c.put("k", TS, b"meta", b"payload").expect("put");
        assert!(
            !stamp_of(&d, "k").exists(),
            "a first write carries its stamp in the header, not a sidecar"
        );

        c.put("k", TS + 5_000, b"meta", b"payload").expect("re-put");
        assert!(
            stamp_of(&d, "k").exists(),
            "an unchanged payload must restamp via the sidecar, not rewrite"
        );
        assert_eq!(c.get("k").expect("hit").saved_at, TS + 5_000);
    }

    #[test]
    fn changed_content_rewrites_and_drops_the_sidecar() {
        let (d, c) = cache(1 << 20);
        c.put("k", TS, b"meta", b"payload").expect("put");
        c.put("k", TS + 5_000, b"meta", b"payload").expect("re-put");

        c.put("k", TS + 9_000, b"meta", b"changed")
            .expect("put changed");
        assert!(
            !stamp_of(&d, "k").exists(),
            "a rewritten header owns the stamp; a stale sidecar would shadow it"
        );
        let got = c.get("k").expect("hit");
        assert_eq!(got.saved_at, TS + 9_000);
        assert_eq!(got.bytes(), b"changed");
    }

    #[test]
    fn evict_and_sweep_take_the_sidecar_too() {
        let (d, c) = cache(1 << 20);
        c.put("gone", TS, b"m", b"p").expect("put");
        c.put("gone", TS + 1, b"m", b"p").expect("re-put");
        assert!(stamp_of(&d, "gone").exists());
        c.evict("gone");
        assert!(
            !stamp_of(&d, "gone").exists(),
            "evict must not orphan a stamp"
        );

        c.put("swept", TS, b"m", b"p").expect("put");
        c.put("swept", TS + 1, b"m", b"p").expect("re-put");
        c.sweep(&HashSet::new());
        assert!(
            !stamp_of(&d, "swept").exists(),
            "sweep must not orphan a stamp"
        );
    }

    #[test]
    fn trim_evicts_by_last_verified_not_last_written() {
        let payload = [7_u8; 512];
        // Roomy while seeding, so `put`'s own trim can't evict mid-setup.
        let (d, c) = cache(1 << 20);
        c.put("old", TS, b"m", &payload).expect("put old");
        c.put("new", TS, b"m", &payload).expect("put new");
        // Re-verifying "old" restamps the sidecar and leaves the payload untouched.
        c.put("old", TS + 1, b"m", &payload).expect("re-put old");

        let at = |secs| std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        let set_mtime = |path: PathBuf, secs| {
            fs::File::options()
                .write(true)
                .open(path)
                .expect("open")
                .set_modified(at(secs))
                .expect("set mtime");
        };
        // "old" was written first but verified last, so payload mtime alone
        // would pick it as the eviction candidate.
        set_mtime(d.path().join("old.blob"), 1_000);
        set_mtime(d.path().join("new.blob"), 2_000);
        set_mtime(stamp_of(&d, "old"), 3_000);

        // One entry fits, two do not, so exactly one must go.
        let tight = DiskCache::new(
            d.path().to_path_buf(),
            entry_len(1, payload.len()).expect("len") + 1,
        );
        tight.trim();
        assert!(
            tight.get("old").is_some(),
            "the most recently verified entry must survive"
        );
        assert!(
            tight.get("new").is_none(),
            "the least recently used entry goes"
        );
    }

    #[test]
    fn rejects_path_escaping_keys() {
        let (_d, c) = cache(1 << 20);
        assert!(c.put("../escape", TS, b"m", b"p").is_err());
        assert!(c.get("../escape").is_none());
        assert!(c.get("a/b").is_none());
        assert!(c.get("").is_none());
        c.evict("../escape"); // no-op; must not touch a sibling path
    }

    #[test]
    fn empty_metadata_round_trips() {
        let (_d, c) = cache(1 << 20);
        c.put("k", TS, &[], b"payload").expect("put");
        let got = c.get("k").expect("hit");
        assert!(got.metadata().is_empty());
        assert_eq!(got.bytes(), b"payload");
    }

    #[test]
    fn miss_is_none() {
        let (_d, c) = cache(1 << 20);
        assert!(c.get("absent").is_none());
    }

    #[test]
    fn truncated_metadata_is_none() {
        let (_d, c) = cache(1 << 20);
        // meta_len claims 99 bytes but the file ends right after the header.
        let mut bad = TS.to_le_bytes().to_vec();
        bad.extend_from_slice(&99_u32.to_le_bytes());
        std::fs::write(c.path("k").expect("valid key"), bad).expect("write");
        assert!(c.get("k").is_none());
    }

    #[test]
    fn huge_meta_len_is_none() {
        let (_d, c) = cache(1 << 20);
        // meta_len = u32::MAX would wrap HEADER + meta_len on a 32-bit usize.
        let mut bad = TS.to_le_bytes().to_vec();
        bad.extend_from_slice(&u32::MAX.to_le_bytes());
        bad.extend_from_slice(b"some payload");
        std::fs::write(c.path("k").expect("valid key"), bad).expect("write");
        assert!(c.get("k").is_none());
    }

    #[test]
    fn over_cap_entry_is_rejected_with_no_leftover_files() {
        let (d, c) = cache(1 << 10); // 1 KiB bucket
        assert!(c.put("k", TS, &[], &vec![0_u8; 4096]).is_err());
        assert!(c.get("k").is_none());
        let stray: Vec<_> = std::fs::read_dir(d.path())
            .expect("readdir")
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        assert!(stray.is_empty(), "leftover files: {stray:?}");
    }

    #[test]
    fn accepts_entry_matches_the_cap() {
        let (_d, c) = cache(HEADER as u64 + 10);
        assert!(c.accepts_entry(4, 6)); // 12 + 4 + 6 == 22 == cap
        assert!(!c.accepts_entry(4, 7)); // 23 > cap
    }

    #[test]
    fn sweep_keeps_live_drops_orphans() {
        let (_d, c) = cache(1 << 20);
        c.put("live", TS, &[], b"a").expect("put live");
        c.put("orphan", TS, &[], b"b").expect("put orphan");
        c.sweep(&["live"].into_iter().collect());
        assert!(c.get("live").is_some());
        assert!(c.get("orphan").is_none());
    }

    #[test]
    fn sweep_reclaims_tmp_orphans() {
        let (d, c) = cache(1 << 20);
        let tmp = d.path().join("k.tmp");
        let file = std::fs::File::create(&tmp).expect("create tmp");
        file.set_modified(std::time::SystemTime::now() - TMP_ORPHAN_AGE * 2)
            .expect("age the tmp past the orphan threshold");
        drop(file);

        c.sweep(&std::collections::HashSet::new());
        assert!(!tmp.exists());
    }

    /// The caller holds only the directory and cannot know which keys are live,
    /// so touching a stored entry would risk deleting one still in use.
    #[test]
    fn reclaiming_temps_spares_every_stored_entry() {
        let (d, c) = cache(1 << 20);
        c.put("kept", TS, &[], b"a").expect("put kept");
        let tmp = d.path().join("kept.tmp");
        let file = std::fs::File::create(&tmp).expect("create tmp");
        file.set_modified(std::time::SystemTime::now() - TMP_ORPHAN_AGE * 2)
            .expect("age the tmp past the orphan threshold");
        drop(file);

        reclaim_orphaned_temps(d.path());
        assert!(!tmp.exists(), "the crashed put's temp is reclaimed");
        assert!(c.get("kept").is_some(), "a stored entry survives untouched");
    }

    /// A second testbed on one bucket is mid-`put` while this one sweeps;
    /// reclaiming its temp would rename a truncated file into place.
    #[test]
    fn sweep_leaves_a_temp_another_writer_may_still_be_inside() {
        let (d, c) = cache(1 << 20);
        let tmp = d.path().join("k.tmp");
        std::fs::write(&tmp, b"partial").expect("write tmp");

        c.sweep(&std::collections::HashSet::new());
        assert!(
            tmp.exists(),
            "a temp written moments ago belongs to a live put"
        );
    }

    /// A second writer waits instead of slipping between another's check
    /// and the stamp that check justified.
    #[test]
    fn a_put_holds_the_bucket_against_another_writer() {
        let (d, _c) = cache(1 << 20);
        std::fs::create_dir_all(d.path()).expect("BUG: bucket dir");
        let held = PutGuard::acquire(d.path()).expect("BUG: first writer takes the bucket");

        let contender = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(d.path().join(LOCK_NAME))
            .expect("BUG: contender opens the lock file");
        let error = rustix::fs::flock(
            &contender,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .expect_err("a second writer must wait for the one holding the bucket");
        assert_eq!(error, rustix::io::Errno::WOULDBLOCK);

        drop(held);
        rustix::fs::flock(
            &contender,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .expect("the bucket frees on release");
    }

    /// Writers racing on one key used to share `<key>.tmp`,
    /// so a rename could publish what another was still writing.
    #[test]
    fn concurrent_puts_of_one_key_leave_a_whole_entry() {
        let (_d, c) = cache(1 << 20);
        let payload = vec![7_u8; 64 * 1024];

        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    c.put("shared", 1, b"meta", &payload).expect("BUG: put");
                });
            }
        });

        let blob = c.get("shared").expect("BUG: an entry survives the race");
        assert_eq!(blob.metadata(), b"meta");
        assert_eq!(
            blob.bytes(),
            payload.as_slice(),
            "a reader must never see a torn write"
        );
    }

    #[test]
    fn trim_enforces_cap() {
        let payload = vec![0_u8; 64];
        let one = HEADER as u64 + 64; // header + one 64-byte payload, no metadata
        let (_d, c) = cache(one + 8); // room for one, not two
        c.put("a", TS, &[], &payload).expect("put a");
        c.put("b", TS, &[], &payload).expect("put b");
        let survivors = [c.get("a"), c.get("b")]
            .iter()
            .filter(|e| e.is_some())
            .count();
        assert_eq!(survivors, 1, "cap must keep exactly one entry");
    }
}
