// Copyright (C) 2026  Braiins Systems s.r.o.

//! Guest memory helpers for the WASM runtime.

#![expect(clippy::cast_possible_truncation)]

use wasmi::{Caller, Extern};

use crate::host_api::HostState;

/// Read a UTF-8 string from WASM memory.
pub(super) fn read_string(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<String> {
    let memory = caller.get_export("memory").and_then(Extern::into_memory)?;
    let data = memory.data(caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    String::from_utf8(data[start..end].to_vec()).ok()
}

/// Read raw bytes from WASM memory.
pub(super) fn read_bytes(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    let memory = caller.get_export("memory").and_then(Extern::into_memory)?;
    let data = memory.data(caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    Some(data[start..end].to_vec())
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
            let start = out_ptr as usize;
            if start + copy_len <= data.len() {
                data[start..start + copy_len].copy_from_slice(&bytes[..copy_len]);
            }
        }
    }

    actual_len as i32
}

/// Allocate guest memory for `bytes` and copy them into the guest heap.
pub(super) fn alloc_and_copy_to_guest(
    instance: &wasmi::Instance,
    store: &mut wasmi::Store<HostState>,
    alloc_func: &wasmi::TypedFunc<u32, u32>,
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
    let start = ptr as usize;
    let end = start + len as usize;
    if end > mem_data.len() {
        tracing::error!(
            ptr,
            len,
            mem_len = mem_data.len(),
            "guest memory too small for {context}"
        );
        return None;
    }
    mem_data[start..end].copy_from_slice(bytes);
    Some((ptr, len))
}
