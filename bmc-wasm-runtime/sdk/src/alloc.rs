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

//! WASM memory allocator exports for host-to-WASM data transfer.
//!
//! The host calls `__alloc` to obtain a pointer into WASM linear memory,
//! writes data there, then passes the pointer to a callback export.
//! The SDK calls `__dealloc` when done reading the data.

/// Allocate `size` bytes of WASM memory and return the pointer,
/// or 0 if the heap cannot hold them.
///
/// A trap here would cut the guest stack, so a failed allocation is reported instead
/// and that payload never reaches the guest.
/// The pointer stays valid until `__dealloc` is called.
#[unsafe(no_mangle)]
pub extern "C" fn __alloc(size: u32) -> u32 {
    let mut buffer = Vec::<u8>::new();
    if buffer.try_reserve_exact(size as usize).is_err() {
        return 0;
    }
    let ptr = buffer.as_ptr() as u32;
    core::mem::forget(buffer);
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

#[cfg(test)]
mod tests {
    use super::{__alloc, __dealloc};
    use crate::wasm_test;

    #[wasm_test]
    fn a_size_beyond_the_address_space_is_reported_rather_than_trapping() {
        assert_eq!(
            __alloc(u32::MAX),
            0,
            "a trap here would cut the guest stack"
        );
    }

    #[wasm_test]
    fn a_size_the_heap_can_hold_gets_a_pointer() {
        let ptr = __alloc(16);

        assert_ne!(ptr, 0, "0 tells the host the payload did not fit");
        __dealloc(ptr, 16);
    }
}
