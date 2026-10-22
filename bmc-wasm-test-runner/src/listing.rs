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

//! The tests a `#[wasm_test]` binary records, read from its custom section.

use anyhow::{Context as _, Result};
use bmc_wasm_test_abi::{SECTION, decode, libtest_name, run_export};
use wasmparser::{Parser, Payload};

/// A test a module records, named as libtest would name it.
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

/// The tests whose records the module's bytes carry, read without entering it.
pub(crate) fn list_tests(wasm: &[u8]) -> Result<Vec<ListedTest>> {
    let mut section = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CustomSection(custom) = payload.context("parse the test binary")?
            && custom.name() == SECTION
        {
            section.extend_from_slice(custom.data());
        }
    }
    let records = decode(&section).with_context(|| format!("read the `{SECTION}` section"))?;
    let mut tests: Vec<ListedTest> = records
        .into_iter()
        .map(|record| ListedTest {
            name: libtest_name(record.name).to_owned(),
            run_export: run_export(record.name),
            ignored: record.ignored,
            should_panic: record.should_panic.map(str::to_owned),
            fuel: record.fuel,
        })
        .collect();
    tests.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(tests)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use bmc_wasm_test_abi::{IGNORED, SHOULD_PANIC, encode, record_len};

    use super::*;

    /// The record `#[wasm_test]` would link for these strings.
    macro_rules! encoded {
        ($name:literal, $flags:expr, $expected:literal, $fuel:expr) => {{
            const N: usize = record_len($name, $expected);
            encode::<N>($name, $flags, $expected, $fuel)
        }};
    }

    /// A module carrying `section` as its test records.
    fn module(section: &[u8]) -> Vec<u8> {
        let data = section.iter().fold(String::new(), |mut text, byte| {
            write!(text, "\\{byte:02x}").expect("BUG: writing to a String cannot fail");
            text
        });
        wat::parse_str(format!(
            r#"(module (@custom "{SECTION}" "{data}") (func (export "unrelated")))"#
        ))
        .expect("BUG: the test-module WAT must parse")
    }

    #[test]
    fn a_listed_test_carries_its_libtest_name_expected_panic_and_fuel() {
        let record = encoded!(
            "probe::tests::panics",
            SHOULD_PANIC,
            "boom",
            Some(300_000_000)
        );

        let tests = list_tests(&module(&record)).expect("BUG: a well-formed record lists");

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
    fn the_records_of_every_test_list_by_name() {
        let section = [
            encoded!("probe::tests::second", IGNORED, "", None).as_slice(),
            encoded!("probe::tests::first", 0, "", None).as_slice(),
        ]
        .concat();

        let tests = list_tests(&module(&section)).expect("BUG: well-formed records list");

        assert_eq!(
            tests
                .iter()
                .map(|test| (test.name.as_str(), test.ignored))
                .collect::<Vec<_>>(),
            [("tests::first", false), ("tests::second", true)]
        );
    }

    #[test]
    fn a_module_without_records_lists_nothing() {
        let wasm = wat::parse_str(r#"(module (func (export "passes")))"#)
            .expect("BUG: the test-module WAT must parse");

        let tests = list_tests(&wasm).expect("BUG: a module without records lists");

        assert!(tests.is_empty(), "{tests:?}");
    }

    #[test]
    fn a_record_cut_short_fails_the_listing() {
        let record = encoded!("probe::tests::panics", SHOULD_PANIC, "boom", None);

        let error = list_tests(&module(&record[..record.len() - 1]))
            .expect_err("BUG: a truncated record cannot list");

        assert_eq!(
            format!("{error:#}"),
            format!("read the `{SECTION}` section: a test record ends before its fields do")
        );
    }
}
