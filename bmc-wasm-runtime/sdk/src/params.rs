// Copyright (C) 2026  Braiins Systems s.r.o.

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

/// Wire-format kind discriminators.
mod kind {
    pub const STR: u8 = 0;
    pub const I32: u8 = 1;
    pub const F64: u8 = 2;
    pub const BOOL: u8 = 3;
    pub const NULL: u8 = 4;
}

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
        self.offset += 1;

        let key_len = u16::from_le_bytes(
            *self
                .bytes
                .get(self.offset..self.offset + 2)?
                .first_chunk::<2>()?,
        ) as usize;
        self.offset += 2;

        let key_bytes = self.bytes.get(self.offset..self.offset + key_len)?;
        let key = core::str::from_utf8(key_bytes).ok()?;
        self.offset += key_len;

        let value = match kind {
            kind::STR => {
                let str_len = u32::from_le_bytes(
                    *self
                        .bytes
                        .get(self.offset..self.offset + 4)?
                        .first_chunk::<4>()?,
                ) as usize;
                self.offset += 4;
                let s_bytes = self.bytes.get(self.offset..self.offset + str_len)?;
                let s = core::str::from_utf8(s_bytes).ok()?;
                self.offset += str_len;
                EntryValue::Str(s)
            }
            kind::I32 => {
                let bytes = self
                    .bytes
                    .get(self.offset..self.offset + 4)?
                    .first_chunk::<4>()?;
                self.offset += 4;
                EntryValue::I32(i32::from_le_bytes(*bytes))
            }
            kind::F64 => {
                let bytes = self
                    .bytes
                    .get(self.offset..self.offset + 8)?
                    .first_chunk::<8>()?;
                self.offset += 8;
                EntryValue::F64(f64::from_le_bytes(*bytes))
            }
            kind::BOOL => {
                let b = *self.bytes.get(self.offset)?;
                self.offset += 1;
                EntryValue::Bool(b != 0)
            }
            kind::NULL => EntryValue::Null,
            _ => return None,
        };

        Some(Entry { key, value })
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
    refresh_if_stale();
    cache_clone_current()
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
    cache_clone_previous()
}

// ── Native-target stubs ─────────────────────────────────────────────
// On native targets the host imports don't exist; SDK tests use `Params::from_bytes` directly.
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

// ── Host imports ────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
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

// ── Cache state ─────────────────────────────────────────────────────
// Single-threaded wasm32 guest, so a plain `RefCell` inside `thread_local!` is sound and the
// borrow checker enforces re-entrancy is impossible.
// The host serialises guest calls, so there's no concurrent access.

#[cfg(target_arch = "wasm32")]
struct ParamsCache {
    current: Params,
    previous: Params,
    /// Last observed value of `host_params_version()`.
    /// `None` before the first fetch — distinguishes "host returned 0" from "never fetched".
    last_seen_version: Option<u64>,
}

#[cfg(target_arch = "wasm32")]
impl ParamsCache {
    const fn new() -> Self {
        Self {
            current: Params {
                bytes: alloc::vec::Vec::new(),
            },
            previous: Params {
                bytes: alloc::vec::Vec::new(),
            },
            last_seen_version: None,
        }
    }
}

#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static PARAMS_CACHE: core::cell::RefCell<ParamsCache> = const { core::cell::RefCell::new(ParamsCache::new()) };
}

#[cfg(target_arch = "wasm32")]
fn refresh_if_stale() {
    // SAFETY: `host_params_version` has no out-params and is safe to call.
    let host_version = unsafe { host_params_version() };

    let needs_refresh = PARAMS_CACHE.with(|c| {
        let cache = c.borrow();
        cache.last_seen_version != Some(host_version)
    });
    if !needs_refresh {
        return;
    }

    // Probe the required byte length.
    // SAFETY: passing a null pointer is sound when `out_cap == 0` — the host implementation
    // explicitly checks the cap before writing and returns the required length without
    // touching the pointer.
    let needed = unsafe { host_params_snapshot(core::ptr::null_mut(), 0) };

    let mut buf = alloc::vec::Vec::with_capacity(needed as usize);
    buf.resize(needed as usize, 0);
    let written = if needed > 0 {
        // SAFETY: `buf` is uniquely owned with capacity `needed`; the host writes at most
        // `out_cap` bytes (= `needed`) starting at `out_ptr`.
        unsafe { host_params_snapshot(buf.as_mut_ptr(), needed) }
    } else {
        0
    };
    buf.truncate(written as usize);

    PARAMS_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        // Rotate: old `current` becomes the new `previous`.
        // `previous` from before this update is dropped — only one step of history is kept.
        let new_current = Params::from_bytes(buf);
        let old_current = core::mem::replace(&mut cache.current, new_current);
        cache.previous = old_current;
        cache.last_seen_version = Some(host_version);
    });
}

#[cfg(target_arch = "wasm32")]
fn cache_clone_current() -> Params {
    PARAMS_CACHE.with(|c| c.borrow().current.clone())
}

#[cfg(target_arch = "wasm32")]
fn cache_clone_previous() -> Params {
    PARAMS_CACHE.with(|c| c.borrow().previous.clone())
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
}
