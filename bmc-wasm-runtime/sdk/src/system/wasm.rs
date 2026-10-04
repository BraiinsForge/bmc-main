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

use super::{Snapshot, SystemSnapshotProvider};

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_system_snapshot(out_ptr: *mut u8, out_cap: u32) -> u32;
    fn host_system_version() -> u64;
}

pub(super) struct Host;

impl crate::snapshot_cache::HostSnapshotProvider for Host {
    fn version(&self) -> u64 {
        // SAFETY: `host_system_version` has no out-params and is safe to call.
        unsafe { host_system_version() }
    }

    fn fill_snapshot(&self, out: &mut [u8]) -> usize {
        let cap = u32::try_from(out.len())
            .expect("BUG: snapshot buffer length must fit in u32 (wire-format guarantee)");
        let written = if out.is_empty() {
            // SAFETY: passing a null pointer is sound when `out_cap == 0` —
            // the host implementation explicitly checks the cap before writing
            // and returns the required length without touching the pointer.
            unsafe { host_system_snapshot(core::ptr::null_mut(), 0) }
        } else {
            // SAFETY: `out` is uniquely borrowed with length `cap`;
            // the host writes at most `out_cap` bytes starting at `out_ptr`.
            unsafe { host_system_snapshot(out.as_mut_ptr(), cap) }
        };
        usize::try_from(written).expect("BUG: host_system_snapshot return must fit in usize")
    }
}

std::thread_local! {
    static SYSTEM_CACHE: core::cell::RefCell<crate::snapshot_cache::Cache<Snapshot>> =
        core::cell::RefCell::new(crate::snapshot_cache::Cache::new());
}

impl SystemSnapshotProvider for Host {
    fn current() -> Snapshot {
        SYSTEM_CACHE.with(|c| crate::snapshot_cache::current_using(&Host, &mut c.borrow_mut()))
    }

    fn previous() -> Snapshot {
        SYSTEM_CACHE.with(|c| crate::snapshot_cache::previous_using(&Host, &mut c.borrow_mut()))
    }
}
