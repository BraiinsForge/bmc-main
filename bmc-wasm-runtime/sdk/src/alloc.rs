// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM memory allocator exports for host-to-WASM data transfer.
//!
//! The host calls `__alloc` to obtain a pointer into WASM linear memory,
//! writes data there, then passes the pointer to a callback export.
//! The SDK calls `__dealloc` when done reading the data.

/// Allocate `size` bytes of WASM memory and return the pointer.
///
/// Uses `Vec::with_capacity` + `leak` to get a stable pointer that
/// survives until `__dealloc` is called.
#[unsafe(no_mangle)]
pub extern "C" fn __alloc(size: u32) -> u32 {
    let layout = Vec::<u8>::with_capacity(size as usize);
    let ptr = layout.as_ptr() as u32;
    core::mem::forget(layout);
    ptr
}

/// Free memory previously allocated by `__alloc`.
///
/// # Safety
/// `ptr` must have been returned by `__alloc` with the same `size`.
#[unsafe(no_mangle)]
pub extern "C" fn __dealloc(ptr: u32, size: u32) {
    unsafe {
        let _ = Vec::from_raw_parts(ptr as *mut u8, 0, size as usize);
    }
}
