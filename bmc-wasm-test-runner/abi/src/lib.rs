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

//! What a `#[wasm_test]` leaves in a wasm test binary, and how a host reads it.
//!
//! Each test exports a run function and links a record into the [`SECTION`] custom section.
//! The linker concatenates the records of every test,
//! so a host lists the tests from the module's bytes without entering it.

use core::fmt;

/// Prefix of the export that runs a test, followed by the test's `module_path!()::name`.
pub const RUN_EXPORT_PREFIX: &str = "__bmc_test_run:";
/// The custom section every test's record is linked into.
pub const SECTION: &str = "bmc_tests";

pub const RECORD_VERSION: u32 = 1;
/// The test carries `#[ignore]`.
pub const IGNORED: u32 = 1 << 0;
/// The test carries `#[should_panic]`.
pub const SHOULD_PANIC: u32 = 1 << 1;

/// A record opens with little-endian `u32` version and flags, a `u64` fuel budget (`0` for the runner's),
/// and the `u32` lengths of the name and the expected panic message, which follow inline.
const HEADER_LEN: usize = 24;

/// The length of the record [`encode`] builds for a test of `name` expecting `expected`.
#[must_use]
pub const fn record_len(name: &str, expected: &str) -> usize {
    HEADER_LEN + name.len() + expected.len()
}

/// A test's record, `N` bytes long as [`record_len`] gives it for the same strings.
///
/// `fuel` is the test's own budget, `None` to take the runner's.
#[must_use]
pub const fn encode<const N: usize>(
    name: &str,
    flags: u32,
    expected: &str,
    fuel: Option<u64>,
) -> [u8; N] {
    assert!(
        N == record_len(name, expected),
        "a record is `record_len` of its strings long"
    );
    let fuel = match fuel {
        Some(units) => units,
        None => 0,
    };
    let mut record = [0; N];
    let mut at = put(&mut record, 0, &RECORD_VERSION.to_le_bytes());
    at = put(&mut record, at, &flags.to_le_bytes());
    at = put(&mut record, at, &fuel.to_le_bytes());
    at = put(&mut record, at, &text_len(name).to_le_bytes());
    at = put(&mut record, at, &text_len(expected).to_le_bytes());
    assert!(at == HEADER_LEN, "the header fields fill HEADER_LEN");
    at = put(&mut record, at, name.as_bytes());
    put(&mut record, at, expected.as_bytes());
    record
}

/// Copy `bytes` into `record` at `at`, returning where they end.
const fn put(record: &mut [u8], at: usize, bytes: &[u8]) -> usize {
    let mut index = 0;
    while index < bytes.len() {
        record[at + index] = bytes[index];
        index += 1;
    }
    at + bytes.len()
}

const fn text_len(text: &str) -> u32 {
    assert!(
        text.len() <= u32::MAX as usize,
        "a test name or panic message fits u32"
    );
    #[expect(clippy::cast_possible_truncation, reason = "checked just above")]
    let len = text.len() as u32;
    len
}

/// One test's record, read back from a [`SECTION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestRecord<'a> {
    /// The test's `module_path!()::name`.
    pub name: &'a str,
    pub ignored: bool,
    /// The message the panic must contain, for a `#[should_panic]` test.
    pub should_panic: Option<&'a str>,
    /// The fuel the test asked for, `None` to take the runner's budget.
    pub fuel: Option<u64>,
}

/// Read every record in a [`SECTION`], in the order the linker laid them out.
///
/// # Errors
/// When a record is cut short, carries another version or a flag this version does not define,
/// or holds a string that is not UTF-8.
pub fn decode(mut section: &[u8]) -> Result<Vec<TestRecord<'_>>, RecordError> {
    let mut records = Vec::new();
    while !section.is_empty() {
        let (record, rest) = decode_one(section)?;
        records.push(record);
        section = rest;
    }
    Ok(records)
}

fn decode_one(bytes: &[u8]) -> Result<(TestRecord<'_>, &[u8]), RecordError> {
    let (version, bytes) = take(bytes)?;
    let version = u32::from_le_bytes(version);
    if version != RECORD_VERSION {
        return Err(RecordError::UnknownVersion(version));
    }
    let (flags, bytes) = take(bytes)?;
    let flags = u32::from_le_bytes(flags);
    let unknown = flags & !(IGNORED | SHOULD_PANIC);
    if unknown != 0 {
        return Err(RecordError::UnknownFlags(unknown));
    }
    let (fuel, bytes) = take(bytes)?;
    let fuel = u64::from_le_bytes(fuel);
    let (name_len, bytes) = take(bytes)?;
    let (expected_len, bytes) = take(bytes)?;
    let (name, bytes) = text(bytes, name_len)?;
    let (expected, bytes) = text(bytes, expected_len)?;
    let record = TestRecord {
        name,
        ignored: flags & IGNORED != 0,
        should_panic: (flags & SHOULD_PANIC != 0).then_some(expected),
        fuel: (fuel != 0).then_some(fuel),
    };
    Ok((record, bytes))
}

fn take<const N: usize>(bytes: &[u8]) -> Result<([u8; N], &[u8]), RecordError> {
    let (field, rest) = bytes.split_first_chunk().ok_or(RecordError::Truncated)?;
    Ok((*field, rest))
}

fn text(bytes: &[u8], len: [u8; 4]) -> Result<(&str, &[u8]), RecordError> {
    let len = usize::try_from(u32::from_le_bytes(len)).map_err(|_| RecordError::Truncated)?;
    let (text, rest) = bytes.split_at_checked(len).ok_or(RecordError::Truncated)?;
    let text = core::str::from_utf8(text).map_err(|_| RecordError::NotUtf8)?;
    Ok((text, rest))
}

/// The run export of the test whose record holds `recorded`.
#[must_use]
pub fn run_export(recorded: &str) -> String {
    [RUN_EXPORT_PREFIX, recorded].concat()
}

/// The name libtest would give a test whose record holds `module_path!()::name`.
/// Libtest leaves the crate out, and cargo, nextest and rust-analyzer select tests by that name.
#[must_use]
pub fn libtest_name(recorded: &str) -> &str {
    recorded
        .split_once("::")
        .map_or(recorded, |(_crate, path)| path)
}

/// Why [`decode`] refused a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    UnknownVersion(u32),
    /// The flag bits this version does not define.
    UnknownFlags(u32),
    Truncated,
    NotUtf8,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVersion(version) => write!(
                f,
                "test record version {version} is not {RECORD_VERSION}; rebuild the tests against this SDK"
            ),
            Self::UnknownFlags(bits) => write!(
                f,
                "test record carries flags {bits:#x} that version {RECORD_VERSION} does not define"
            ),
            Self::Truncated => f.write_str("a test record ends before its fields do"),
            Self::NotUtf8 => f.write_str("a test record holds a string that is not UTF-8"),
        }
    }
}

impl core::error::Error for RecordError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record `#[wasm_test]` would link for these strings.
    macro_rules! encoded {
        ($name:literal, $flags:expr, $expected:literal, $fuel:expr) => {{
            const N: usize = record_len($name, $expected);
            encode::<N>($name, $flags, $expected, $fuel)
        }};
    }

    #[test]
    fn a_record_decodes_to_what_it_was_built_from() {
        let record = encoded!(
            "probe::tests::panics",
            SHOULD_PANIC,
            "boom",
            Some(300_000_000)
        );

        assert_eq!(
            decode(&record),
            Ok(vec![TestRecord {
                name: "probe::tests::panics",
                ignored: false,
                should_panic: Some("boom"),
                fuel: Some(300_000_000),
            }])
        );
    }

    #[test]
    fn a_record_without_a_budget_takes_the_runner_budget() {
        let record = encoded!("probe::tests::slow", IGNORED, "", None);

        assert_eq!(
            decode(&record),
            Ok(vec![TestRecord {
                name: "probe::tests::slow",
                ignored: true,
                should_panic: None,
                fuel: None,
            }])
        );
    }

    #[test]
    fn records_the_linker_concatenated_decode_in_order() {
        let section = [
            encoded!("probe::a", 0, "", None).as_slice(),
            encoded!("probe::b", SHOULD_PANIC, "boom", None).as_slice(),
        ]
        .concat();

        let names: Vec<_> = decode(&section)
            .expect("BUG: concatenated records decode")
            .iter()
            .map(|record| record.name)
            .collect();

        assert_eq!(names, ["probe::a", "probe::b"]);
    }

    #[test]
    fn a_section_cut_short_is_refused() {
        let record = encoded!("probe::tests::panics", SHOULD_PANIC, "boom", None);

        for cut in [1, HEADER_LEN, record.len() - 1] {
            assert_eq!(
                decode(&record[..cut]),
                Err(RecordError::Truncated),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_record_of_another_version_is_refused() {
        let mut record = encoded!("probe::a", 0, "", None);
        record[..4].copy_from_slice(&(RECORD_VERSION + 1).to_le_bytes());

        assert_eq!(
            decode(&record),
            Err(RecordError::UnknownVersion(RECORD_VERSION + 1))
        );
    }

    #[test]
    fn a_record_with_a_flag_this_version_does_not_define_is_refused() {
        let unknown = 1 << 7;
        let record = encoded!("probe::a", IGNORED | unknown, "", None);

        assert_eq!(decode(&record), Err(RecordError::UnknownFlags(unknown)));
    }

    #[test]
    fn a_name_that_is_not_utf8_is_refused() {
        let mut record = encoded!("probe::a", 0, "", None);
        record[HEADER_LEN] = 0xff;

        assert_eq!(decode(&record), Err(RecordError::NotUtf8));
    }

    #[test]
    fn an_empty_section_holds_no_tests() {
        assert_eq!(decode(&[]), Ok(Vec::new()));
    }

    #[test]
    fn libtest_names_leave_the_crate_out() {
        assert_eq!(
            libtest_name("clock::screens::tests::digital"),
            "screens::tests::digital"
        );
    }

    #[test]
    fn a_record_names_its_run_export() {
        assert_eq!(
            run_export("clock::tests::digital"),
            "__bmc_test_run:clock::tests::digital"
        );
    }
}
