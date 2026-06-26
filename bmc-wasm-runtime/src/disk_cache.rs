// Copyright (C) 2026  Braiins Systems s.r.o.

//! Flash-backed blob cache keyed by an opaque path-safe id.
//!
//! Content-agnostic: it stores opaque `bytes` with a first-class `saved_at`
//! stamp and a caller-owned `metadata` blob — the caller owns what both mean.
//! Reads are `mmap`'d for zero-copy restore; the cache owns only the on-disk
//! format, a byte-cap LRU, and mark-and-sweep against a live-key set.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use memmap2::Mmap;

/// On-disk header (LE): `saved_at` u64, `meta_len` u32; then metadata, then bytes.
const HEADER: usize = 12;
const EXT: &str = "blob";

/// Flash store under a single directory, trimmed to a byte cap.
#[derive(Debug)]
pub struct DiskCache {
    dir: PathBuf,
    max_bytes: u64,
}

/// An `mmap`'d entry; [`Self::metadata`]/[`Self::bytes`] borrow the mapped file.
#[derive(Debug)]
pub struct CachedBlob {
    map: Mmap,
    meta_len: usize,
    pub saved_at: u64,
}

impl CachedBlob {
    /// Opaque caller metadata stored beside the artifact (e.g. a URL hash, dimensions).
    #[must_use]
    pub fn metadata(&self) -> &[u8] {
        &self.map[HEADER..HEADER + self.meta_len]
    }

    /// The cached artifact bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.map[HEADER + self.meta_len..]
    }
}

impl DiskCache {
    #[must_use]
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self { dir, max_bytes }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.{EXT}"))
    }

    /// Write an entry (temp+rename, torn-read-safe), then trim. `saved_at` is a
    /// caller UTC epoch; `metadata` is an opaque blob returned verbatim by `get`.
    pub fn put(&self, key: &str, saved_at: u64, metadata: &[u8], bytes: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let meta_len = u32::try_from(metadata.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "metadata too large"))?;
        let mut buf = Vec::with_capacity(HEADER + metadata.len() + bytes.len());
        buf.extend_from_slice(&saved_at.to_le_bytes());
        buf.extend_from_slice(&meta_len.to_le_bytes());
        buf.extend_from_slice(metadata);
        buf.extend_from_slice(bytes);
        let path = self.path(key);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &buf)?;
        fs::rename(&tmp, &path)?;
        self.trim();
        Ok(())
    }

    /// `mmap` the entry for `key`, validating its header. `None` on a miss or a
    /// malformed file, so the caller falls back to a fresh fetch.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<CachedBlob> {
        let file = fs::File::open(self.path(key)).ok()?;
        // SAFETY: host-owned dir, entries written atomically (temp+rename), so a
        // mapped file is never truncated mid-read by this process.
        let map = unsafe { Mmap::map(&file) }.ok()?;
        if map.len() < HEADER {
            return None;
        }
        let saved_at = u64::from_le_bytes(map[0..8].try_into().ok()?);
        let meta_len = u32::from_le_bytes(map[8..12].try_into().ok()?) as usize;
        if map.len() < HEADER + meta_len {
            return None;
        }
        Some(CachedBlob {
            map,
            meta_len,
            saved_at,
        })
    }

    pub fn evict(&self, key: &str) {
        let _ = fs::remove_file(self.path(key));
    }

    /// Mark-and-sweep: delete every entry whose key is not in `live`. Models
    /// `bmc-nix` `cleanup_stale_files` — the GC root is the live config set.
    pub fn sweep(&self, live: &HashSet<&str>) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXT) {
                continue;
            }
            let key = path.file_stem().and_then(|s| s.to_str());
            if key.is_some_and(|k| !live.contains(k)) {
                let _ = fs::remove_file(&path);
            }
        }
    }

    /// Trim to `max_bytes`, evicting oldest-modified entries first.
    fn trim(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = entries
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                meta.is_file()
                    .then(|| Some((e.path(), meta.modified().ok()?, meta.len())))
                    .flatten()
            })
            .collect();
        let total: u64 = files.iter().map(|(_, _, len)| len).sum();
        let Some(mut over) = total.checked_sub(self.max_bytes).filter(|o| *o > 0) else {
            return;
        };
        files.sort_by_key(|(_, mtime, _)| *mtime);
        for (path, _, len) in files {
            if over == 0 {
                break;
            }
            if fs::remove_file(&path).is_ok() {
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
        std::fs::write(c.path("k"), bad).expect("write");
        assert!(c.get("k").is_none());
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
