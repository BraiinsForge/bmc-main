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

use core::str;

use bmc_wasm_protocol::{
    PACKAGE_ASSET_FORMAT_FLAGS, PACKAGE_ASSET_FORMAT_VERSION, PACKAGE_ASSET_ID_LEN,
    PACKAGE_ASSET_RECORD_MAGIC, PackageAssetId, PackageAssetKind,
};
use thiserror::Error;

use crate::package_asset_id;

pub const MAX_PACKAGE_ASSET_PAYLOAD_LEN: usize = 24 * 1_024 * 1_024;
const RECORD_HEADER_LEN: usize = 8 + 2 + 1 + 1 + PACKAGE_ASSET_ID_LEN + 4 + 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordRef<'a> {
    pub kind: PackageAssetKind,
    pub id: PackageAssetId,
    pub logical_name: &'a str,
    pub payload: &'a [u8],
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RecordError {
    #[error("package asset record is truncated while reading {0}")]
    Truncated(&'static str),
    #[error("package asset record has invalid magic")]
    InvalidMagic,
    #[error("unsupported package asset format version {0}")]
    UnsupportedVersion(u16),
    #[error("unknown package asset kind {0}")]
    UnknownKind(u8),
    #[error("unsupported package asset flags {0:#04x}")]
    UnsupportedFlags(u8),
    #[error("package asset logical name length does not fit this platform")]
    LogicalNameLength,
    #[error("package asset payload length does not fit this platform")]
    PayloadLength,
    #[error("package asset payload is {actual} bytes; maximum is {maximum}")]
    PayloadTooLarge { actual: usize, maximum: usize },
    #[error("package asset record length overflows this platform")]
    RecordLengthOverflow,
    #[error("package asset logical name is not UTF-8")]
    LogicalNameUtf8,
    #[error("package asset digest does not match its kind and payload")]
    DigestMismatch,
    #[error("package asset logical name is too long for the record format")]
    EncodeLogicalNameLength,
}

#[derive(Clone, Debug)]
pub struct Records<'a> {
    remaining: &'a [u8],
    failed: bool,
}

impl<'a> Records<'a> {
    #[must_use]
    pub const fn new(section: &'a [u8]) -> Self {
        Self {
            remaining: section,
            failed: false,
        }
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<RecordRef<'a>, RecordError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.remaining.is_empty() {
            return None;
        }
        match parse_record(self.remaining) {
            Ok((record, consumed)) => {
                self.remaining = &self.remaining[consumed..];
                Some(Ok(record))
            }
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}

pub fn encode_record(
    kind: PackageAssetKind,
    logical_name: &str,
    payload: &[u8],
) -> Result<Vec<u8>, RecordError> {
    if payload.len() > MAX_PACKAGE_ASSET_PAYLOAD_LEN {
        return Err(RecordError::PayloadTooLarge {
            actual: payload.len(),
            maximum: MAX_PACKAGE_ASSET_PAYLOAD_LEN,
        });
    }
    let logical_name_len =
        u32::try_from(logical_name.len()).map_err(|_| RecordError::EncodeLogicalNameLength)?;
    let payload_len = u64::try_from(payload.len()).map_err(|_| RecordError::PayloadLength)?;
    let total_len = RECORD_HEADER_LEN
        .checked_add(logical_name.len())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or(RecordError::RecordLengthOverflow)?;
    let id = package_asset_id(kind, payload);
    let mut record = Vec::with_capacity(total_len);
    record.extend_from_slice(&PACKAGE_ASSET_RECORD_MAGIC);
    record.extend_from_slice(&PACKAGE_ASSET_FORMAT_VERSION.to_le_bytes());
    record.push(kind.to_wire());
    record.push(PACKAGE_ASSET_FORMAT_FLAGS);
    record.extend_from_slice(id.as_bytes());
    record.extend_from_slice(&logical_name_len.to_le_bytes());
    record.extend_from_slice(&payload_len.to_le_bytes());
    record.extend_from_slice(logical_name.as_bytes());
    record.extend_from_slice(payload);
    Ok(record)
}

pub(crate) fn parse_record(input: &[u8]) -> Result<(RecordRef<'_>, usize), RecordError> {
    let header = input
        .get(..RECORD_HEADER_LEN)
        .ok_or(RecordError::Truncated("header"))?;
    if header[..PACKAGE_ASSET_RECORD_MAGIC.len()] != PACKAGE_ASSET_RECORD_MAGIC {
        return Err(RecordError::InvalidMagic);
    }
    let version = u16::from_le_bytes([header[8], header[9]]);
    if version != PACKAGE_ASSET_FORMAT_VERSION {
        return Err(RecordError::UnsupportedVersion(version));
    }
    let kind = PackageAssetKind::from_wire(header[10])
        .map_err(|unknown| RecordError::UnknownKind(unknown.0))?;
    let flags = header[11];
    if flags != PACKAGE_ASSET_FORMAT_FLAGS {
        return Err(RecordError::UnsupportedFlags(flags));
    }
    let id = PackageAssetId::from_bytes(
        header[12..12 + PACKAGE_ASSET_ID_LEN]
            .try_into()
            .expect("BUG: fixed package asset ID header slice has the declared length"),
    );
    let name_offset = 12 + PACKAGE_ASSET_ID_LEN;
    let logical_name_len = usize::try_from(u32::from_le_bytes(
        header[name_offset..name_offset + 4]
            .try_into()
            .expect("BUG: fixed logical name length header slice has four bytes"),
    ))
    .map_err(|_| RecordError::LogicalNameLength)?;
    let payload_offset = name_offset + 4;
    let payload_len = usize::try_from(u64::from_le_bytes(
        header[payload_offset..payload_offset + 8]
            .try_into()
            .expect("BUG: fixed payload length header slice has eight bytes"),
    ))
    .map_err(|_| RecordError::PayloadLength)?;
    if payload_len > MAX_PACKAGE_ASSET_PAYLOAD_LEN {
        return Err(RecordError::PayloadTooLarge {
            actual: payload_len,
            maximum: MAX_PACKAGE_ASSET_PAYLOAD_LEN,
        });
    }
    let logical_name_end = RECORD_HEADER_LEN
        .checked_add(logical_name_len)
        .ok_or(RecordError::RecordLengthOverflow)?;
    let record_end = logical_name_end
        .checked_add(payload_len)
        .ok_or(RecordError::RecordLengthOverflow)?;
    let logical_name = str::from_utf8(
        input
            .get(RECORD_HEADER_LEN..logical_name_end)
            .ok_or(RecordError::Truncated("logical name"))?,
    )
    .map_err(|_| RecordError::LogicalNameUtf8)?;
    let payload = input
        .get(logical_name_end..record_end)
        .ok_or(RecordError::Truncated("payload"))?;
    if package_asset_id(kind, payload) != id {
        return Err(RecordError::DigestMismatch);
    }
    Ok((
        RecordRef {
            kind,
            id,
            logical_name,
            payload,
        },
        record_end,
    ))
}
