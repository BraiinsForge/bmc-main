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

//! Which of the widget's credential slots the operator has bound.
//!
//! A widget never receives the secret itself.
//! It embeds the placeholder from its codegen'd accessors
//! (`credentials::pool::TOKEN`) into a request,
//! and the host substitutes the value on the way out.
//!
//! This module answers only *"is this slot usable, and whose account is it"*
//! — enough to render a sign-in prompt instead of a broken panel,
//! and to name the account in the UI.
//!
//! Read it on the render path.
//! The host invokes an exported `on_credentials_update` immediately
//! when a binding changes, but delivery and hook invocation do not repaint.
//! Call `request_frame()` or `request_frame_after()`
//! when the change affects visible output.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// One slot's bound account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bound {
    /// Credential-type id the slot resolved to, e.g. `braiins-pool`.
    pub type_id: String,
    /// Operator-chosen account name, for display.
    pub account_name: String,
}

/// The widget's bound slots. A slot missing from the snapshot is unbound.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    slots: Vec<(String, Bound)>,
}

impl Snapshot {
    #[must_use]
    pub fn is_bound(&self, slot: &str) -> bool {
        self.lookup(slot).is_some()
    }

    #[must_use]
    pub fn get(&self, slot: &str) -> Option<&Bound> {
        self.lookup(slot)
    }

    /// Every bound slot, in the order the host encoded them.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Bound)> {
        self.slots
            .iter()
            .map(|(slot, bound)| (slot.as_str(), bound))
    }

    fn lookup(&self, slot: &str) -> Option<&Bound> {
        self.slots
            .iter()
            .find(|(name, _)| name == slot)
            .map(|(_, bound)| bound)
    }

    /// Parse the host's packed buffer: a `u32` slot count,
    /// then per slot its length-prefixed name, type id and account name.
    ///
    /// A truncated buffer yields the slots decoded so far.
    /// The host is the sole producer, so a short read is a host bug
    /// rather than operator input, and dropping the tail
    /// keeps the widget rendering instead of trapping on it.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut slots = Vec::new();
        let Some(count) = read_u32(bytes, 0) else {
            return Self { slots };
        };

        let mut at = 4;
        for _ in 0..count {
            let Some((slot, next)) = read_str(bytes, at) else {
                break;
            };
            let Some((type_id, next)) = read_str(bytes, next) else {
                break;
            };
            let Some((account_name, next)) = read_str(bytes, next) else {
                break;
            };
            at = next;
            slots.push((
                slot,
                Bound {
                    type_id,
                    account_name,
                },
            ));
        }

        Self { slots }
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

/// Read one length-prefixed UTF-8 string, returning it and the next offset.
fn read_str(bytes: &[u8], at: usize) -> Option<(String, usize)> {
    let raw: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
    let len = u16::from_le_bytes(raw) as usize;
    let start = at + 2;
    let text = core::str::from_utf8(bytes.get(start..start + len)?).ok()?;

    Some((String::from(text), start + len))
}

#[cfg(any(target_arch = "wasm32", test))]
impl crate::snapshot_cache::FromHostBytes for Snapshot {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_bytes(&bytes)
    }
}

/// Latest credential resolution for this widget instance.
///
/// The first call inside `init` fetches via `host_credentials_snapshot`.
/// Later calls reuse the cached bytes
/// until `host_credentials_version` changes.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn current() -> Snapshot {
    CREDENTIALS_CACHE.with(|c| crate::snapshot_cache::current_using(&WasmHost, &mut c.borrow_mut()))
}

/// Resolution delivered immediately before [`current`].
///
/// Inside `on_credentials_update` this holds the just-replaced snapshot,
/// so a widget can tell a fresh binding from a rotated account.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn previous() -> Snapshot {
    CREDENTIALS_CACHE
        .with(|c| crate::snapshot_cache::previous_using(&WasmHost, &mut c.borrow_mut()))
}

// ── Native-target stubs ─────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn current() -> Snapshot {
    Snapshot::default()
}

#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn previous() -> Snapshot {
    Snapshot::default()
}

// ── Wasm host bindings ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_credentials_snapshot(out_ptr: *mut u8, out_cap: u32) -> u32;
    fn host_credentials_version() -> u64;
}

#[cfg(target_arch = "wasm32")]
struct WasmHost;

#[cfg(target_arch = "wasm32")]
impl crate::snapshot_cache::HostSnapshotProvider for WasmHost {
    fn version(&self) -> u64 {
        // SAFETY: `host_credentials_version` has no out-params and is safe to call.
        unsafe { host_credentials_version() }
    }

    fn fill_snapshot(&self, out: &mut [u8]) -> usize {
        let cap = u32::try_from(out.len())
            .expect("BUG: snapshot buffer length must fit in u32 (wire-format guarantee)");
        let written = if out.is_empty() {
            // SAFETY: a null pointer is sound when `out_cap == 0` — the host
            // checks the cap before writing and returns the required length
            // without touching the pointer.
            unsafe { host_credentials_snapshot(core::ptr::null_mut(), 0) }
        } else {
            // SAFETY: `out` is uniquely borrowed with length `cap`;
            // the host writes at most `out_cap` bytes starting at `out_ptr`.
            unsafe { host_credentials_snapshot(out.as_mut_ptr(), cap) }
        };
        usize::try_from(written).expect("BUG: host_credentials_snapshot return must fit in usize")
    }
}

// ── Cache state ─────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static CREDENTIALS_CACHE: core::cell::RefCell<crate::snapshot_cache::Cache<Snapshot>> =
        core::cell::RefCell::new(crate::snapshot_cache::Cache::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same packing the host's `encode_view` produces.
    fn encode(slots: &[(&str, &str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(
            &u32::try_from(slots.len())
                .expect("BUG: test size")
                .to_le_bytes(),
        );
        for (slot, type_id, account) in slots {
            for field in [slot, type_id, account] {
                let bytes = field.as_bytes();
                out.extend_from_slice(
                    &u16::try_from(bytes.len())
                        .expect("BUG: test size")
                        .to_le_bytes(),
                );
                out.extend_from_slice(bytes);
            }
        }
        out
    }

    #[test]
    fn an_unbound_widget_decodes_to_an_empty_snapshot() {
        let snapshot = Snapshot::from_bytes(&encode(&[]));

        assert!(!snapshot.is_bound("pool"));
        assert_eq!(snapshot.get("pool"), None);
    }

    #[test]
    fn a_bound_slot_reports_its_type_and_account() {
        let snapshot = Snapshot::from_bytes(&encode(&[("pool", "braiins-pool", "Můj účet")]));

        assert!(snapshot.is_bound("pool"));
        assert_eq!(
            snapshot.get("pool"),
            Some(&Bound {
                type_id: String::from("braiins-pool"),
                account_name: String::from("Můj účet"),
            })
        );
    }

    #[test]
    fn an_unlisted_slot_reads_as_unbound() {
        let snapshot = Snapshot::from_bytes(&encode(&[("pool", "braiins-pool", "Mine")]));

        assert!(!snapshot.is_bound("spare"));
    }

    #[test]
    fn a_truncated_buffer_keeps_the_slots_decoded_so_far() {
        let mut bytes = encode(&[
            ("pool", "braiins-pool", "Mine"),
            ("spare", "generic-token", "T"),
        ]);
        bytes.truncate(bytes.len() - 4);

        let snapshot = Snapshot::from_bytes(&bytes);

        assert!(
            snapshot.is_bound("pool"),
            "the intact leading slot survives"
        );
        assert!(
            !snapshot.is_bound("spare"),
            "the cut trailing slot is dropped"
        );
    }
}
