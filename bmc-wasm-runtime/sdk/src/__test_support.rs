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

//! What `#[wasm_test]` expands to on wasm32: a test's record, and the runner that reports its panics.

pub use bmc_wasm_test_abi::{IGNORED, SHOULD_PANIC, encode, record_len};

use crate::ffi_u32;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_test_panic(
        message_ptr: *const u8,
        message_len: u32,
        file_ptr: *const u8,
        file_len: u32,
        line: u32,
    );
}

/// Run a test, handing any panic to the test host
/// before the abort that follows it.
///
/// The hook replaces the SDK's widget panic hook,
/// whose log line would repeat the host's report.
pub fn run(test: fn()) {
    std::panic::set_hook(Box::new(|info| {
        let (file, line) = info.location().map_or(("?", 0), |l| (l.file(), l.line()));
        let message = info.payload_as_str().unwrap_or("panic");
        // SAFETY: both pointers stay valid for their lengths
        // for the duration of the call, which only reads them.
        unsafe {
            host_test_panic(
                message.as_ptr(),
                ffi_u32(message.len()),
                file.as_ptr(),
                ffi_u32(file.len()),
                line,
            );
        }
    }));
    test();
}

#[cfg(test)]
mod tests {
    use crate::wasm_test;

    macro_rules! stamped {
        ($module:ident) => {
            mod $module {
                use crate::wasm_test;

                #[wasm_test]
                fn holds() {}
            }
        };
    }

    stamped!(first);
    stamped!(second);

    #[wasm_test]
    fn run() {}
}
