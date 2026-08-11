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

//! Guest-side widget parameter snapshots.
//!
//! A [`Params`] is an owned snapshot of every parameter the host delivered for the current widget
//! instance. Widgets read params through the typed accessors ([`Params::get_str`],
//! [`Params::get_i32`], [`Params::get_f64`], [`Params::get_bool`]); the parser is lazy,
//! so an accessor only walks the buffer as far as it needs to find the requested key.
//!
//! ## Wire format
//!
//! Snapshots arrive from the host as a packed byte buffer in little-endian order.
//! The format mirrors the manifest's `ParamKind` value space:
//!
//! ```text
//! u32  count
//! for each entry:
//!   u8   kind       0 = str, 1 = i32, 2 = f64, 3 = bool, 4 = null
//!   u16  key_len
//!   key_len bytes utf-8
//!   variant payload:
//!     str  → u32 len; len bytes utf-8
//!     i32  → 4 bytes LE
//!     f64  → 8 bytes LE
//!     bool → 1 byte
//!     null → no bytes
//! ```
//!
//! Null entries are packed for every manifest-declared key that has no resolved value — the snapshot's shape
//! is faithful to the manifest. The typed accessors return `None` for null entries (same as for missing keys);
//! [`Params::keys`] still yields them so callers iterating the full set see the full manifest.
//!
//! ## Snapshot lifecycle
//!
//! [`current`] returns the latest snapshot the host has delivered.
//! The first call from `init` reads via the `host_params_snapshot` import; subsequent calls
//! reuse the cached bytes until the host bumps `host_params_version`, at which point the next
//! [`current`] re-fetches and the old snapshot is moved into [`previous`].
//!
//! [`previous`] returns the snapshot immediately before [`current`].
//! It is [`Params::default`] until at least one update has been observed.
//! `on_params_update` is the canonical place to diff the two for React-style change detection.
//!
//! ## End-to-end byte flow
//!
//! ```text
//! host BTreeMap<ParamKey, ParamValue>
//!   └─ encode_params  →  Vec<u8> in the wire format above
//!        └─ extern fn host_params_snapshot  (probe-then-fetch via wasm imports)
//!             └─ thread-local ParamsCache { current, previous, last_seen_version }
//!                  └─ snapshot::current()  →  Params { bytes: Vec<u8> }   (clone of cached buffer)
//!                       └─ Params::get_str / get_i32 / …
//!                            →  linear scan for the matching key
//!                            →  Option<&str>  borrowed into snap.bytes
//!                       └─ typed::ParamRead::read_required / read_optional
//!                            →  .to_owned() for String, by value for primitives
//!                            →  from_manifest_value(...) for enum_values fields
//! ```
//!
//! - `host_params_version` is a cheap separate import; idle frames don't cross the
//!   wasm boundary for params at all (the cache short-circuits when the version is unchanged).
//! - The fetch is probe-then-allocate: `host_params_snapshot(null, 0)` returns the
//!   required byte length; the SDK allocates an exact `Vec<u8>` and calls back to fill it.
//! - `Params::current()` clones the cache's `Vec<u8>` so the caller owns a snapshot
//!   independent of subsequent host updates. The clone is the only allocation per frame
//!   when nothing changed; accessors borrow into the cloned buffer until you copy out.

pub mod typed;

use bmc_wasm_protocol::params::kind;

/// Owned snapshot of the host-delivered params for this widget instance.
///
/// Cheap to [`Clone`] — the underlying storage is a single `Vec<u8>`.
/// Designed so the React-style "did this param change since last render"
/// pattern can hold both [`current`] and [`previous`] without crossing
/// the wasm boundary on every read.
#[derive(Clone, Default, Debug)]
pub struct Params {
    /// Packed bytes in the wire format above.
    bytes: Vec<u8>,
}

impl Params {
    /// Build a `Params` from an owned packed-byte buffer.
    ///
    /// The host owns the wire layout; a malformed buffer here means the host messed up,
    /// not the widget. The constructor accepts the bytes unconditionally — accessors
    /// and iterators stop on the first parse error and yield no further entries.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Returns `true` when the snapshot carries no entries (count == 0,
    /// or the buffer is too short to even read the count header).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Number of entries the snapshot's header claims.
    /// Reads the first 4 LE bytes; returns 0 if the buffer is shorter than the header itself.
    fn count(&self) -> u32 {
        let Some(head) = self.bytes.first_chunk::<4>() else {
            return 0;
        };
        u32::from_le_bytes(*head)
    }

    /// Returns the string value for `key`, or `None` if the key
    /// is missing, null, or has a different kind.
    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.entries()
            .find(|e| e.key == key)
            .and_then(|e| e.value.as_str())
    }

    /// Returns the i32 value for `key`, or `None` if the key
    /// is missing, null, or has a different kind.
    #[must_use]
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.entries()
            .find(|e| e.key == key)
            .and_then(|e| e.value.as_i32())
    }

    /// Returns the f64 value for `key`, or `None` if the key
    /// is missing, null, or has a different kind.
    #[must_use]
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.entries()
            .find(|e| e.key == key)
            .and_then(|e| e.value.as_f64())
    }

    /// Returns the boolean value for `key`, or `None` if the key
    /// is missing, null, or has a different kind.
    #[must_use]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.entries()
            .find(|e| e.key == key)
            .and_then(|e| e.value.as_bool())
    }

    /// Iterator over every key the snapshot carries, including keys whose value is `null`.
    /// Order matches the host's serialisation order, which is alphabetical by key
    /// for deterministic snapshot equality.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries().map(|e| e.key)
    }

    /// Internal walker over decoded entries. Stops at the first parse error.
    fn entries(&self) -> EntryIter<'_> {
        EntryIter {
            bytes: &self.bytes,
            remaining: self.count(),
            offset: 4,
        }
    }
}

#[derive(Debug)]
struct Entry<'a> {
    key: &'a str,
    value: EntryValue<'a>,
}

#[derive(Debug)]
enum EntryValue<'a> {
    Str(&'a str),
    I32(i32),
    F64(f64),
    Bool(bool),
    Null,
}

impl<'a> EntryValue<'a> {
    fn as_str(&self) -> Option<&'a str> {
        if let Self::Str(s) = *self {
            Some(s)
        } else {
            None
        }
    }

    fn as_i32(&self) -> Option<i32> {
        if let Self::I32(v) = *self {
            Some(v)
        } else {
            None
        }
    }

    fn as_f64(&self) -> Option<f64> {
        if let Self::F64(v) = *self {
            Some(v)
        } else {
            None
        }
    }

    fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = *self {
            Some(b)
        } else {
            None
        }
    }
}

struct EntryIter<'a> {
    bytes: &'a [u8],
    remaining: u32,
    offset: usize,
}

impl<'a> Iterator for EntryIter<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Entry<'a>> {
        if self.remaining == 0 {
            return None;
        }
        let entry = self.read_entry()?;
        self.remaining -= 1;
        Some(entry)
    }
}

impl<'a> EntryIter<'a> {
    fn read_entry(&mut self) -> Option<Entry<'a>> {
        let kind = *self.bytes.get(self.offset)?;
        self.offset = self.offset.checked_add(1)?;

        let key_len_end = self.offset.checked_add(2)?;
        let key_len = u16::from_le_bytes(
            *self
                .bytes
                .get(self.offset..key_len_end)?
                .first_chunk::<2>()?,
        ) as usize;
        self.offset = key_len_end;

        let key_end = self.offset.checked_add(key_len)?;
        let key_bytes = self.bytes.get(self.offset..key_end)?;
        let key = core::str::from_utf8(key_bytes).ok()?;
        self.offset = key_end;

        let value = match kind {
            kind::STR => {
                let str_len_end = self.offset.checked_add(4)?;
                let str_len = u32::from_le_bytes(
                    *self
                        .bytes
                        .get(self.offset..str_len_end)?
                        .first_chunk::<4>()?,
                ) as usize;
                self.offset = str_len_end;
                let s_end = self.offset.checked_add(str_len)?;
                let s_bytes = self.bytes.get(self.offset..s_end)?;
                let s = core::str::from_utf8(s_bytes).ok()?;
                self.offset = s_end;
                EntryValue::Str(s)
            }
            kind::I32 => {
                let end = self.offset.checked_add(4)?;
                let bytes = self.bytes.get(self.offset..end)?.first_chunk::<4>()?;
                self.offset = end;
                EntryValue::I32(i32::from_le_bytes(*bytes))
            }
            kind::F64 => {
                let end = self.offset.checked_add(8)?;
                let bytes = self.bytes.get(self.offset..end)?.first_chunk::<8>()?;
                self.offset = end;
                EntryValue::F64(f64::from_le_bytes(*bytes))
            }
            kind::BOOL => {
                let b = *self.bytes.get(self.offset)?;
                self.offset = self.offset.checked_add(1)?;
                EntryValue::Bool(b != 0)
            }
            kind::NULL => EntryValue::Null,
            _ => return None,
        };

        Some(Entry { key, value })
    }
}

// `FromHostBytes` lets the generic snapshot cache construct `Params` from the raw bytes
// the host writes. The wasm32 public API + the test module are the only consumers; native
// non-test builds don't pull in the cache machinery at all.
#[cfg(any(target_arch = "wasm32", test))]
impl crate::snapshot_cache::FromHostBytes for Params {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_bytes(bytes)
    }
}

/// Latest snapshot delivered for this widget instance.
///
/// First call inside `init` fetches via `host_params_snapshot`.
/// Subsequent calls reuse the cached bytes until `host_params_version` changes;
/// at that point the old snapshot is moved into [`previous`] and the new one is fetched.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn current() -> Params {
    PARAMS_CACHE.with(|c| crate::snapshot_cache::current_using(&WasmHost, &mut c.borrow_mut()))
}

/// Snapshot delivered immediately before [`current`].
///
/// [`Params::default`] until at least one update has been observed (i.e. during `init`
/// and the first `render` of any widget life).
///
/// Inside `on_params_update`, holds the just-replaced snapshot — diff against [`current`]
/// to react only to keys whose value actually changed.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn previous() -> Params {
    PARAMS_CACHE.with(|c| crate::snapshot_cache::previous_using(&WasmHost, &mut c.borrow_mut()))
}

// ── Native-target stubs ─────────────────────────────────────────────
// On native targets the host imports don't exist; SDK tests use `Params::from_bytes` directly,
// and the cache logic is exercised via `current_using` / `previous_using` with a mock host.
// The `current` / `previous` entry points return empty snapshots so non-wasm consumers of the
// SDK API (storybook etc.) still compile and behave consistently.

#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn current() -> Params {
    Params::default()
}

#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn previous() -> Params {
    Params::default()
}

/// Monotonic version of the latest params snapshot the host has delivered.
/// Cheap host call (no buffer copy); use it to gate per-frame re-parses.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn version() -> u64 {
    <WasmHost as crate::snapshot_cache::HostSnapshotProvider>::version(&WasmHost)
}

/// Non-wasm stub; widgets only run on wasm but the crate compiles for
/// native targets in tests / docs.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn version() -> u64 {
    0
}

// ── Wasm host bindings ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Probe-then-allocate snapshot reader.
    /// `out_cap == 0` returns required byte length without writing;
    /// `out_cap >= required` writes and returns bytes written;
    /// `out_cap < required` returns required length so the caller can retry with a larger buffer.
    fn host_params_snapshot(out_ptr: *mut u8, out_cap: u32) -> u32;

    /// Opaque change marker for the current host-side snapshot.
    /// Different value from last read = re-fetch; equal = use cached bytes.
    fn host_params_version() -> u64;
}

/// Wasm-target [`crate::snapshot_cache::HostSnapshotProvider`] for the params channel —
/// wraps the `host_params_*` externs.
#[cfg(target_arch = "wasm32")]
struct WasmHost;

#[cfg(target_arch = "wasm32")]
impl crate::snapshot_cache::HostSnapshotProvider for WasmHost {
    fn version(&self) -> u64 {
        // SAFETY: `host_params_version` has no out-params and is safe to call.
        unsafe { host_params_version() }
    }

    fn fill_snapshot(&self, out: &mut [u8]) -> usize {
        let cap = u32::try_from(out.len())
            .expect("BUG: snapshot buffer length must fit in u32 (wire-format guarantee)");
        let written = if out.is_empty() {
            // SAFETY: passing a null pointer is sound when `out_cap == 0` — the host implementation
            // explicitly checks the cap before writing and returns the required length without
            // touching the pointer.
            unsafe { host_params_snapshot(core::ptr::null_mut(), 0) }
        } else {
            // SAFETY: `out` is uniquely borrowed with length `cap`; the host writes at most
            // `out_cap` bytes starting at `out_ptr`.
            unsafe { host_params_snapshot(out.as_mut_ptr(), cap) }
        };
        usize::try_from(written).expect("BUG: host_params_snapshot return must fit in usize")
    }
}

// ── Cache state ─────────────────────────────────────────────────────
// Single-threaded wasm32 guest, so a plain `RefCell` inside `thread_local!` is sound and the
// borrow checker enforces re-entrancy is impossible.
// The host serialises guest calls, so there's no concurrent access.

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static PARAMS_CACHE: core::cell::RefCell<crate::snapshot_cache::Cache<Params>> =
        core::cell::RefCell::new(crate::snapshot_cache::Cache::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a packed buffer from an iterator of (key, value) pairs
    /// in the same wire format the host will produce.
    ///
    /// Used by the unit tests to exercise the parser end-to-end
    /// without the host plumbing.
    struct PackedBuilder {
        out: Vec<u8>,
        count: u32,
    }

    impl PackedBuilder {
        fn new() -> Self {
            // Reserve space for the count header.
            Self {
                out: vec![0; 4],
                count: 0,
            }
        }

        fn push_key(&mut self, kind: u8, key: &str) {
            self.count += 1;
            self.out.push(kind);
            let key_len = u16::try_from(key.len())
                .expect("BUG: test fixtures always use keys well under 64 KiB");
            self.out.extend_from_slice(&key_len.to_le_bytes());
            self.out.extend_from_slice(key.as_bytes());
        }

        fn str(mut self, key: &str, value: &str) -> Self {
            self.push_key(kind::STR, key);
            let len = u32::try_from(value.len())
                .expect("BUG: test fixtures always use values well under 4 GiB");
            self.out.extend_from_slice(&len.to_le_bytes());
            self.out.extend_from_slice(value.as_bytes());
            self
        }

        fn i32(mut self, key: &str, value: i32) -> Self {
            self.push_key(kind::I32, key);
            self.out.extend_from_slice(&value.to_le_bytes());
            self
        }

        fn f64(mut self, key: &str, value: f64) -> Self {
            self.push_key(kind::F64, key);
            self.out.extend_from_slice(&value.to_le_bytes());
            self
        }

        fn bool(mut self, key: &str, value: bool) -> Self {
            self.push_key(kind::BOOL, key);
            self.out.push(u8::from(value));
            self
        }

        fn null(mut self, key: &str) -> Self {
            self.push_key(kind::NULL, key);
            self
        }

        fn build(mut self) -> Vec<u8> {
            let head = self.count.to_le_bytes();
            self.out[0..4].copy_from_slice(&head);
            self.out
        }
    }

    #[test]
    fn default_params_is_empty() {
        let p = Params::default();
        assert!(p.is_empty());
        assert_eq!(p.keys().count(), 0);
        assert_eq!(p.get_str("x"), None);
        assert_eq!(p.get_i32("x"), None);
        assert_eq!(p.get_f64("x"), None);
        assert_eq!(p.get_bool("x"), None);
    }

    #[test]
    fn empty_header_is_empty() {
        let p = Params::from_bytes(0_u32.to_le_bytes().to_vec());
        assert!(p.is_empty());
    }

    #[test]
    fn each_scalar_kind_round_trips() {
        let bytes = PackedBuilder::new()
            .str("label", "hello")
            .i32("count", -7)
            .f64("ratio", 2.5)
            .bool("active", true)
            .null("optional")
            .build();

        let p = Params::from_bytes(bytes);

        assert!(!p.is_empty());
        assert_eq!(p.get_str("label"), Some("hello"));
        assert_eq!(p.get_i32("count"), Some(-7));
        assert_eq!(p.get_f64("ratio"), Some(2.5));
        assert_eq!(p.get_bool("active"), Some(true));
    }

    #[test]
    fn null_entries_are_visible_to_keys_but_invisible_to_typed_accessors() {
        let bytes = PackedBuilder::new()
            .null("optional")
            .str("present", "yes")
            .build();
        let p = Params::from_bytes(bytes);

        let keys: Vec<&str> = p.keys().collect();
        assert_eq!(keys, vec!["optional", "present"]);

        assert_eq!(p.get_str("optional"), None);
        assert_eq!(p.get_i32("optional"), None);
        assert_eq!(p.get_f64("optional"), None);
        assert_eq!(p.get_bool("optional"), None);
        assert_eq!(p.get_str("present"), Some("yes"));
    }

    #[test]
    fn wrong_kind_returns_none() {
        let bytes = PackedBuilder::new()
            .str("label", "hello")
            .i32("count", 42)
            .build();
        let p = Params::from_bytes(bytes);

        assert_eq!(p.get_i32("label"), None);
        assert_eq!(p.get_str("count"), None);
        assert_eq!(p.get_bool("label"), None);
        assert_eq!(p.get_f64("count"), None);
    }

    #[test]
    fn missing_key_returns_none() {
        let bytes = PackedBuilder::new().str("present", "yes").build();
        let p = Params::from_bytes(bytes);
        assert_eq!(p.get_str("absent"), None);
        assert_eq!(p.get_i32("absent"), None);
        assert_eq!(p.get_f64("absent"), None);
        assert_eq!(p.get_bool("absent"), None);
    }

    #[test]
    fn keys_iterates_in_packed_order() {
        let bytes = PackedBuilder::new()
            .str("z", "")
            .str("a", "")
            .str("m", "")
            .build();
        let p = Params::from_bytes(bytes);
        let keys: Vec<&str> = p.keys().collect();
        assert_eq!(keys, vec!["z", "a", "m"]);
    }

    #[test]
    fn truncated_buffer_stops_parser_gracefully() {
        let mut bytes = PackedBuilder::new().str("label", "hello").build();
        // Drop the last 3 bytes of the value — the parser should not panic.
        bytes.truncate(bytes.len() - 3);
        let p = Params::from_bytes(bytes);
        assert_eq!(p.get_str("label"), None);
    }

    #[test]
    fn unknown_kind_stops_parser_gracefully() {
        // Count = 1, kind = 99 (unknown).
        let bytes = vec![1, 0, 0, 0, 99, 1, 0, b'x'];
        let p = Params::from_bytes(bytes);
        assert_eq!(p.keys().count(), 0);
    }

    #[test]
    fn non_utf8_key_stops_parser_gracefully() {
        // Count = 1, kind = STR, key_len = 1, key = 0xFF (invalid UTF-8).
        let bytes = vec![1, 0, 0, 0, kind::STR, 1, 0, 0xFF, 0, 0, 0, 0];
        let p = Params::from_bytes(bytes);
        assert_eq!(p.keys().count(), 0);
    }

    #[test]
    fn clone_is_cheap_and_preserves_content() {
        let bytes = PackedBuilder::new()
            .str("label", "hello")
            .i32("count", 42)
            .build();
        let p = Params::from_bytes(bytes);
        let p2 = p.clone();
        assert_eq!(p.get_str("label"), p2.get_str("label"));
        assert_eq!(p.get_i32("count"), p2.get_i32("count"));
    }

    #[test]
    fn current_and_previous_are_default_on_native() {
        // On `wasm32`, [`current`] and [`previous`] call the host imports.
        // On native (this test target), those imports don't exist; the native fallback returns
        // [`Params::default`] so non-wasm consumers of the SDK API compile and behave consistently.
        // End-to-end coverage of the wasm path lives in `bmc-wasm-runtime` integration tests.
        assert!(current().is_empty());
        assert!(previous().is_empty());
    }

    /// Test-side fake of [`crate::snapshot_cache::HostSnapshotProvider`].
    /// Holds a current `(version, snapshot)` pair the test sets up; the SDK's cache logic
    /// pulls from it via the trait without touching wasm imports.
    struct MockHost {
        version: u64,
        snapshot: Vec<u8>,
    }

    impl MockHost {
        fn new() -> Self {
            Self {
                version: 0,
                snapshot: Vec::new(),
            }
        }

        fn set(&mut self, version: u64, snapshot: Vec<u8>) {
            self.version = version;
            self.snapshot = snapshot;
        }
    }

    impl crate::snapshot_cache::HostSnapshotProvider for MockHost {
        fn version(&self) -> u64 {
            self.version
        }

        fn fill_snapshot(&self, out: &mut [u8]) -> usize {
            if out.is_empty() {
                // Probe path: return required byte length without writing.
                return self.snapshot.len();
            }
            let to_write = self.snapshot.len().min(out.len());
            out[..to_write].copy_from_slice(&self.snapshot[..to_write]);
            to_write
        }
    }

    #[test]
    fn previous_first_after_version_bump_returns_just_replaced_snapshot() {
        // Channel-specific regression test for the BDK-432 #1 bug, exercised end-to-end through
        // the params wire format. The companion test in `snapshot_cache::tests` covers the
        // generic rotation logic; this one pins the Params type's parser-side decoding against
        // the same scenario.
        use crate::snapshot_cache::{Cache, current_using, previous_using};

        let mut host = MockHost::new();
        let mut cache = Cache::<Params>::new();

        // Seed version 1 by reading through `current_using`.
        host.set(1, PackedBuilder::new().str("a", "v1").build());
        let cur_v1 = current_using(&host, &mut cache);
        assert_eq!(cur_v1.get_str("a"), Some("v1"));
        assert!(
            previous_using(&host, &mut cache).is_empty(),
            "before any rotation, previous is empty"
        );

        // Host bumps to version 2. Critical: read `previous` FIRST.
        host.set(2, PackedBuilder::new().str("a", "v2").build());
        let prev_after_bump = previous_using(&host, &mut cache);
        let cur_after_bump = current_using(&host, &mut cache);

        assert_eq!(
            prev_after_bump.get_str("a"),
            Some("v1"),
            "previous() called before current() after a version bump must return the \
             just-replaced snapshot, not the version-before-previous (i.e. the empty default)"
        );
        assert_eq!(cur_after_bump.get_str("a"), Some("v2"));
    }
}
