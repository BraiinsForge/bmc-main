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

use super::{Params, ParamsSnapshotProvider};

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

pub(super) struct Host;

impl crate::snapshot_cache::HostSnapshotProvider for Host {
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

// Single-threaded wasm32 guest, so a plain `RefCell` inside `thread_local!` is sound and the
// borrow checker enforces re-entrancy is impossible.
// The host serialises guest calls, so there's no concurrent access.
std::thread_local! {
    static PARAMS_CACHE: core::cell::RefCell<crate::snapshot_cache::Cache<Params>> =
        core::cell::RefCell::new(crate::snapshot_cache::Cache::new());
}

impl ParamsSnapshotProvider for Host {
    fn current() -> Params {
        PARAMS_CACHE.with(|c| crate::snapshot_cache::current_using(&Host, &mut c.borrow_mut()))
    }

    fn previous() -> Params {
        PARAMS_CACHE.with(|c| crate::snapshot_cache::previous_using(&Host, &mut c.borrow_mut()))
    }

    fn snapshot_version() -> u64 {
        <Host as crate::snapshot_cache::HostSnapshotProvider>::version(&Host)
    }
}
