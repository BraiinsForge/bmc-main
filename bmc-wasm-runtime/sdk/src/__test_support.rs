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

use bmc_wasm_protocol::test_descriptor::{GuestSpan, RECORD_LEN, RECORD_VERSION, offset};
pub use bmc_wasm_protocol::test_descriptor::{IGNORED, SHOULD_PANIC};

/// A test's record, laid out as [`bmc_wasm_protocol::test_descriptor`] reads it.
#[derive(Debug)]
#[repr(C)]
pub struct TestRecord {
    version: u32,
    flags: u32,
    name_ptr: *const u8,
    name_len: usize,
    expected_ptr: *const u8,
    expected_len: usize,
    fuel: u64,
}

// SAFETY: a record never changes after construction,
// and its pointers are to `'static` strings.
unsafe impl Sync for TestRecord {}

const _: () = {
    assert!(size_of::<TestRecord>() == RECORD_LEN);
    assert!(core::mem::offset_of!(TestRecord, version) == offset::VERSION);
    assert!(core::mem::offset_of!(TestRecord, flags) == offset::FLAGS);
    assert!(core::mem::offset_of!(TestRecord, name_ptr) == offset::NAME_PTR);
    assert!(core::mem::offset_of!(TestRecord, name_len) == offset::NAME_LEN);
    assert!(core::mem::offset_of!(TestRecord, expected_ptr) == offset::EXPECTED_PTR);
    assert!(core::mem::offset_of!(TestRecord, expected_len) == offset::EXPECTED_LEN);
    assert!(core::mem::offset_of!(TestRecord, fuel) == offset::FUEL);
};

impl TestRecord {
    /// `fuel` is the test's own budget,
    /// `None` to take the runner's.
    #[must_use]
    pub const fn new(
        name: &'static str,
        flags: u32,
        expected: &'static str,
        fuel: Option<u64>,
    ) -> Self {
        Self {
            version: RECORD_VERSION,
            flags,
            name_ptr: name.as_ptr(),
            name_len: name.len(),
            expected_ptr: expected.as_ptr(),
            expected_len: expected.len(),
            fuel: match fuel {
                Some(units) => units,
                None => 0,
            },
        }
    }

    /// Where this record sits in guest memory,
    /// packed as a metadata export returns it.
    #[must_use]
    pub fn locate(&'static self) -> u64 {
        GuestSpan {
            ptr: wasm32_u32(core::ptr::from_ref(self).addr()),
            len: wasm32_u32(RECORD_LEN),
        }
        .pack()
    }
}

fn wasm32_u32(value: usize) -> u32 {
    u32::try_from(value).expect("BUG: wasm32 addresses and lengths fit in u32")
}

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
                wasm32_u32(message.len()),
                file.as_ptr(),
                wasm32_u32(file.len()),
                line,
            );
        }
    }));
    test();
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::test_descriptor::TestDescriptor;

    use super::*;
    use crate::wasm_test;

    fn span(text: &'static str) -> GuestSpan {
        GuestSpan {
            ptr: wasm32_u32(text.as_ptr().addr()),
            len: wasm32_u32(text.len()),
        }
    }

    fn decoded(record: &TestRecord) -> TestDescriptor {
        // SAFETY: the layout asserts above make a record exactly `RECORD_LEN` bytes
        // with no padding, so viewing it as a byte array reads every byte it holds.
        let bytes: &[u8; RECORD_LEN] = unsafe { &*core::ptr::from_ref(record).cast() };
        TestDescriptor::decode(bytes).expect("BUG: a record the SDK built decodes")
    }

    #[wasm_test]
    fn a_record_decodes_to_what_it_was_built_from() {
        const NAME: &str = "probe::tests::panics";
        const EXPECTED: &str = "boom";

        let record = TestRecord::new(NAME, SHOULD_PANIC, EXPECTED, Some(300_000_000));

        assert_eq!(
            decoded(&record),
            TestDescriptor {
                name: span(NAME),
                ignored: false,
                should_panic: Some(span(EXPECTED)),
                fuel: Some(300_000_000),
            }
        );
    }

    #[wasm_test]
    fn a_record_without_a_budget_decodes_to_the_runner_budget() {
        let record = TestRecord::new("probe::tests::ignored", IGNORED, "", None);

        let descriptor = decoded(&record);

        assert!(descriptor.ignored);
        assert_eq!(descriptor.should_panic, None);
        assert_eq!(descriptor.fuel, None);
    }

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
