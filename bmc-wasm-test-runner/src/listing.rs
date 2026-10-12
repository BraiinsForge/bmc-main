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

//! The tests a `#[wasm_test]` binary exports, read back from its records.

use anyhow::{Context as _, Result};
use bmc_wasm_protocol::test_descriptor::{
    GuestSpan, META_EXPORT_PREFIX, RECORD_LEN, TestDescriptor, libtest_name, run_export_for,
};
use bmc_wasm_runtime::hermetic_host::HermeticHost;
use bmc_wasm_runtime::platform_catalog::Target;
use bmc_wasm_runtime::{EntryPolicy, WasmWidgetRuntime};

/// A test a module exports, named as libtest would name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedTest {
    pub(crate) name: String,
    pub(crate) run_export: String,
    pub(crate) ignored: bool,
    /// `Some` when the test must panic; the panic message must contain it.
    pub(crate) should_panic: Option<String>,
    /// The test's own fuel budget, from `#[wasm_test(fuel = …)]`.
    pub(crate) fuel: Option<u64>,
}

/// The tests the module exports, read from one instance entered for the purpose.
///
/// A module without records is never entered:
/// a test binary that links no SDK has no tests,
/// and no handshake to enter it by.
pub(crate) fn list_tests(host: &HermeticHost, target: Target) -> Result<Vec<ListedTest>> {
    let meta_exports: Vec<String> = host
        .export_names()
        .into_iter()
        .filter(|name| name.starts_with(META_EXPORT_PREFIX))
        .collect();
    if meta_exports.is_empty() {
        return Ok(Vec::new());
    }
    let mut runtime = host.instantiate(EntryPolicy::TestModule, target, host.config()?)?;
    let mut tests = Vec::new();
    for meta_export in meta_exports {
        let span = runtime
            .call_export_i64(&meta_export)
            .map(|packed| GuestSpan::unpack(packed.cast_unsigned()))
            .with_context(|| format!("`{meta_export}` does not return a record"))?;
        let record: [u8; RECORD_LEN] = runtime
            .read_guest_memory(span.ptr, span.len)
            .and_then(|bytes| bytes.try_into().ok())
            .with_context(|| format!("`{meta_export}` points past its record"))?;
        let descriptor = TestDescriptor::decode(&record)?;
        let name = guest_string(&runtime, descriptor.name)
            .with_context(|| format!("`{meta_export}` names its test badly"))?;
        let should_panic = descriptor
            .should_panic
            .map(|expected| {
                guest_string(&runtime, expected)
                    .with_context(|| format!("`{meta_export}` expects a panic message badly"))
            })
            .transpose()?;
        tests.push(ListedTest {
            name: libtest_name(&name).to_owned(),
            run_export: run_export_for(&meta_export)
                .expect("BUG: listed under the metadata prefix"),
            ignored: descriptor.ignored,
            should_panic,
            fuel: descriptor.fuel,
        });
    }
    tests.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(tests)
}

fn guest_string(runtime: &WasmWidgetRuntime, span: GuestSpan) -> Option<String> {
    String::from_utf8(runtime.read_guest_memory(span.ptr, span.len)?).ok()
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use bmc_wasm_protocol::test_descriptor::{IGNORED, RECORD_VERSION, SHOULD_PANIC, offset};
    use bmc_wasm_runtime::platform_catalog;

    use super::*;

    fn prepared(wat: &str) -> HermeticHost {
        let wasm = wat::parse_str(wat).expect("BUG: the test-module WAT must parse");
        let directory = tempfile::tempdir().expect("BUG: a temporary directory is available");
        let path = directory.path().join("tests.wasm");
        std::fs::write(&path, wasm).expect("BUG: the temporary directory is writable");
        HermeticHost::prepare(&path).expect("BUG: a plain test module prepares")
    }

    /// A test module exporting `body`'s items beside the SDK handshake.
    fn host_for(body: &str) -> HermeticHost {
        prepared(&format!(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "__bmc_sdk_init") (result i64)
                i64.const {version})
              {body})
            "#,
            version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
        ))
    }

    fn target() -> Target {
        let platform = platform_catalog::default_platform();
        Target {
            platform,
            viewport: &platform.viewports[0],
        }
    }

    fn wat_bytes(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut text, byte| {
            write!(text, "\\{byte:02x}").expect("BUG: writing to a String cannot fail");
            text
        })
    }

    /// One `should_panic` test with a fuel budget of its own,
    /// as `#[wasm_test]` lays it out, its record carrying `flags`.
    fn should_panic_test_module(flags: u32) -> HermeticHost {
        let (name, expected) = ("probe::tests::panics", "boom");
        let length = |text: &str| u32::try_from(text.len()).expect("BUG: a short string");
        let mut record = [0_u8; RECORD_LEN];
        for (at, value) in [
            (offset::VERSION, RECORD_VERSION),
            (offset::FLAGS, flags),
            (offset::NAME_PTR, 512),
            (offset::NAME_LEN, length(name)),
            (offset::EXPECTED_PTR, 640),
            (offset::EXPECTED_LEN, length(expected)),
        ] {
            record[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        record[offset::FUEL..offset::FUEL + 8].copy_from_slice(&300_000_000_u64.to_le_bytes());
        let span = GuestSpan {
            ptr: 256,
            len: u32::try_from(RECORD_LEN).expect("BUG: a record fits u32"),
        };
        host_for(&format!(
            r#"
              (data (i32.const 256) "{record}")
              (data (i32.const 512) "{name}")
              (data (i32.const 640) "{expected}")
              (func (export "__bmc_test_meta:probe::tests::panics") (result i64)
                i64.const {packed})
              (func (export "__bmc_test_run:probe::tests::panics"))
            "#,
            record = wat_bytes(&record),
            packed = span.pack(),
        ))
    }

    #[test]
    fn a_listed_test_carries_its_libtest_name_expected_panic_and_fuel() {
        let tests = list_tests(&should_panic_test_module(SHOULD_PANIC), target())
            .expect("BUG: a well-formed record lists");

        assert_eq!(
            tests,
            [ListedTest {
                name: "tests::panics".to_owned(),
                run_export: "__bmc_test_run:probe::tests::panics".to_owned(),
                ignored: false,
                should_panic: Some("boom".to_owned()),
                fuel: Some(300_000_000),
            }]
        );
    }

    #[test]
    fn an_ignored_record_lists_an_ignored_test() {
        let tests = list_tests(&should_panic_test_module(SHOULD_PANIC | IGNORED), target())
            .expect("BUG: a well-formed record lists");

        assert!(
            tests.iter().all(|test| test.ignored),
            "the ignore flag reaches the trial: {tests:?}"
        );
    }

    #[test]
    fn a_module_without_records_lists_nothing() {
        let tests = list_tests(&host_for(r#"(func (export "passes"))"#), target())
            .expect("BUG: a module without records lists");

        assert!(tests.is_empty(), "{tests:?}");
    }

    #[test]
    fn a_module_without_the_sdk_lists_nothing() {
        let host = prepared(r#"(module (memory (export "memory") 1))"#);
        let tests = list_tests(&host, target()).expect("BUG: a module without the SDK lists");

        assert!(tests.is_empty(), "{tests:?}");
    }
}
