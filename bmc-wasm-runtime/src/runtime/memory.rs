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

//! Guest memory helpers for the WASM runtime.

#![expect(clippy::cast_possible_truncation)]

use wasmi::{Caller, Extern};

use crate::host_api::HostState;

/// Resolve `[ptr .. ptr + len]` against `data_len`, rejecting overflow and
/// out-of-bounds in one place. `usize == u32` on armv7 makes the naive
/// `start + len` wrap on guest-controlled values; `checked_add` catches that.
fn bounded_range(ptr: u32, len: u32, data_len: usize) -> Option<core::ops::Range<usize>> {
    let start = ptr as usize;
    let end = start.checked_add(len as usize)?;
    (end <= data_len).then_some(start..end)
}

/// Read a UTF-8 string from WASM memory.
pub(super) fn read_string(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<String> {
    let memory = caller.get_export("memory").and_then(Extern::into_memory)?;
    let data = memory.data(caller);
    let range = bounded_range(ptr, len, data.len())?;
    String::from_utf8(data[range].to_vec()).ok()
}

/// Read raw bytes from WASM memory.
pub(super) fn read_bytes(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    let memory = caller.get_export("memory").and_then(Extern::into_memory)?;
    let data = memory.data(caller);
    let range = bounded_range(ptr, len, data.len())?;
    Some(data[range].to_vec())
}

/// Read optional bytes from WASM memory (returns `None` if ptr is null / len is 0).
pub(super) fn read_optional_bytes(
    caller: &Caller<'_, HostState>,
    ptr: u32,
    len: u32,
) -> Option<Vec<u8>> {
    if ptr == 0 || len == 0 {
        return None;
    }
    read_bytes(caller, ptr, len)
}

/// Parse newline-separated "Key: Value" headers from WASM memory.
pub(super) fn parse_headers(
    caller: &Caller<'_, HostState>,
    ptr: u32,
    len: u32,
) -> Vec<(String, String)> {
    if len == 0 {
        return Vec::new();
    }
    let Some(raw) = read_string(caller, ptr, len) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect()
}

/// Write a UTF-8 string into WASM memory at `out_ptr`, returning actual byte length.
/// Negative return on error (no memory export).
#[expect(clippy::cast_possible_wrap)]
pub(super) fn write_to_wasm(
    caller: &mut Caller<'_, HostState>,
    s: &str,
    out_ptr: u32,
    out_len: u32,
) -> i32 {
    let bytes = s.as_bytes();
    let actual_len = bytes.len();
    let copy_len = actual_len.min(out_len as usize);

    if copy_len > 0 {
        let memory = caller.get_export("memory").and_then(Extern::into_memory);
        if let Some(memory) = memory {
            let data = memory.data_mut(caller);
            // `copy_len` already fits `out_len: u32`, but `bounded_range`
            // still enforces `start + copy_len` doesn't wrap on armv7.
            #[expect(clippy::cast_possible_truncation, reason = "copy_len ≤ out_len: u32")]
            if let Some(range) = bounded_range(out_ptr, copy_len as u32, data.len()) {
                data[range].copy_from_slice(&bytes[..copy_len]);
            }
        }
    }

    actual_len as i32
}

/// Allocate guest memory for `bytes` and copy them into the guest heap.
pub(super) fn alloc_and_copy_to_guest(
    instance: wasmi::Instance,
    store: &mut wasmi::Store<HostState>,
    alloc_func: wasmi::TypedFunc<u32, u32>,
    fuel_per_frame: u64,
    bytes: &[u8],
    context: &str,
) -> Option<(u32, u32)> {
    let len = u32::try_from(bytes.len()).ok()?;
    if len == 0 {
        return Some((0, 0));
    }

    if let Err(e) = store.set_fuel(fuel_per_frame) {
        tracing::error!("set_fuel failed for {context}: {e}");
        return None;
    }

    let ptr = match alloc_func.call(&mut *store, len) {
        Ok(ptr) => ptr,
        Err(e) => {
            tracing::error!("__alloc failed for {context}: {e}");
            return None;
        }
    };

    let memory = instance
        .get_export(&*store, "memory")
        .and_then(Extern::into_memory);
    let Some(memory) = memory else {
        tracing::error!("memory export missing for {context}");
        return None;
    };

    let mem_data = memory.data_mut(store);
    let Some(range) = bounded_range(ptr, len, mem_data.len()) else {
        tracing::error!(
            ptr,
            len,
            mem_len = mem_data.len(),
            "guest memory too small for {context}"
        );
        return None;
    };
    mem_data[range].copy_from_slice(bytes);
    Some((ptr, len))
}
