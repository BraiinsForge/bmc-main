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
//! Each test exports a run function and a metadata function.
//! The metadata function returns the [`GuestSpan`] of the test's record,
//! and the record names two more spans: the test's name and the message `should_panic` expects.

use core::fmt;

/// Prefix of the export that runs a test, followed by the test's `module_path!()::name`.
///
/// Neither prefix starts the other, so no test path can make one export read as the other kind.
pub const RUN_EXPORT_PREFIX: &str = "__bmc_test_run:";
/// Prefix of the export that returns a test's record, followed by the test's `module_path!()::name`.
pub const META_EXPORT_PREFIX: &str = "__bmc_test_meta:";

pub const RECORD_VERSION: u32 = 1;
/// The test carries `#[ignore]`.
pub const IGNORED: u32 = 1 << 0;
/// The test carries `#[should_panic]`.
pub const SHOULD_PANIC: u32 = 1 << 1;

/// A record is six little-endian `u32` fields
/// and a `u64` fuel budget, at the [`offset`]s below.
pub const RECORD_LEN: usize = 32;

pub mod offset {
    pub const VERSION: usize = 0;
    pub const FLAGS: usize = 4;
    pub const NAME_PTR: usize = 8;
    pub const NAME_LEN: usize = 12;
    pub const EXPECTED_PTR: usize = 16;
    pub const EXPECTED_LEN: usize = 20;
    /// The test's own fuel budget,
    /// `0` when it takes the runner's.
    pub const FUEL: usize = 24;
}

/// Read a fuel budget: a whole number of units, written out or in exponent form (`250000000`, `250e6`, `2.5e8`).
#[must_use]
pub fn parse_fuel(text: &str) -> Option<u64> {
    /// 2^53, up to which `f64` holds every whole number exactly.
    const EXACT_IN_F64: f64 = 9_007_199_254_740_992.0;

    if let Ok(units) = text.parse() {
        return Some(units);
    }
    let units: f64 = text.parse().ok()?;
    if units.fract() != 0.0 || !(0.0..=EXACT_IN_F64).contains(&units) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a whole number in 0..=2^53, which u64 holds exactly"
    )]
    let units = units as u64;
    Some(units)
}

/// A range of guest memory, which crosses the boundary packed as `ptr | len << 32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestSpan {
    pub ptr: u32,
    pub len: u32,
}

impl GuestSpan {
    #[must_use]
    pub fn pack(self) -> u64 {
        u64::from(self.ptr) | (u64::from(self.len) << 32)
    }

    #[must_use]
    pub fn unpack(packed: u64) -> Self {
        let [p0, p1, p2, p3, l0, l1, l2, l3] = packed.to_le_bytes();
        Self {
            ptr: u32::from_le_bytes([p0, p1, p2, p3]),
            len: u32::from_le_bytes([l0, l1, l2, l3]),
        }
    }
}

/// The `N` bytes of `record` at `AT`; an offset past the record fails the build.
fn at<const AT: usize, const N: usize>(record: &[u8; RECORD_LEN]) -> [u8; N] {
    const { assert!(AT + N <= RECORD_LEN, "the field runs past the record") };
    let mut bytes = [0; N];
    bytes.copy_from_slice(&record[AT..AT + N]);
    bytes
}

fn u32_at<const AT: usize>(record: &[u8; RECORD_LEN]) -> u32 {
    u32::from_le_bytes(at::<AT, 4>(record))
}

fn u64_at<const AT: usize>(record: &[u8; RECORD_LEN]) -> u64 {
    u64::from_le_bytes(at::<AT, 8>(record))
}

/// One test's record, its strings still in guest memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestDescriptor {
    pub name: GuestSpan,
    pub ignored: bool,
    /// The message the panic must contain, for a `#[should_panic]` test.
    pub should_panic: Option<GuestSpan>,
    /// The fuel the test asked for, `None` to take the runner's budget.
    pub fuel: Option<u64>,
}

impl TestDescriptor {
    /// Read a record a guest built for [`RECORD_VERSION`].
    ///
    /// # Errors
    /// When the record carries another version, or a flag this version does not define.
    pub fn decode(record: &[u8; RECORD_LEN]) -> Result<Self, RecordError> {
        let version = u32_at::<{ offset::VERSION }>(record);
        if version != RECORD_VERSION {
            return Err(RecordError::UnknownVersion(version));
        }
        let flags = u32_at::<{ offset::FLAGS }>(record);
        let unknown = flags & !(IGNORED | SHOULD_PANIC);
        if unknown != 0 {
            return Err(RecordError::UnknownFlags(unknown));
        }
        let fuel = u64_at::<{ offset::FUEL }>(record);
        Ok(Self {
            name: GuestSpan {
                ptr: u32_at::<{ offset::NAME_PTR }>(record),
                len: u32_at::<{ offset::NAME_LEN }>(record),
            },
            ignored: flags & IGNORED != 0,
            should_panic: (flags & SHOULD_PANIC != 0).then(|| GuestSpan {
                ptr: u32_at::<{ offset::EXPECTED_PTR }>(record),
                len: u32_at::<{ offset::EXPECTED_LEN }>(record),
            }),
            fuel: (fuel != 0).then_some(fuel),
        })
    }
}

/// The run export paired with a metadata export, which share everything after their prefixes.
#[must_use]
pub fn run_export_for(meta_export: &str) -> Option<String> {
    let id = meta_export.strip_prefix(META_EXPORT_PREFIX)?;
    Some([RUN_EXPORT_PREFIX, id].concat())
}

/// The name libtest would give a test whose record holds `module_path!()::name`.
/// Libtest leaves the crate out, and cargo, nextest and rust-analyzer select tests by that name.
#[must_use]
pub fn libtest_name(recorded: &str) -> &str {
    recorded
        .split_once("::")
        .map_or(recorded, |(_crate, path)| path)
}

/// Why [`TestDescriptor::decode`] refused a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    UnknownVersion(u32),
    /// The flag bits this version does not define.
    UnknownFlags(u32),
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
        }
    }
}

impl core::error::Error for RecordError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(version: u32, flags: u32, fuel: u64) -> [u8; RECORD_LEN] {
        let mut bytes = [0_u8; RECORD_LEN];
        for (at, value) in [
            (offset::VERSION, version),
            (offset::FLAGS, flags),
            (offset::NAME_PTR, 64),
            (offset::NAME_LEN, 9),
            (offset::EXPECTED_PTR, 128),
            (offset::EXPECTED_LEN, 4),
        ] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[offset::FUEL..offset::FUEL + 8].copy_from_slice(&fuel.to_le_bytes());
        bytes
    }

    #[test]
    fn a_should_panic_record_decodes_its_expected_message() {
        assert_eq!(
            TestDescriptor::decode(&record(RECORD_VERSION, SHOULD_PANIC, 0)),
            Ok(TestDescriptor {
                name: GuestSpan { ptr: 64, len: 9 },
                ignored: false,
                should_panic: Some(GuestSpan { ptr: 128, len: 4 }),
                fuel: None,
            })
        );
    }

    #[test]
    fn a_record_without_should_panic_has_no_expected_message() {
        let descriptor = TestDescriptor::decode(&record(RECORD_VERSION, 0, 0))
            .expect("BUG: a current record decodes");

        assert_eq!(descriptor.should_panic, None);
    }

    #[test]
    fn an_ignored_record_reads_as_ignored() {
        let descriptor = TestDescriptor::decode(&record(RECORD_VERSION, IGNORED, 0))
            .expect("BUG: a current record decodes");

        assert!(descriptor.ignored);
    }

    #[test]
    fn a_record_with_a_flag_this_version_does_not_define_is_refused() {
        let unknown = 1 << 7;

        assert_eq!(
            TestDescriptor::decode(&record(RECORD_VERSION, IGNORED | unknown, 0)),
            Err(RecordError::UnknownFlags(unknown))
        );
    }

    #[test]
    fn a_record_carries_the_fuel_its_test_asked_for() {
        let descriptor = TestDescriptor::decode(&record(RECORD_VERSION, 0, 300_000_000))
            .expect("BUG: a current record decodes");

        assert_eq!(descriptor.fuel, Some(300_000_000));
    }

    #[test]
    fn a_fuel_budget_reads_written_out_or_in_exponent_form() {
        assert_eq!(
            ["250000000", "250e6", "2.5e8"].map(parse_fuel),
            [Some(250_000_000); 3]
        );
    }

    #[test]
    fn a_fuel_budget_is_a_whole_number_of_units() {
        assert_eq!(
            ["2.5", "-1e3", "1e300", "NaN", "lots"].map(parse_fuel),
            [None; 5]
        );
    }

    #[test]
    fn a_record_of_another_version_is_refused() {
        assert_eq!(
            TestDescriptor::decode(&record(RECORD_VERSION + 1, 0, 0)),
            Err(RecordError::UnknownVersion(RECORD_VERSION + 1))
        );
    }

    #[test]
    fn a_span_survives_packing() {
        let span = GuestSpan {
            ptr: 0x0001_0040,
            len: 24,
        };

        assert_eq!(GuestSpan::unpack(span.pack()), span);
    }

    #[test]
    fn libtest_names_leave_the_crate_out() {
        assert_eq!(
            libtest_name("clock::screens::tests::digital"),
            "screens::tests::digital"
        );
    }

    #[test]
    fn a_metadata_export_names_its_run_export() {
        assert_eq!(
            run_export_for("__bmc_test_meta:clock::tests::digital").as_deref(),
            Some("__bmc_test_run:clock::tests::digital")
        );
    }
}
