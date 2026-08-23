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

//! Braiins Forge board serial number, factory-burned into the STM32 BSEC OTP.
//!
//! The 24-character serial `BFnnnnBxxyyzzBmmmmmmmmmm` (`nnnn` product id,
//! `xxyyzz` PCB version, `m…` device counter, `B` separators) is nibble-packed
//! into OTP words 60-62 by bos-factory (`devel-scripts/bcb_flash_emmc.sh`),
//! so the string is exactly the uppercase hex rendering of those 12 bytes.

use packed_struct::prelude::*;
use std::fmt::{self, Display};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Product identifier nibbles `0001` (Braiins Deck), as packed BCD.
pub const PRODUCT_DECK: u16 = 0x0001;

const SERIAL_PREFIX: u8 = 0xBF;
const SEPARATOR: u8 = 0xB;

const OTP_NVMEM_PATH: &str = "/sys/bus/nvmem/devices/stm32-romem0/nvmem";
/// The serial occupies OTP words 60-62; the nvmem file is addressed in bytes.
const OTP_SERIAL_OFFSET: u64 = 60 * 4;
const OTP_SERIAL_LEN: usize = 12;

/// PCB version `xxyyzz` as its three packed-BCD components.
///
/// Components stay in packed form (`yy` of `000200` is `0x02`);
/// ordering is lexicographic over `(xx, yy, zz)`, matching numeric order for valid BCD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PcbVersion {
    xx: u8,
    yy: u8,
    zz: u8,
}

impl PcbVersion {
    #[must_use]
    pub const fn new(xx: u8, yy: u8, zz: u8) -> Self {
        Self { xx, yy, zz }
    }
}

#[derive(Error, Debug)]
pub enum ParseSerialError {
    #[error("OTP serial area is blank (board not factory-provisioned)")]
    Blank,
    #[error("not a Braiins Forge serial: prefix {found:#04x}, expected {SERIAL_PREFIX:#04x}")]
    Prefix { found: u8 },
    #[error("malformed serial: 0xB separator nibbles missing")]
    Separators,
}

#[derive(Error, Debug)]
pub enum LoadSerialError {
    #[error("cannot read board serial from {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Parse(#[from] ParseSerialError),
}

/// The nibble layout behind `BFnnnnBxxyyzzBmmmmmmmmmm`.
#[derive(PackedStruct, Debug, Clone, Copy, PartialEq, Eq)]
#[packed_struct(size_bytes = "12", bit_numbering = "msb0", endian = "msb")]
struct RawSerial {
    #[packed_field(bits = "0..=7")]
    prefix: u8,
    #[packed_field(bits = "8..=23")]
    product: u16,
    #[packed_field(bits = "24..=27")]
    separator_product: Integer<u8, packed_bits::Bits<4>>,
    #[packed_field(bits = "28..=35")]
    version_xx: u8,
    #[packed_field(bits = "36..=43")]
    version_yy: u8,
    #[packed_field(bits = "44..=51")]
    version_zz: u8,
    #[packed_field(bits = "52..=55")]
    separator_version: Integer<u8, packed_bits::Bits<4>>,
    #[packed_field(bits = "56..=95")]
    device_counter: Integer<u64, packed_bits::Bits<40>>,
}

/// A parsed, validated Braiins Forge board serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardSerial {
    raw: [u8; OTP_SERIAL_LEN],
    product: u16,
    pcb_version: PcbVersion,
}

impl BoardSerial {
    pub fn parse(raw: [u8; OTP_SERIAL_LEN]) -> Result<Self, ParseSerialError> {
        if raw == [0; OTP_SERIAL_LEN] {
            return Err(ParseSerialError::Blank);
        }
        let unpacked =
            RawSerial::unpack(&raw).expect("BUG: fixed 12-byte layout has no unpack failures");
        if unpacked.prefix != SERIAL_PREFIX {
            return Err(ParseSerialError::Prefix {
                found: unpacked.prefix,
            });
        }
        if *unpacked.separator_product != SEPARATOR || *unpacked.separator_version != SEPARATOR {
            return Err(ParseSerialError::Separators);
        }
        Ok(Self {
            raw,
            product: unpacked.product,
            pcb_version: PcbVersion::new(
                unpacked.version_xx,
                unpacked.version_yy,
                unpacked.version_zz,
            ),
        })
    }

    /// Read and parse the serial from the STM32MP157 BSEC OTP nvmem device.
    pub fn load_stm32mp157() -> Result<Self, LoadSerialError> {
        Self::load_from(Path::new(OTP_NVMEM_PATH))
    }

    fn load_from(nvmem: &Path) -> Result<Self, LoadSerialError> {
        let mut raw = [0_u8; OTP_SERIAL_LEN];
        File::open(nvmem)
            .and_then(|mut file| {
                file.seek(SeekFrom::Start(OTP_SERIAL_OFFSET))?;
                file.read_exact(&mut raw)
            })
            .map_err(|source| LoadSerialError::Io {
                path: nvmem.to_owned(),
                source,
            })?;
        Ok(Self::parse(raw)?)
    }

    #[must_use]
    pub fn product(&self) -> u16 {
        self.product
    }

    #[must_use]
    pub fn pcb_version(&self) -> PcbVersion {
        self.pcb_version
    }
}

impl Display for BoardSerial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.raw {
            write!(f, "{byte:02X}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;

    /// OTP words 60-62 as read from a hubless Deck (10.0.0.129).
    const DECK_REV1: [u8; 12] = [
        0xBF, 0x00, 0x01, 0xB0, 0x00, 0x10, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    /// OTP words 60-62 as read from a hubbed Deck (10.37.50.118).
    const DECK_REV2: [u8; 12] = [
        0xBF, 0x00, 0x01, 0xB0, 0x00, 0x20, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn parses_rev1_hubless_deck_serial() {
        let serial = BoardSerial::parse(DECK_REV1).expect("BUG: real rev1 serial must parse");
        assert_eq!(serial.product(), PRODUCT_DECK);
        assert_eq!(serial.pcb_version(), PcbVersion::new(0, 1, 0));
    }

    #[test]
    fn parses_rev2_hubbed_deck_serial() {
        let serial = BoardSerial::parse(DECK_REV2).expect("BUG: real rev2 serial must parse");
        assert_eq!(serial.product(), PRODUCT_DECK);
        assert_eq!(serial.pcb_version(), PcbVersion::new(0, 2, 0));
    }

    #[test]
    fn renders_canonical_serial_string() {
        let serial = BoardSerial::parse(DECK_REV1).expect("BUG: real rev1 serial must parse");
        assert_eq!(serial.to_string(), "BF0001B000100B0000000000");
    }

    #[test]
    fn rejects_blank_otp() {
        assert!(matches!(
            BoardSerial::parse([0; 12]),
            Err(ParseSerialError::Blank)
        ));
    }

    #[test]
    fn rejects_non_forge_scheme() {
        let mut foreign = DECK_REV1;
        foreign[0] = 0xBC;
        assert!(matches!(
            BoardSerial::parse(foreign),
            Err(ParseSerialError::Prefix { found: 0xBC })
        ));
    }

    #[test]
    fn rejects_corrupt_separator() {
        let mut corrupt = DECK_REV1;
        corrupt[3] = 0x00;
        assert!(matches!(
            BoardSerial::parse(corrupt),
            Err(ParseSerialError::Separators)
        ));
    }

    #[test]
    fn pcb_version_orders_by_component() {
        assert!(PcbVersion::new(0, 1, 0) < PcbVersion::new(0, 2, 0));
        assert!(PcbVersion::new(0, 2, 0) < PcbVersion::new(0, 2, 1));
        assert!(PcbVersion::new(0, 0x10, 0) < PcbVersion::new(1, 0, 0));
    }

    #[test]
    fn loads_serial_from_otp_words_60_to_62() {
        let mut nvmem = tempfile::NamedTempFile::new().expect("BUG: create temp nvmem");
        let mut image = vec![0_u8; 380];
        image[240..252].copy_from_slice(&DECK_REV2);
        nvmem.write_all(&image).expect("BUG: write temp nvmem");
        let serial =
            BoardSerial::load_from(nvmem.path()).expect("BUG: serial at words 60-62 must load");
        assert_eq!(serial.pcb_version(), PcbVersion::new(0, 2, 0));
    }

    #[test]
    fn load_reports_io_error_on_truncated_nvmem() {
        let mut nvmem = tempfile::NamedTempFile::new().expect("BUG: create temp nvmem");
        nvmem
            .write_all(&[0_u8; 100])
            .expect("BUG: write temp nvmem");
        assert!(matches!(
            BoardSerial::load_from(nvmem.path()),
            Err(LoadSerialError::Io { .. })
        ));
    }
}
